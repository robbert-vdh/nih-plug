use atomic_refcell::AtomicRefCell;
use baseview::{EventStatus, Window, WindowHandler, WindowOpenOptions};
use crossbeam::channel::{self, Sender};
use crossbeam::queue::ArrayQueue;
use parking_lot::Mutex;
use raw_window_handle::HasRawWindowHandle;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use super::backend::Backend;
use super::config::WrapperConfig;
use super::context::{WrapperGuiContext, WrapperInitContext, WrapperProcessContext};
use crate::event_loop::{EventLoop, MainThreadExecutor, OsEventLoop};
use crate::prelude::{
    AsyncExecutor, AudioIOLayout, BufferConfig, Editor, ParamFlags, ParamPtr, Params,
    ParentWindowHandle, Plugin, PluginNoteEvent, ProcessMode, ProcessStatus, TaskExecutor,
    Transport,
};
use crate::util::permit_alloc;
use crate::wrapper::state::{self, PluginState};
use crate::wrapper::util::process_wrapper;

/// How many parameter changes we can store in our unprocessed parameter change queue. Storing more
/// than this many parameters at a time will cause changes to get lost.
const EVENT_QUEUE_CAPACITY: usize = 2048;

pub struct Wrapper<P: Plugin, B: Backend<P>> {
    backend: AtomicRefCell<B>,

    /// The wrapped plugin instance.
    plugin: Mutex<P>,
    /// The plugin's background task executor closure. Tasks scheduled by the plugin will be
    /// executed on the GUI or background thread using this function.
    pub task_executor: Mutex<TaskExecutor<P>>,
    /// The plugin's parameters. These are fetched once during initialization. That way the
    /// `ParamPtr`s are guaranteed to live at least as long as this object and we can interact with
    /// the `Params` object without having to acquire a lock on `plugin`.
    params: Arc<dyn Params>,
    /// The plugin's editor, if it has one. This object does not do anything on its own, but we need
    /// to instantiate this in advance so we don't need to lock the entire [`Plugin`] object when
    /// creating an editor. Wrapped in an `AtomicRefCell` because it needs to be initialized late.
    pub editor: AtomicRefCell<Option<Arc<Mutex<Box<dyn Editor>>>>>,
    /// A channel for sending tasks to the GUI window, if the plugin has a GUI. Set in `run()`.
    gui_tasks_sender: AtomicRefCell<Option<Sender<GuiTask>>>,

    /// A realtime-safe task queue so the plugin can schedule tasks that need to be run later on the
    /// GUI thread. See the same field in the VST3 wrapper for more information on why this looks
    /// the way it does.
    event_loop: AtomicRefCell<Option<OsEventLoop<Task<P>, Self>>>,

    /// This is used to grab the DPI scaling config. Not used on macOS.
    #[allow(unused)]
    config: WrapperConfig,

    /// A mapping from parameter pointers to string parameter IDs. This is used as part of
    /// `Task::ParamValueChanged` to send a parameter change event to the editor from the GUI
    /// thread. This is also used to check whether the `ParamPtr` for an incoming parameter change
    /// actually belongs to a registered parameter.
    param_ptr_to_id: HashMap<ParamPtr, String>,
    /// A mapping from parameter string IDs to parameter pointers. Used for serialization and
    /// deserialization.
    param_id_to_ptr: HashMap<String, ParamPtr>,

    /// The bus and buffer configurations are static for the standalone target.
    audio_io_layout: AudioIOLayout,
    buffer_config: BufferConfig,

    /// Parameter changes that have been output by the GUI that have not yet been set in the plugin.
    /// This queue will be flushed at the end of every processing cycle, just like in the plugin
    /// versions.
    unprocessed_param_changes: ArrayQueue<(ParamPtr, f32)>,
    /// The plugin is able to restore state through a method on the `GuiContext`. To avoid changing
    /// parameters mid-processing and running into garbled data if the host also tries to load state
    /// at the same time the restoring happens at the end of each processing call. If this zero
    /// capacity channel contains state data at that point, then the audio thread will take the
    /// state out of the channel, restore the state, and then send it back through the same channel.
    /// In other words, the GUI thread acts as a sender and then as a receiver, while the audio
    /// thread acts as a receiver and then as a sender. That way deallocation can happen on the GUI
    /// thread. All of this happens without any blocking on the audio thread.
    updated_state_sender: channel::Sender<PluginState>,
    /// The receiver belonging to [`new_state_sender`][Self::new_state_sender].
    updated_state_receiver: channel::Receiver<PluginState>,
    /// The current latency in samples, as set by the plugin through the [`InitContext`] and the
    /// [`ProcessContext`]. This value may not be used depending on the audio backend, but it's
    /// still kept track of to avoid firing debug assertions multiple times for the same latency
    /// value.
    current_latency: AtomicU32,
}

/// Tasks that can be sent from the plugin to be executed on the main thread in a non-blocking
/// realtime-safe way (either a random thread or `IRunLoop` on Linux, the OS' message loop on
/// Windows and macOS).
#[allow(clippy::enum_variant_names)]
pub enum Task<P: Plugin> {
    /// Execute one of the plugin's background tasks.
    PluginTask(P::BackgroundTask),
    /// Inform the plugin that one or more parameter values have changed.
    ParameterValuesChanged,
    /// Inform the plugin that one parameter's value has changed. This uses the parameter hashes
    /// since the task will be created from the audio thread. We don't have parameter hashes here
    /// like in the plugin APIs, so we'll just use the `ParamPtr`s directly. These are used to index
    /// the hashmaps stored on `Wrapper`.
    ParameterValueChanged(ParamPtr, f32),
}

/// Errors that may arise while initializing the wrapped plugins.
#[derive(Debug, Clone, Copy)]
pub enum WrapperError {
    /// The plugin returned `false` during initialization.
    InitializationFailed,
}

struct WrapperWindowHandler {
    /// The editor handle for the plugin's open editor. The editor should clean itself up when it
    /// gets dropped.
    _editor_handle: Box<dyn Any>,

    /// This is used to communicate with the wrapper from the audio thread and from within the
    /// baseview window handler on the GUI thread.
    gui_task_receiver: channel::Receiver<GuiTask>,
}

/// A message sent to the GUI thread.
pub enum GuiTask {
    /// Resize the window to the following physical size.
    Resize(u32, u32),
    /// The close window. This will cause the application to terminate.
    Close,
}

impl WindowHandler for WrapperWindowHandler {
    fn on_frame(&mut self, window: &mut Window) {
        while let Ok(task) = self.gui_task_receiver.try_recv() {
            match task {
                GuiTask::Resize(new_width, new_height) => {
                    window.resize(baseview::Size {
                        width: new_width as f64,
                        height: new_height as f64,
                    });
                }
                GuiTask::Close => window.close(),
            }
        }
    }

    fn on_event(&mut self, _window: &mut Window, _event: baseview::Event) -> EventStatus {
        EventStatus::Ignored
    }
}

impl<P: Plugin, B: Backend<P>> MainThreadExecutor<Task<P>> for Wrapper<P, B> {
    fn execute(&self, task: Task<P>, _is_gui_thread: bool) {
        match task {
            Task::PluginTask(task) => (self.task_executor.lock())(task),
            Task::ParameterValuesChanged => {
                if let Some(editor) = self.editor.borrow().as_ref() {
                    editor.lock().param_values_changed();
                }
            }
            Task::ParameterValueChanged(param_ptr, normalized_value) => {
                if let Some(editor) = self.editor.borrow().as_ref() {
                    let param_id = &self.param_ptr_to_id[&param_ptr];
                    editor
                        .lock()
                        .param_value_changed(param_id, normalized_value);
                }
            }
        }
    }
}

impl<P: Plugin, B: Backend<P>> Wrapper<P, B> {
    /// Instantiate a new instance of the standalone wrapper. Returns an error if the plugin does
    /// not accept the IO configuration from the wrapper config.
    pub fn new(backend: B, config: WrapperConfig) -> Result<Arc<Self>, WrapperError> {
        // The backend has already queried this, so this will never cause the program to exit
        // TODO: Do the validation and parsing in the argument parser so this value can be stored on
        //       the config itself. Right now clap doesn't support this.
        let audio_io_layout = config.audio_io_layout_or_exit::<P>();

        let mut plugin = P::default();
        let task_executor = Mutex::new(plugin.task_executor());
        let params = plugin.params();

        // This is used to allow the plugin to restore preset data from its editor, see the comment
        // on `Self::updated_state_sender`
        let (updated_state_sender, updated_state_receiver) = channel::bounded(0);

        // For consistency's sake we'll include the same assertions as the other backends
        // TODO: Move these common checks to a function instead of repeating them in every wrapper
        let param_map = params.param_map();
        if cfg!(debug_assertions) {
            let param_ids: HashSet<_> = param_map.iter().map(|(id, _, _)| id.clone()).collect();
            nih_debug_assert_eq!(
                param_map.len(),
                param_ids.len(),
                "The plugin has duplicate parameter IDs, weird things may happen. Consider using \
                 6 character parameter IDs to avoid collisions."
            );

            let mut bypass_param_exists = false;
            for (_, ptr, _) in &param_map {
                let flags = unsafe { ptr.flags() };
                let is_bypass = flags.contains(ParamFlags::BYPASS);

                if is_bypass && bypass_param_exists {
                    nih_debug_assert_failure!(
                        "Duplicate bypass parameters found, the host will only use the first one"
                    );
                }

                bypass_param_exists |= is_bypass;
            }
        }

        let wrapper = Arc::new(Wrapper {
            backend: AtomicRefCell::new(backend),

            plugin: Mutex::new(plugin),
            task_executor,
            params,
            // Initialized later as it needs a reference to the wrapper for the async executor
            editor: AtomicRefCell::new(None),
            // Set in `run()`
            gui_tasks_sender: AtomicRefCell::new(None),

            // Also initialized later as it also needs a reference to the wrapper
            event_loop: AtomicRefCell::new(None),

            param_ptr_to_id: param_map
                .iter()
                .map(|(param_id, param_ptr, _)| (*param_ptr, param_id.clone()))
                .collect(),
            param_id_to_ptr: param_map
                .into_iter()
                .map(|(param_id, param_ptr, _)| (param_id, param_ptr))
                .collect(),

            audio_io_layout,
            buffer_config: BufferConfig {
                sample_rate: config.sample_rate,
                min_buffer_size: None,
                max_buffer_size: config.period_size,
                // TODO: Detect JACK freewheeling and report it here
                process_mode: ProcessMode::Realtime,
            },
            config,

            unprocessed_param_changes: ArrayQueue::new(EVENT_QUEUE_CAPACITY),
            updated_state_sender,
            updated_state_receiver,
            current_latency: AtomicU32::new(0),
        });

        *wrapper.event_loop.borrow_mut() =
            Some(OsEventLoop::new_and_spawn(Arc::downgrade(&wrapper)));

        // The editor needs to be initialized later so the Async executor can work.
        *wrapper.editor.borrow_mut() = wrapper
            .plugin
            .lock()
            .editor(AsyncExecutor {
                execute_background: Arc::new({
                    let wrapper = wrapper.clone();

                    move |task| {
                        let task_posted = wrapper.schedule_background(Task::PluginTask(task));
                        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
                    }
                }),
                execute_gui: Arc::new({
                    let wrapper = wrapper.clone();

                    move |task| {
                        let task_posted = wrapper.schedule_gui(Task::PluginTask(task));
                        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
                    }
                }),
            })
            .map(|editor| Arc::new(Mutex::new(editor)));

        // Before initializing the plugin, make sure all smoothers are set the the default values
        for param in wrapper.param_id_to_ptr.values() {
            unsafe { param.update_smoother(wrapper.buffer_config.sample_rate, true) };
        }

        {
            let mut plugin = wrapper.plugin.lock();
            if !plugin.initialize(
                &wrapper.audio_io_layout,
                &wrapper.buffer_config,
                &mut wrapper.make_init_context(),
            ) {
                return Err(WrapperError::InitializationFailed);
            }
            process_wrapper(|| plugin.reset());
        }

        Ok(wrapper)
    }

    /// Open the editor, start processing audio, and block this thread until the editor is closed.
    /// If the plugin does not have an editor, then this will block until SIGINT is received.
    ///
    /// Will return an error if the plugin threw an error during audio processing or if the editor
    /// could not be opened.
    pub fn run(self: Arc<Self>) -> Result<(), WrapperError> {
        let (gui_task_sender, gui_task_receiver) = channel::bounded(512);
        *self.gui_tasks_sender.borrow_mut() = Some(gui_task_sender.clone());

        // We'll spawn a separate thread to handle IO and to process audio. This audio thread should
        // terminate together with this function.
        let terminate_audio_thread = Arc::new(AtomicBool::new(false));
        let audio_thread = {
            let this = self.clone();
            let terminate_audio_thread = terminate_audio_thread.clone();
            thread::spawn(move || this.run_audio_thread(terminate_audio_thread, gui_task_sender))
        };

        match self.editor.borrow().clone() {
            Some(editor) => {
                let context = self.clone().make_gui_context();

                // DPI scaling should not be used on macOS since the OS handles it there
                #[cfg(target_os = "macos")]
                let scaling_policy = baseview::WindowScalePolicy::SystemScaleFactor;
                #[cfg(not(target_os = "macos"))]
                let scaling_policy = {
                    editor.lock().set_scale_factor(self.config.dpi_scale);
                    baseview::WindowScalePolicy::ScaleFactor(self.config.dpi_scale as f64)
                };

                let (width, height) = editor.lock().size();
                Window::open_blocking(
                    WindowOpenOptions {
                        title: String::from(P::NAME),
                        size: baseview::Size {
                            width: width as f64,
                            height: height as f64,
                        },
                        scale: scaling_policy,
                        gl_config: None,
                    },
                    move |window| {
                        let parent_handle = match window.raw_window_handle() {
                            raw_window_handle::RawWindowHandle::Xlib(handle) => {
                                ParentWindowHandle::X11Window(handle.window as u32)
                            }
                            raw_window_handle::RawWindowHandle::Xcb(handle) => {
                                ParentWindowHandle::X11Window(handle.window)
                            }
                            raw_window_handle::RawWindowHandle::AppKit(handle) => {
                                ParentWindowHandle::AppKitNsView(handle.ns_view)
                            }
                            raw_window_handle::RawWindowHandle::Win32(handle) => {
                                ParentWindowHandle::Win32Hwnd(handle.hwnd)
                            }
                            handle => unimplemented!("Unsupported window handle: {handle:?}"),
                        };

                        // TODO: This spawn function should be able to fail and return an error, but
                        //       baseview does not support this yet. Once this is added, we should
                        //       immediately close the parent window when this happens so the loop
                        //       can exit.
                        let editor_handle = editor.lock().spawn(parent_handle, context);

                        WrapperWindowHandler {
                            _editor_handle: editor_handle,
                            gui_task_receiver,
                        }
                    },
                )
            }
            None => {
                // TODO: Properly block until SIGINT is received if the plugin does not have an editor
                // TODO: Make sure to handle `GuiTask::Close` here as well
                nih_log!("{} does not have a GUI, blocking indefinitely...", P::NAME);
                std::thread::park();
            }
        }

        terminate_audio_thread.store(true, Ordering::SeqCst);
        audio_thread.join().unwrap();

        // Some plugins may use this to clean up resources. Should not be needed for the standalone
        // application, but it seems like a good idea to stay consistent.
        self.plugin.lock().deactivate();

        Ok(())
    }

    /// Get a parameter's ID based on a `ParamPtr`. Used in the `GuiContext` implementation for the
    /// gesture checks.
    #[allow(unused)]
    pub fn param_id_from_ptr(&self, param: ParamPtr) -> Option<&str> {
        self.param_ptr_to_id.get(&param).map(|s| s.as_str())
    }

    /// Set a parameter based on a `ParamPtr`. The value will be updated at the end of the next
    /// processing cycle, and this won't do anything if the parameter has not been registered by the
    /// plugin.
    ///
    /// This returns false if the parameter was not set because the `ParamPtr` was either unknown or
    /// the queue is full.
    pub fn set_parameter(&self, param: ParamPtr, normalized: f32) -> bool {
        if !self.param_ptr_to_id.contains_key(&param) {
            return false;
        }

        let push_successful = self
            .unprocessed_param_changes
            .push((param, normalized))
            .is_ok();
        nih_debug_assert!(push_successful, "The parameter change queue was full");

        push_successful
    }

    /// Get the plugin's state object, may be called by the plugin's GUI as part of its own preset
    /// management. The wrapper doesn't use these functions and serializes and deserializes directly
    /// the JSON in the relevant plugin API methods instead.
    pub fn get_state_object(&self) -> PluginState {
        unsafe {
            state::serialize_object::<P>(
                self.params.clone(),
                self.param_id_to_ptr
                    .iter()
                    .map(|(param_id, param_ptr)| (param_id, *param_ptr)),
            )
        }
    }

    /// Update the plugin's internal state, called by the plugin itself from the GUI thread. To
    /// prevent corrupting data and changing parameters during processing the actual state is only
    /// updated at the end of the audio processing cycle.
    pub fn set_state_object_from_gui(&self, state: PluginState) {
        match self.updated_state_sender.send(state) {
            Ok(_) => {
                // As mentioned above, the state object will be passed back to this thread
                // so we can deallocate it without blocking.
                let state = self.updated_state_receiver.recv();
                drop(state);
            }
            Err(err) => {
                nih_debug_assert_failure!(
                    "Could not send new state to the audio thread: {:?}",
                    err
                );
            }
        }
    }

    /// Posts the task to the background task queue using [`EventLoop::schedule_background()`] so it
    /// can be run in the background without blocking either the GUI or the audio thread.
    ///
    /// If the task queue is full, then this will return false.
    #[must_use]
    pub fn schedule_background(&self, task: Task<P>) -> bool {
        let event_loop = self.event_loop.borrow();
        let event_loop = event_loop.as_ref().unwrap();
        event_loop.schedule_background(task)
    }

    /// Posts the task to the task queue using [`EventLoop::schedule_gui()`] so it can be delegated
    /// to the main thread. The task is run directly if this is the GUI thread.
    ///
    /// If the task queue is full, then this will return false.
    #[must_use]
    pub fn schedule_gui(&self, task: Task<P>) -> bool {
        let event_loop = self.event_loop.borrow();
        let event_loop = event_loop.as_ref().unwrap();
        event_loop.schedule_gui(task)
    }

    /// Request the outer window to be resized to the editor's current size.
    pub fn request_resize(&self) {
        if let Some(gui_tasks_sender) = self.gui_tasks_sender.borrow().as_ref() {
            let (unscaled_width, unscaled_height) =
                self.editor.borrow().as_ref().unwrap().lock().size();

            // This will cause the editor to be resized at the start of the next frame
            let push_successful = gui_tasks_sender
                .send(GuiTask::Resize(unscaled_width, unscaled_height))
                .is_ok();
            nih_debug_assert!(push_successful, "Could not queue window resize");
        }
    }

    pub fn set_latency_samples(&self, samples: u32) {
        // This should only change the value if it's actually needed
        let old_latency = self.current_latency.swap(samples, Ordering::SeqCst);
        if old_latency != samples {
            // None of the backends actually support this at the moment
            nih_debug_assert_failure!("Standalones currently don't support latency reporting");
        }
    }

    /// The audio thread. This should be called from another thread, and it will run until
    /// `should_terminate` is `true`.
    fn run_audio_thread(
        self: Arc<Self>,
        should_terminate: Arc<AtomicBool>,
        gui_task_sender: channel::Sender<GuiTask>,
    ) {
        self.clone().backend.borrow_mut().run(
            move |buffer, aux, transport, input_events, output_events| {
                // TODO: This process wrapper should actually be in the backends (since the backends
                //       should also not allocate in their audio callbacks), but that's a bit more
                //       error prone
                process_wrapper(|| {
                    if should_terminate.load(Ordering::SeqCst) {
                        return false;
                    }

                    let sample_rate = self.buffer_config.sample_rate;
                    {
                        let mut plugin = self.plugin.lock();
                        if let ProcessStatus::Error(err) = plugin.process(
                            buffer,
                            aux,
                            &mut self.make_process_context(transport, input_events, output_events),
                        ) {
                            nih_error!("The plugin returned an error while processing:");
                            nih_error!("{}", err);

                            let push_successful = gui_task_sender.send(GuiTask::Close).is_ok();
                            nih_debug_assert!(
                                push_successful,
                                "Could not queue window close, the editor will remain open"
                            );

                            return false;
                        }
                    }

                    // Any output note events are now in a vector that can be processed by the
                    // audio/MIDI backend

                    // We'll always write these events to the first sample, so even when we add note
                    // output we shouldn't have to think about interleaving events here
                    while let Some((param_ptr, normalized_value)) =
                        self.unprocessed_param_changes.pop()
                    {
                        if unsafe { param_ptr.set_normalized_value(normalized_value) } {
                            unsafe { param_ptr.update_smoother(sample_rate, false) };
                            let task_posted = self.schedule_gui(Task::ParameterValueChanged(
                                param_ptr,
                                normalized_value,
                            ));
                            nih_debug_assert!(
                                task_posted,
                                "The task queue is full, dropping task..."
                            );
                        }
                    }

                    // After processing audio, we'll check if the editor has sent us updated plugin
                    // state.  We'll restore that here on the audio thread to prevent changing the
                    // values during the process call and also to prevent inconsistent state when
                    // the host also wants to load plugin state.
                    // FIXME: Zero capacity channels allocate on receiving, find a better
                    //        alternative that doesn't do that
                    let updated_state = permit_alloc(|| self.updated_state_receiver.try_recv());
                    if let Ok(mut state) = updated_state {
                        self.set_state_inner(&mut state);

                        // We'll pass the state object back to the GUI thread so deallocation can
                        // happen there without potentially blocking the audio thread
                        if let Err(err) = self.updated_state_sender.send(state) {
                            nih_debug_assert_failure!(
                                "Failed to send state object back to GUI thread: {}",
                                err
                            );
                        };
                    }

                    true
                })
            },
        );
    }

    fn make_gui_context(self: Arc<Self>) -> Arc<WrapperGuiContext<P, B>> {
        Arc::new(WrapperGuiContext {
            wrapper: self,
            #[cfg(debug_assertions)]
            param_gesture_checker: Default::default(),
        })
    }

    fn make_init_context(&self) -> WrapperInitContext<'_, P, B> {
        WrapperInitContext { wrapper: self }
    }

    fn make_process_context<'a>(
        &'a self,
        transport: Transport,
        input_events: &'a [PluginNoteEvent<P>],
        output_events: &'a mut Vec<PluginNoteEvent<P>>,
    ) -> WrapperProcessContext<'a, P, B> {
        WrapperProcessContext {
            wrapper: self,
            input_events,
            input_events_idx: 0,
            output_events,
            transport,
        }
    }

    /// Immediately set the plugin state. Returns `false` if the deserialization failed. In other
    /// wrappers state is set from a couple places, so this function is here to be consistent and to
    /// centralize all of this behavior. Includes `permit_alloc()`s around the deserialization and
    /// initialization for the use case where `set_state_object_from_gui()` was called while the
    /// plugin is process audio.
    ///
    /// Implicitly emits `Task::ParameterValuesChanged`.
    ///
    /// # Notes
    ///
    /// `self.plugin` must _not_ be locked while calling this function or it will deadlock.
    fn set_state_inner(&self, state: &mut PluginState) -> bool {
        // FIXME: This is obviously not realtime-safe, but loading presets without doing this could
        //        lead to inconsistencies. It's the plugin's responsibility to not perform any
        //        realtime-unsafe work when the initialize function is called a second time if it
        //        supports runtime preset loading. `state::deserialize_object()` normally never
        //        allocates, but if the plugin has persistent non-parameter data then its
        //        `deserialize_fields()` implementation may still allocate.
        let mut success = permit_alloc(|| unsafe {
            state::deserialize_object::<P>(
                state,
                self.params.clone(),
                |param_id| self.param_id_to_ptr.get(param_id).copied(),
                Some(&self.buffer_config),
            )
        });
        if !success {
            nih_debug_assert_failure!("Deserializing plugin state from a state object failed");
            return false;
        }

        // If the plugin was already initialized then it needs to be reinitialized
        {
            // NOTE: This needs to be dropped after the `plugin` lock to avoid deadlocks
            let mut init_context = self.make_init_context();
            let mut plugin = self.plugin.lock();

            // See above
            success = permit_alloc(|| {
                plugin.initialize(
                    &self.audio_io_layout,
                    &self.buffer_config,
                    &mut init_context,
                )
            });
            if success {
                process_wrapper(|| plugin.reset());
            }
        }

        nih_debug_assert!(
            success,
            "Plugin returned false when reinitializing after loading state"
        );

        // Reinitialize the plugin after loading state so it can respond to the new parameter values
        let task_posted = self.schedule_gui(Task::ParameterValuesChanged);
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");

        // TODO: Right now there's no way to know if loading the state changed the GUI's size. We
        //       could keep track of the last known size and compare the GUI's current size against
        //       that but that also seems brittle.
        self.request_resize();

        success
    }
}
