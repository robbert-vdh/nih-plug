use atomic_refcell::AtomicRefCell;
use baseview::{EventStatus, Window, WindowHandler, WindowOpenOptions};
use crossbeam::channel;
use crossbeam::queue::ArrayQueue;
use parking_lot::RwLock;
use raw_window_handle::HasRawWindowHandle;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use super::backend::Backend;
use super::context::{WrapperGuiContext, WrapperProcessContext};
use crate::context::Transport;
use crate::param::internals::{ParamPtr, Params};
use crate::param::ParamFlags;
use crate::plugin::{
    AuxiliaryIOConfig, BufferConfig, BusConfig, Editor, ParentWindowHandle, Plugin, ProcessMode,
    ProcessStatus,
};
use crate::util::permit_alloc;
use crate::wrapper::state::{self, PluginState};

/// How many parameter changes we can store in our unprocessed parameter change queue. Storing more
/// than this many parameters at a time will cause changes to get lost.
const EVENT_QUEUE_CAPACITY: usize = 2048;

/// Configuration for a standalone plugin that would normally be provided by the DAW.
#[derive(Debug, Clone)]
pub struct WrapperConfig {
    /// The number of input channels.
    pub input_channels: u32,
    /// The number of output channels.
    pub output_channels: u32,
    /// The audio backend's sample rate.
    pub sample_rate: f32,
    /// The audio backend's period size.
    pub period_size: u32,

    /// The editor's DPI scaling factor. Currently baseview has no way to report this to us, so
    /// we'll expose it as a command line option instead.
    ///
    /// This option is ignored on macOS.
    pub dpi_scale: f32,

    /// The current tempo.
    pub tempo: f32,
    /// The time signature's numerator.
    pub timesig_num: u32,
    /// The time signature's denominator.
    pub timesig_denom: u32,
}

pub struct Wrapper<P: Plugin, B: Backend> {
    backend: AtomicRefCell<B>,

    /// The wrapped plugin instance.
    plugin: RwLock<P>,
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
    /// creating an editor.
    pub editor: Option<Arc<dyn Editor>>,

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
    IncompatibleConfig,
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
                    // Window resizing in baseview has only been implemented on Linux
                    #[cfg(target_os = "linux")]
                    {
                        window.resize(baseview::Size {
                            width: new_width as f64,
                            height: new_height as f64,
                        });
                    }
                }
                GuiTask::Close => window.close(),
            }
        }
    }

    fn on_event(&mut self, _window: &mut Window, _event: baseview::Event) -> EventStatus {
        EventStatus::Ignored
    }
}

impl<P: Plugin, B: Backend> Wrapper<P, B> {
    /// Instantiate a new instance of the standalone wrapper. Returns an error if the plugin does
    /// not accept the IO configuration from the wrapper config.
    pub fn new(backend: B, config: WrapperConfig) -> Result<Arc<Self>, WrapperError> {
        let plugin = P::default();
        let params = plugin.params();
        let editor = plugin.editor().map(Arc::from);

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
                "The plugin has duplicate parameter IDs, weird things may happen. \
                 Consider using 6 character parameter IDs to avoid collissions.."
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

            plugin: RwLock::new(plugin),
            params,
            known_parameters: param_map.iter().map(|(_, ptr, _)| *ptr).collect(),
            param_map: param_map
                .into_iter()
                .map(|(param_id, param_ptr, _)| (param_id, param_ptr))
                .collect(),
            editor,

            bus_config: BusConfig {
                num_input_channels: config.input_channels,
                num_output_channels: config.output_channels,
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

        // Right now the IO configuration is fixed in the standalone target, so if the plugin cannot
        // work with this then we cannot initialize the plugin at all.
        {
            let mut plugin = wrapper.plugin.write();
            if !plugin.accepts_bus_config(&wrapper.bus_config) {
                return Err(WrapperError::IncompatibleConfig);
            }

            if !plugin.initialize(
                &wrapper.bus_config,
                &wrapper.buffer_config,
                &mut wrapper.make_process_context(Transport::new(wrapper.config.sample_rate)),
            ) {
                return Err(WrapperError::InitializationFailed);
            }
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

        match self.editor.clone() {
            Some(editor) => {
                let context = self.clone().make_gui_context(gui_task_sender);

                // DPI scaling should not be used on macOS since the OS handles it there
                #[cfg(target_os = "macos")]
                let scaling_policy = baseview::WindowScalePolicy::SystemScaleFactor;
                #[cfg(not(target_os = "macos"))]
                let scaling_policy = {
                    editor.set_scale_factor(self.config.dpi_scale);
                    baseview::WindowScalePolicy::ScaleFactor(self.config.dpi_scale as f64)
                };

                let (width, height) = editor.size();
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
                        let editor_handle = editor.spawn(
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
                // TODO: Block until SIGINT is received if the plugin does not have an editor
                // TODO: Make sure to handle `GuiTask::Close` here as well
                todo!("Support standalone plugins without editors");
            }
        }

        terminate_audio_thread.store(true, Ordering::SeqCst);
        audio_thread.join().unwrap();

        Ok(())
    }

    /// Set a parameter based on a `ParamPtr`. The value will be updated at the end of the next
    /// processing cycle, and this won't do anything if the parameter has not been registered by the
    /// plugin.
    ///
    /// This returns false if the parmeter was not set because the `Paramptr` was either unknown or
    /// the queue is full.
    pub fn set_parameter(&self, param: ParamPtr, normalized: f32) -> bool {
        if !self.known_parameters.contains(&param) {
            return false;
        }

        let push_succesful = self
            .unprocessed_param_changes
            .push((param, normalized))
            .is_ok();
        nih_debug_assert!(push_succesful, "The parameter change queue was full");

        push_succesful
    }

    /// The DPI scale factor for this standalone application
    pub fn dpi_scale(&self) -> f32 {
        // DPI scaling should be ignored on macOS since the OS already handles this
        #[cfg(target_os = "macos")]
        return 1.0;
        #[cfg(not(target_os = "macos"))]
        return self.config.dpi_scale;
    }

    /// Get the plugin's state object, may be called by the plugin's GUI as part of its own preset
    /// management. The wrapper doesn't use these functions and serializes and deserializes directly
    /// the JSON in the relevant plugin API methods instead.
    pub fn get_state_object(&self) -> PluginState {
        unsafe {
            state::serialize_object(
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
        // TODO: We should add a way to pull the transport information from the JACK backend
        let mut num_processed_samples = 0;

        self.clone().backend.borrow_mut().run(move |buffer| {
            if should_terminate.load(Ordering::SeqCst) {
                return false;
            }

            // TODO: Process incoming events

            let sample_rate = self.buffer_config.sample_rate;
            let mut transport = Transport::new(sample_rate);
            transport.pos_samples = Some(num_processed_samples);
            transport.tempo = Some(self.config.tempo as f64);
            transport.time_sig_numerator = Some(self.config.timesig_num as i32);
            transport.time_sig_denominator = Some(self.config.timesig_denom as i32);
            transport.playing = true;

            if let ProcessStatus::Error(err) = self
                .plugin
                .write()
                .process(buffer, &mut self.make_process_context(transport))
            {
                eprintln!("The plugin returned an error while processing:");
                eprintln!("{}", err);

                let push_successful = gui_task_sender.send(GuiTask::Close).is_ok();
                nih_debug_assert!(
                    push_successful,
                    "Could not queue window close, the editor will remain open"
                );

                return false;
            }

            // We'll always write these events to the first sample, so even when we add note output we
            // shouldn't have to think about interleaving events here
            let mut parameter_values_changed = false;
            while let Some((param_ptr, normalized_value)) = self.unprocessed_param_changes.pop() {
                unsafe { param_ptr.set_normalized_value(normalized_value) };
                unsafe { param_ptr.update_smoother(sample_rate, false) };
                parameter_values_changed = true;
            }

            // Allow the editor to react to the new parameter values if the editor uses a reactive data
            // binding model
            if parameter_values_changed {
                self.notify_param_values_changed();
            }

            // TODO: MIDI output

            // After processing audio, we'll check if the editor has sent us updated plugin state.
            // We'll restore that here on the audio thread to prevent changing the values during the
            // process call and also to prevent inconsistent state when the host also wants to load
            // plugin state.
            // FIXME: Zero capacity channels allocate on receiving, find a better alternative that
            //        doesn't do that
            let updated_state = permit_alloc(|| self.updated_state_receiver.try_recv());
            if let Ok(state) = updated_state {
                unsafe {
                    state::deserialize_object(
                        &state,
                        self.params.clone(),
                        |param_id| self.param_map.get(param_id).copied(),
                        Some(&self.buffer_config),
                    );
                }

                self.notify_param_values_changed();

                // TODO: Normally we'd also call initialize after deserializing state, but that's
                //       not guaranteed to be realtime safe. Should we do it anyways?
                self.plugin.write().reset();

                // We'll pass the state object back to the GUI thread so deallocation can happen
                // there without potentially blocking the audio thread
                if let Err(err) = self.updated_state_sender.send(state) {
                    nih_debug_assert_failure!(
                        "Failed to send state object back to GUI thread: {}",
                        err
                    );
                };
            }

            num_processed_samples += buffer.len() as i64;

            true
        });
    }

    /// Tell the editor that the parameter values have changed, if the plugin has an editor.
    fn notify_param_values_changed(&self) {
        if let Some(editor) = &self.editor {
            editor.param_values_changed();
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

    fn make_process_context(&self, transport: Transport) -> WrapperProcessContext<'_, P, B> {
        WrapperProcessContext {
            wrapper: self,
            transport,
        }
    }
}
