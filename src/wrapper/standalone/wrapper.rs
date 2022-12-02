use atomic_refcell::AtomicRefCell;
use baseview::{EventStatus, Window, WindowHandler, WindowOpenOptions};
use crossbeam::channel;
use crossbeam::queue::ArrayQueue;
use parking_lot::Mutex;
use raw_window_handle::HasRawWindowHandle;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use super::backend::Backend;
use super::config::WrapperConfig;
use super::context::{WrapperGuiContext, WrapperInitContext, WrapperProcessContext};
use crate::context::gui::AsyncExecutor;
use crate::context::process::Transport;
use crate::editor::{Editor, ParentWindowHandle};
use crate::event_loop::{EventLoop, MainThreadExecutor, OsEventLoop};
use crate::midi::NoteEvent;
use crate::params::internals::ParamPtr;
use crate::params::{ParamFlags, Params};
use crate::plugin::{
    AuxiliaryBuffers, AuxiliaryIOConfig, BufferConfig, BusConfig, Plugin, ProcessMode,
    ProcessStatus, TaskExecutor,
};
use crate::util::permit_alloc;
use crate::wrapper::state::{self, PluginState};
use crate::wrapper::util::process_wrapper;

/// How many parameter changes we can store in our unprocessed parameter change queue. Storing more
/// than this many parameters at a time will cause changes to get lost.
const EVENT_QUEUE_CAPACITY: usize = 2048;

pub struct Wrapper<P: Plugin, B: Backend> {
    backend: AtomicRefCell<B>,

    /// The wrapped plugin instance.
    plugin: Mutex<P>,
    /// The plugin's background task executor closure. Wrapped in another struct so it can be used
    /// as a [`MainContext`] with [`EventLoop`].
    pub task_executor_wrapper: Arc<TaskExecutorWrapper<P>>,
    /// The plugin's parameters. These are fetched once during initialization. That way the
    /// `ParamPtr`s are guaranteed to live at least as long as this object and we can interact with
    /// the `Params` object without having to acquire a lock on `plugin`.
    params: Arc<dyn Params>,
    /// The set of parameter pointers in `params`. This is technically not necessary, but for
    /// consistency with the plugin wrappers we'll check whether the `ParamPtr` for an incoming
    /// parameter change actually belongs to a registered parameter.
    known_parameters: HashSet<ParamPtr>,
    /// A mapping from parameter string IDs to parameter pointers.
    param_map: HashMap<String, ParamPtr>,
    /// The plugin's editor, if it has one. This object does not do anything on its own, but we need
    /// to instantiate this in advance so we don't need to lock the entire [`Plugin`] object when
    /// creating an editor. Wrapped in an `AtomicRefCell` because it needs to be initialized late.
    pub editor: AtomicRefCell<Option<Arc<Mutex<Box<dyn Editor>>>>>,

    /// A realtime-safe task queue so the plugin can schedule tasks that need to be run later on the
    /// GUI thread. See the same field in the VST3 wrapper for more information on why this looks
    /// the way it does.
    ///
    /// This is only used for executing [`AsyncExecutor`] tasks, so it's parameterized directly over
    /// that using a special `MainThreadExecutor` wrapper around `AsyncExecutor`.
    pub(crate) event_loop: OsEventLoop<P::BackgroundTask, TaskExecutorWrapper<P>>,

    /// This is used to grab the DPI scaling config. Not used on macOS.
    #[allow(unused)]
    config: WrapperConfig,

    /// The bus and buffer configurations are static for the standalone target.
    bus_config: BusConfig,
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
}

/// Errors that may arise while initializing the wrapped plugins.
#[derive(Debug, Clone, Copy)]
pub enum WrapperError {
    /// The plugin does not accept the IO configuration from the config.
    IncompatibleConfig {
        input_channels: u32,
        output_channels: u32,
    },
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

/// Adapter to make `TaskExecutor<P>` work as a `MainThreadExecutor`.
pub struct TaskExecutorWrapper<P: Plugin> {
    pub task_executor: Mutex<TaskExecutor<P>>,
}

impl<P: Plugin> MainThreadExecutor<P::BackgroundTask> for TaskExecutorWrapper<P> {
    fn execute(&self, task: P::BackgroundTask, _is_gui_thread: bool) {
        (self.task_executor.lock())(task)
    }
}

impl<P: Plugin, B: Backend> Wrapper<P, B> {
    /// Instantiate a new instance of the standalone wrapper. Returns an error if the plugin does
    /// not accept the IO configuration from the wrapper config.
    pub fn new(backend: B, config: WrapperConfig) -> Result<Arc<Self>, WrapperError> {
        let plugin = P::default();
        let task_executor_wrapper = Arc::new(TaskExecutorWrapper {
            task_executor: Mutex::new(plugin.task_executor()),
        });
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

        // TODO: Sidechain inputs and auxiliary outputs
        if P::DEFAULT_AUX_INPUTS.is_some() {
            nih_log!("Sidechain inputs are not yet supported in this standalone version");
        }
        if P::DEFAULT_AUX_OUTPUTS.is_some() {
            nih_log!("Auxiliary outputs are not yet supported in this standalone version");
        }

        let wrapper = Arc::new(Wrapper {
            backend: AtomicRefCell::new(backend),

            plugin: Mutex::new(plugin),
            task_executor_wrapper: task_executor_wrapper.clone(),
            params,
            known_parameters: param_map.iter().map(|(_, ptr, _)| *ptr).collect(),
            param_map: param_map
                .into_iter()
                .map(|(param_id, param_ptr, _)| (param_id, param_ptr))
                .collect(),
            // Initialized later as it needs a reference to the wrapper for the async executor
            editor: AtomicRefCell::new(None),

            event_loop: OsEventLoop::new_and_spawn(task_executor_wrapper),

            bus_config: BusConfig {
                num_input_channels: config.input_channels.unwrap_or(P::DEFAULT_INPUT_CHANNELS),
                num_output_channels: config.output_channels.unwrap_or(P::DEFAULT_OUTPUT_CHANNELS),
                // TODO: Expose additional sidechain IO in the JACK backend
                aux_input_busses: AuxiliaryIOConfig::default(),
                aux_output_busses: AuxiliaryIOConfig::default(),
            },
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
        });

        // The editor needs to be initialized later so the Async executor can work.
        *wrapper.editor.borrow_mut() = wrapper
            .plugin
            .lock()
            .editor(AsyncExecutor {
                execute_background: Arc::new({
                    let wrapper = wrapper.clone();

                    move |task| {
                        let task_posted = wrapper.event_loop.schedule_background(task);
                        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
                    }
                }),
                execute_gui: Arc::new({
                    let wrapper = wrapper.clone();

                    move |task| {
                        let task_posted = wrapper.event_loop.schedule_gui(task);
                        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
                    }
                }),
            })
            .map(|editor| Arc::new(Mutex::new(editor)));

        // Right now the IO configuration is fixed in the standalone target, so if the plugin cannot
        // work with this then we cannot initialize the plugin at all.
        {
            let mut plugin = wrapper.plugin.lock();
            if !plugin.accepts_bus_config(&wrapper.bus_config) {
                return Err(WrapperError::IncompatibleConfig {
                    input_channels: wrapper.bus_config.num_input_channels,
                    output_channels: wrapper.bus_config.num_output_channels,
                });
            }

            // Before initializing the plugin, make sure all smoothers are set the the default values
            for param in wrapper.known_parameters.iter() {
                unsafe { param.update_smoother(wrapper.buffer_config.sample_rate, true) };
            }

            if !plugin.initialize(
                &wrapper.bus_config,
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

        // We'll spawn a separate thread to handle IO and to process audio. This audio thread should
        // terminate together with this function.
        let terminate_audio_thread = Arc::new(AtomicBool::new(false));
        let audio_thread = {
            let this = self.clone();
            let terminate_audio_thread = terminate_audio_thread.clone();
            let gui_task_sender = gui_task_sender.clone();
            thread::spawn(move || this.run_audio_thread(terminate_audio_thread, gui_task_sender))
        };

        match self.editor.borrow().clone() {
            Some(editor) => {
                let context = self.clone().make_gui_context(gui_task_sender);

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
                        // TODO: This spawn function should be able to fail and return an error, but
                        //       baseview does not support this yet. Once this is added, we should
                        //       immediately close the parent window when this happens so the loop
                        //       can exit.
                        let editor_handle = editor.lock().spawn(
                            ParentWindowHandle {
                                handle: window.raw_window_handle(),
                            },
                            context,
                        );

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

    /// Set a parameter based on a `ParamPtr`. The value will be updated at the end of the next
    /// processing cycle, and this won't do anything if the parameter has not been registered by the
    /// plugin.
    ///
    /// This returns false if the parameter was not set because the `ParamPtr` was either unknown or
    /// the queue is full.
    pub fn set_parameter(&self, param: ParamPtr, normalized: f32) -> bool {
        if !self.known_parameters.contains(&param) {
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
                self.param_map
                    .iter()
                    .map(|(param_id, param_ptr)| (param_id, *param_ptr)),
            )
        }
    }

    /// Update the plugin's internal state, called by the plugin itself from the GUI thread. To
    /// prevent corrupting data and changing parameters during processing the actual state is only
    /// updated at the end of the audio processing cycle.
    pub fn set_state_object(&self, state: PluginState) {
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

    /// The audio thread. This should be called from another thread, and it will run until
    /// `should_terminate` is `true`.
    fn run_audio_thread(
        self: Arc<Self>,
        should_terminate: Arc<AtomicBool>,
        gui_task_sender: channel::Sender<GuiTask>,
    ) {
        self.clone().backend.borrow_mut().run(
            move |buffer, transport, input_events, output_events| {
                // TODO: This process wrapper should actually be in the backends (since the backends
                //       should also not allocate in their audio callbacks), but that's a bit more
                //       error prone
                process_wrapper(|| {
                    if should_terminate.load(Ordering::SeqCst) {
                        return false;
                    }

                    let sample_rate = self.buffer_config.sample_rate;
                    let mut plugin = self.plugin.lock();
                    if let ProcessStatus::Error(err) = plugin.process(
                        buffer,
                        // TODO: Provide extra inputs and outputs in the JACk backend
                        &mut AuxiliaryBuffers {
                            inputs: &mut [],
                            outputs: &mut [],
                        },
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

                    // Any output note events are now in a vector that can be processed by the
                    // audio/MIDI backend

                    // We'll always write these events to the first sample, so even when we add note
                    // output we shouldn't have to think about interleaving events here
                    let mut parameter_values_changed = false;
                    while let Some((param_ptr, normalized_value)) =
                        self.unprocessed_param_changes.pop()
                    {
                        unsafe { param_ptr.set_normalized_value(normalized_value) };
                        unsafe { param_ptr.update_smoother(sample_rate, false) };
                        parameter_values_changed = true;
                    }

                    // Allow the editor to react to the new parameter values if the editor uses a
                    // reactive data binding model
                    if parameter_values_changed {
                        self.notify_param_values_changed();
                    }

                    // After processing audio, we'll check if the editor has sent us updated plugin
                    // state.  We'll restore that here on the audio thread to prevent changing the
                    // values during the process call and also to prevent inconsistent state when
                    // the host also wants to load plugin state.
                    // FIXME: Zero capacity channels allocate on receiving, find a better
                    //        alternative that doesn't do that
                    let updated_state = permit_alloc(|| self.updated_state_receiver.try_recv());
                    if let Ok(mut state) = updated_state {
                        unsafe {
                            state::deserialize_object::<P>(
                                &mut state,
                                self.params.clone(),
                                |param_id| self.param_map.get(param_id).copied(),
                                Some(&self.buffer_config),
                            );
                        }

                        self.notify_param_values_changed();

                        // FIXME: This is obviously not realtime-safe, but loading presets without
                        //         doing this could lead to inconsistencies. It's the plugin's
                        //         responsibility to not perform any realtime-unsafe work when the
                        //         initialize function is called a second time if it supports
                        //         runtime preset loading.
                        permit_alloc(|| {
                            plugin.initialize(
                                &self.bus_config,
                                &self.buffer_config,
                                &mut self.make_init_context(),
                            )
                        });
                        plugin.reset();

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

    /// Tell the editor that the parameter values have changed, if the plugin has an editor. In the
    /// off-chance that the editor instance is currently locked then nothing will happen, and the
    /// request can safely be ignored.
    fn notify_param_values_changed(&self) {
        if let Some(editor) = self.editor.borrow().as_ref() {
            match editor.try_lock() {
                Some(editor) => editor.param_values_changed(),
                None => nih_debug_assert_failure!(
                    "The editor was locked when sending a parameter value change notification, \
                     ignoring"
                ),
            }
        }
    }

    fn make_gui_context(
        self: Arc<Self>,
        gui_task_sender: channel::Sender<GuiTask>,
    ) -> Arc<WrapperGuiContext<P, B>> {
        Arc::new(WrapperGuiContext {
            wrapper: self,
            gui_task_sender,
        })
    }

    fn make_init_context(&self) -> WrapperInitContext<'_, P, B> {
        WrapperInitContext { wrapper: self }
    }

    fn make_process_context<'a>(
        &'a self,
        transport: Transport,
        input_events: &'a [NoteEvent],
        output_events: &'a mut Vec<NoteEvent>,
    ) -> WrapperProcessContext<'a, P, B> {
        WrapperProcessContext {
            wrapper: self,
            input_events,
            input_events_idx: 0,
            output_events,
            transport,
        }
    }
}
