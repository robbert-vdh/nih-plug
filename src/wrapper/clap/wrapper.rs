use atomic_float::AtomicF32;
use atomic_refcell::{AtomicRefCell, AtomicRefMut};
use clap_sys::events::{
    clap_event_header, clap_event_midi, clap_event_midi_sysex, clap_event_note,
    clap_event_note_expression, clap_event_param_gesture, clap_event_param_mod,
    clap_event_param_value, clap_event_transport, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_IS_LIVE, CLAP_EVENT_MIDI, CLAP_EVENT_MIDI_SYSEX,
    CLAP_EVENT_NOTE_CHOKE, CLAP_EVENT_NOTE_END, CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF,
    CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_GESTURE_BEGIN, CLAP_EVENT_PARAM_GESTURE_END,
    CLAP_EVENT_PARAM_MOD, CLAP_EVENT_PARAM_VALUE, CLAP_EVENT_TRANSPORT,
    CLAP_NOTE_EXPRESSION_BRIGHTNESS, CLAP_NOTE_EXPRESSION_EXPRESSION, CLAP_NOTE_EXPRESSION_PAN,
    CLAP_NOTE_EXPRESSION_PRESSURE, CLAP_NOTE_EXPRESSION_TUNING, CLAP_NOTE_EXPRESSION_VIBRATO,
    CLAP_NOTE_EXPRESSION_VOLUME, CLAP_TRANSPORT_HAS_BEATS_TIMELINE,
    CLAP_TRANSPORT_HAS_SECONDS_TIMELINE, CLAP_TRANSPORT_HAS_TEMPO,
    CLAP_TRANSPORT_HAS_TIME_SIGNATURE, CLAP_TRANSPORT_IS_LOOP_ACTIVE, CLAP_TRANSPORT_IS_PLAYING,
    CLAP_TRANSPORT_IS_RECORDING, CLAP_TRANSPORT_IS_WITHIN_PRE_ROLL,
};
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS,
    CLAP_PORT_MONO, CLAP_PORT_STEREO,
};
use clap_sys::ext::audio_ports_config::{
    clap_audio_ports_config, clap_plugin_audio_ports_config, CLAP_EXT_AUDIO_PORTS_CONFIG,
};
use clap_sys::ext::draft::remote_controls::{
    clap_plugin_remote_controls, clap_remote_controls_page, CLAP_EXT_REMOTE_CONTROLS,
};
use clap_sys::ext::gui::{
    clap_gui_resize_hints, clap_host_gui, clap_plugin_gui, clap_window, CLAP_EXT_GUI,
    CLAP_WINDOW_API_COCOA, CLAP_WINDOW_API_WIN32, CLAP_WINDOW_API_X11,
};
use clap_sys::ext::latency::{clap_host_latency, clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::note_ports::{
    clap_note_port_info, clap_plugin_note_ports, CLAP_EXT_NOTE_PORTS, CLAP_NOTE_DIALECT_CLAP,
    CLAP_NOTE_DIALECT_MIDI,
};
use clap_sys::ext::params::{
    clap_host_params, clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS,
    CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_HIDDEN,
    CLAP_PARAM_IS_MODULATABLE, CLAP_PARAM_IS_MODULATABLE_PER_NOTE_ID, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED, CLAP_PARAM_RESCAN_VALUES,
};
use clap_sys::ext::render::{
    clap_plugin_render, clap_plugin_render_mode, CLAP_EXT_RENDER, CLAP_RENDER_OFFLINE,
    CLAP_RENDER_REALTIME,
};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::ext::tail::{clap_plugin_tail, CLAP_EXT_TAIL};
use clap_sys::ext::thread_check::{clap_host_thread_check, CLAP_EXT_THREAD_CHECK};
use clap_sys::ext::voice_info::{
    clap_host_voice_info, clap_plugin_voice_info, clap_voice_info, CLAP_EXT_VOICE_INFO,
    CLAP_VOICE_INFO_SUPPORTS_OVERLAPPING_NOTES,
};
use clap_sys::fixedpoint::{CLAP_BEATTIME_FACTOR, CLAP_SECTIME_FACTOR};
use clap_sys::host::clap_host;
use clap_sys::id::{clap_id, CLAP_INVALID_ID};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{
    clap_process, clap_process_status, CLAP_PROCESS_CONTINUE, CLAP_PROCESS_CONTINUE_IF_NOT_QUIET,
    CLAP_PROCESS_ERROR,
};
use clap_sys::stream::{clap_istream, clap_ostream};
use crossbeam::atomic::AtomicCell;
use crossbeam::channel::{self, SendTimeoutError};
use crossbeam::queue::ArrayQueue;
use parking_lot::Mutex;
use std::any::Any;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{c_void, CStr};
use std::mem;
use std::num::NonZeroU32;
use std::os::raw::c_char;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::thread::{self, ThreadId};
use std::time::Duration;

use super::context::{WrapperGuiContext, WrapperInitContext, WrapperProcessContext};
use super::descriptor::PluginDescriptor;
use super::util::ClapPtr;
use crate::event_loop::{BackgroundThread, EventLoop, MainThreadExecutor, TASK_QUEUE_CAPACITY};
use crate::midi::MidiResult;
use crate::prelude::{
    AsyncExecutor, AudioIOLayout, AuxiliaryBuffers, BufferConfig, ClapPlugin, Editor, MidiConfig,
    NoteEvent, ParamFlags, ParamPtr, Params, ParentWindowHandle, Plugin, PluginNoteEvent,
    ProcessMode, ProcessStatus, SysExMessage, TaskExecutor, Transport,
};
use crate::util::permit_alloc;
use crate::wrapper::clap::context::RemoteControlPages;
use crate::wrapper::clap::util::{read_stream, write_stream};
use crate::wrapper::state::{self, PluginState};
use crate::wrapper::util::buffer_management::{BufferManager, ChannelPointers};
use crate::wrapper::util::{
    clamp_input_event_timing, clamp_output_event_timing, hash_param_id, process_wrapper, strlcpy,
};

/// How many output parameter changes we can store in our output parameter change queue. Storing
/// more than this many parameters at a time will cause changes to get lost.
const OUTPUT_EVENT_QUEUE_CAPACITY: usize = 2048;

pub struct Wrapper<P: ClapPlugin> {
    /// A reference to this object, upgraded to an `Arc<Self>` for the GUI context.
    this: AtomicRefCell<Weak<Self>>,

    /// The wrapped plugin instance.
    plugin: Mutex<P>,
    /// The plugin's background task executor closure.
    pub task_executor: Mutex<TaskExecutor<P>>,
    /// The plugin's parameters. These are fetched once during initialization. That way the
    /// `ParamPtr`s are guaranteed to live at least as long as this object and we can interact with
    /// the `Params` object without having to acquire a lock on `plugin`.
    params: Arc<dyn Params>,
    /// The plugin's editor, if it has one. This object does not do anything on its own, but we need
    /// to instantiate this in advance so we don't need to lock the entire [`Plugin`] object when
    /// creating an editor. Wrapped in an `AtomicRefCell` because it needs to be initialized late.
    editor: AtomicRefCell<Option<Mutex<Box<dyn Editor>>>>,
    /// A handle for the currently active editor instance. The plugin should implement `Drop` on
    /// this handle for its closing behavior.
    editor_handle: Mutex<Option<Box<dyn Any + Send>>>,
    /// The DPI scaling factor as passed to the [IPlugViewContentScaleSupport::set_scale_factor()]
    /// function. Defaults to 1.0, and will be kept there on macOS. When reporting and handling size
    /// the sizes communicated to and from the DAW should be scaled by this factor since NIH-plug's
    /// APIs only deal in logical pixels.
    editor_scaling_factor: AtomicF32,

    is_processing: AtomicBool,
    /// The current IO configuration, modified through the `clap_plugin_audio_ports_config`
    /// extension. Initialized to the plugin's first audio IO configuration.
    current_audio_io_layout: AtomicCell<AudioIOLayout>,
    /// The current buffer configuration, containing the sample rate and the maximum block size.
    /// Will be set in `clap_plugin::activate()`.
    current_buffer_config: AtomicCell<Option<BufferConfig>>,
    /// The current audio processing mode. Set through the render extension. Defaults to realtime.
    pub current_process_mode: AtomicCell<ProcessMode>,
    /// The incoming events for the plugin, if `P::MIDI_INPUT` is set to `MidiConfig::Basic` or
    /// higher.
    ///
    /// TODO: Maybe load these lazily at some point instead of needing to spool them all to this
    ///       queue first
    input_events: AtomicRefCell<VecDeque<PluginNoteEvent<P>>>,
    /// Stores any events the plugin has output during the current processing cycle, analogous to
    /// `input_events`.
    output_events: AtomicRefCell<VecDeque<PluginNoteEvent<P>>>,
    /// The last process status returned by the plugin. This is used for tail handling.
    last_process_status: AtomicCell<ProcessStatus>,
    /// The current latency in samples, as set by the plugin through the [`ProcessContext`]. Uses
    /// the latency extension.
    pub current_latency: AtomicU32,
    /// A data structure that helps manage and create buffers for all of the plugin's inputs and
    /// outputs based on channel pointers provided by the host.
    buffer_manager: AtomicRefCell<BufferManager>,
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

    // We'll query all of the host's extensions upfront
    host_callback: ClapPtr<clap_host>,

    clap_plugin_audio_ports_config: clap_plugin_audio_ports_config,

    // The main `clap_plugin` vtable. A pointer to this `Wrapper<P>` instance is stored in the
    // `plugin_data` field. This pointer is set after creating the `Arc<Wrapper<P>>`.
    pub clap_plugin: AtomicRefCell<clap_plugin>,
    /// Needs to be boxed because the plugin object is supposed to contain a static reference to
    /// this.
    _plugin_descriptor: Box<PluginDescriptor>,

    clap_plugin_audio_ports: clap_plugin_audio_ports,

    clap_plugin_gui: clap_plugin_gui,
    host_gui: AtomicRefCell<Option<ClapPtr<clap_host_gui>>>,

    clap_plugin_latency: clap_plugin_latency,
    host_latency: AtomicRefCell<Option<ClapPtr<clap_host_latency>>>,

    clap_plugin_note_ports: clap_plugin_note_ports,

    clap_plugin_params: clap_plugin_params,
    host_params: AtomicRefCell<Option<ClapPtr<clap_host_params>>>,
    // These fields are exactly the same as their VST3 wrapper counterparts.
    //
    /// The keys from `param_map` in a stable order.
    param_hashes: Vec<u32>,
    // TODO: Merge the three `*_by_hash` hashmaps at some point
    /// A mapping from parameter ID hashes (obtained from the string parameter IDs) to pointers to
    /// parameters belonging to the plugin. These addresses will remain stable as long as the
    /// `params` object does not get deallocated.
    param_by_hash: HashMap<u32, ParamPtr>,
    /// Mappings from parameter hashes to string parameter IDs. Used for notifying the plugin's
    /// editor about parameter changes.
    param_id_by_hash: HashMap<u32, String>,
    /// The group name of a parameter, indexed by the parameter's hash. Nested groups are delimited
    /// by slashes, and they're only used to allow the DAW to display parameters in a tree
    /// structure.
    param_group_by_hash: HashMap<u32, String>,
    /// Mappings from string parameter identifiers to parameter hashes. Useful for debug logging
    /// and when storing and restoring plugin state.
    param_id_to_hash: HashMap<String, u32>,
    /// The inverse mapping from [`param_by_hash`][Self::param_by_hash]. This is needed to be able
    /// to have an ergonomic parameter setting API that uses references to the parameters instead of
    /// having to add a setter function to the parameter (or even worse, have it be completely
    /// untyped).
    pub param_ptr_to_hash: HashMap<ParamPtr, u32>,
    /// For all polyphonically modulatable parameters, mappings from the parameter hash's hash to
    /// the parameter's poly modulation ID. These IDs are then passed to the plugin, so it can
    /// quickly refer to parameter by matching on constant IDs.
    poly_mod_ids_by_hash: HashMap<u32, u32>,
    /// A queue of parameter changes and gestures that should be output in either the next process
    /// call or in the next parameter flush.
    ///
    /// XXX: There's no guarantee that a single parameter doesn't occur twice in this queue, but
    ///      even if it does then that should still not be a problem because the host also reads it
    ///      in the same order, right?
    output_parameter_events: ArrayQueue<OutputParamEvent>,

    host_thread_check: AtomicRefCell<Option<ClapPtr<clap_host_thread_check>>>,

    clap_plugin_remote_controls: clap_plugin_remote_controls,
    /// The plugin's remote control pages, if it defines any. Filled when initializing the plugin.
    remote_control_pages: Vec<clap_remote_controls_page>,

    clap_plugin_render: clap_plugin_render,

    clap_plugin_state: clap_plugin_state,

    clap_plugin_tail: clap_plugin_tail,

    clap_plugin_voice_info: clap_plugin_voice_info,
    host_voice_info: AtomicRefCell<Option<ClapPtr<clap_host_voice_info>>>,
    /// If `P::CLAP_POLY_MODULATION_CONFIG` is set, then the plugin can configure the current number
    /// of active voices using a context method called from the initialization or processing
    /// context. This defaults to the maximum number of voices.
    current_voice_capacity: AtomicU32,

    /// A queue of tasks that still need to be performed. Because CLAP lets the plugin request a
    /// host callback directly, we don't need to use the OsEventLoop we use in our other plugin
    /// implementations. Instead, we'll post tasks to this queue, ask the host to call
    /// [`on_main_thread()`][Self::on_main_thread()] on the main thread, and then continue to pop
    /// tasks off this queue there until it is empty.
    tasks: ArrayQueue<Task<P>>,
    /// The ID of the main thread. In practice this is the ID of the thread that created this
    /// object. If the host supports the thread check extension (and
    /// [`host_thread_check`][Self::host_thread_check] thus contains a value), then that extension
    /// is used instead.
    main_thread_id: ThreadId,
    /// A background thread for running tasks independently from the host'main GUI thread. Useful
    /// for longer, blocking tasks. Initialized later as it needs a reference to the wrapper.
    background_thread: AtomicRefCell<Option<BackgroundThread<Task<P>, Self>>>,
}

/// Tasks that can be sent from the plugin to be executed on the main thread in a non-blocking
/// realtime-safe way. Instead of using a random thread or the OS' event loop like in the Linux
/// implementation, this uses [`clap_host::request_callback()`] instead.
#[allow(clippy::enum_variant_names)]
pub enum Task<P: Plugin> {
    /// Execute one of the plugin's background tasks.
    PluginTask(P::BackgroundTask),
    /// Inform the plugin that one or more parameter values have changed.
    ParameterValuesChanged,
    /// Inform the plugin that one parameter's value has changed. This uses the parameter hashes
    /// since the task will be created from the audio thread.
    ParameterValueChanged(u32, f32),
    /// Inform the plugin that one parameter's modulation offset has changed. This uses the
    /// parameter hashes since the task will be created from the audio thread.
    ParameterModulationChanged(u32, f32),
    /// Inform the host that the latency has changed.
    LatencyChanged,
    /// Inform the host that the voice info has changed.
    VoiceInfoChanged,
    /// Tell the host that it should rescan the current parameter values.
    RescanParamValues,
}

/// The types of CLAP parameter updates for events.
pub enum ClapParamUpdate {
    /// Set the parameter to this plain value. In our wrapper the plain values are the normalized
    /// values multiplied by the step count for discrete parameters.
    PlainValueSet(f64),
    /// Set a normalized offset for the parameter's plain value. Subsequent modulation events
    /// override the previous one, but `PlainValueSet`s do not override the existing modulation.
    /// These values should also be divided by the step size.
    PlainValueMod(f64),
}

/// A parameter event that should be output by the plugin, stored in a queue on the wrapper and
/// written to the host either at the end of the process function or during a flush.
#[derive(Debug, Clone)]
pub enum OutputParamEvent {
    /// Begin an automation gesture. This must always be sent before sending [`SetValue`].
    BeginGesture { param_hash: u32 },
    /// Change the value of a parameter using a plain CLAP value, aka the normalized value
    /// multiplied by the number of steps.
    SetValue {
        /// The internal hash for the parameter.
        param_hash: u32,
        /// The 'plain' value as reported to CLAP. This is the normalized value multiplied by
        /// [`params::step_size()`][crate::params::step_size()].
        clap_plain_value: f64,
    },
    /// Begin an automation gesture. This must always be sent after sending one or more [`SetValue`]
    /// events.
    EndGesture { param_hash: u32 },
}

/// Because CLAP has this [`clap_host::request_host_callback()`] function, we don't need to use
/// `OsEventLoop` and can instead just request a main thread callback directly.
impl<P: ClapPlugin> EventLoop<Task<P>, Wrapper<P>> for Wrapper<P> {
    fn new_and_spawn(_executor: Weak<Self>) -> Self {
        panic!("What are you doing");
    }

    fn schedule_gui(&self, task: Task<P>) -> bool {
        if self.is_main_thread() {
            self.execute(task, true);
            true
        } else {
            let success = self.tasks.push(task).is_ok();
            if success {
                // CLAP lets us use the host's event loop instead of having to implement our own
                let host = &self.host_callback;
                unsafe_clap_call! { host=>request_callback(&**host) };
            }

            success
        }
    }

    fn schedule_background(&self, task: Task<P>) -> bool {
        self.background_thread
            .borrow()
            .as_ref()
            .unwrap()
            .schedule(task)
    }

    fn is_main_thread(&self) -> bool {
        // If the host supports the thread check interface then we'll use that, otherwise we'll
        // check if this is the same thread as the one that created the plugin instance.
        match &*self.host_thread_check.borrow() {
            Some(thread_check) => {
                unsafe_clap_call! { thread_check=>is_main_thread(&*self.host_callback) }
            }
            // FIXME: `thread::current()` may allocate the first time it's called, is there a safe
            //        non-allocating version of this without using huge OS-specific libraries?
            None => permit_alloc(|| thread::current().id() == self.main_thread_id),
        }
    }
}

impl<P: ClapPlugin> MainThreadExecutor<Task<P>> for Wrapper<P> {
    fn execute(&self, task: Task<P>, is_gui_thread: bool) {
        // This function is always called from the main thread, from [Self::on_main_thread].
        match task {
            Task::PluginTask(task) => (self.task_executor.lock())(task),
            Task::ParameterValuesChanged => {
                if self.editor_handle.lock().is_some() {
                    if let Some(editor) = self.editor.borrow().as_ref() {
                        editor.lock().param_values_changed();
                    }
                }
            }
            Task::ParameterValueChanged(param_hash, normalized_value) => {
                if self.editor_handle.lock().is_some() {
                    if let Some(editor) = self.editor.borrow().as_ref() {
                        let param_id = &self.param_id_by_hash[&param_hash];
                        editor
                            .lock()
                            .param_value_changed(param_id, normalized_value);
                    }
                }
            }
            Task::ParameterModulationChanged(param_hash, modulation_offset) => {
                if self.editor_handle.lock().is_some() {
                    if let Some(editor) = self.editor.borrow().as_ref() {
                        let param_id = &self.param_id_by_hash[&param_hash];
                        editor
                            .lock()
                            .param_modulation_changed(param_id, modulation_offset);
                    }
                }
            }
            Task::LatencyChanged => match &*self.host_latency.borrow() {
                Some(host_latency) => {
                    nih_debug_assert!(is_gui_thread);

                    // XXX: The CLAP docs mention that you should request a restart if this happens
                    //      while the plugin is activated (which is not entirely the same thing as
                    //      is processing, but we'll treat it as the same thing). In practice just
                    //      calling the latency changed function also seems to work just fine.
                    if self.is_processing.load(Ordering::SeqCst) {
                        unsafe_clap_call! { &*self.host_callback=>request_restart(&*self.host_callback) };
                    } else {
                        unsafe_clap_call! { host_latency=>changed(&*self.host_callback) };
                    }
                }
                None => nih_debug_assert_failure!("Host does not support the latency extension"),
            },
            Task::VoiceInfoChanged => match &*self.host_voice_info.borrow() {
                Some(host_voice_info) => {
                    nih_debug_assert!(is_gui_thread);
                    unsafe_clap_call! { host_voice_info=>changed(&*self.host_callback) };
                }
                None => nih_debug_assert_failure!("Host does not support the voice-info extension"),
            },
            Task::RescanParamValues => match &*self.host_params.borrow() {
                Some(host_params) => {
                    nih_debug_assert!(is_gui_thread);
                    unsafe_clap_call! { host_params=>rescan(&*self.host_callback, CLAP_PARAM_RESCAN_VALUES) };
                }
                None => nih_debug_assert_failure!("The host does not support parameters? What?"),
            },
        };
    }
}

impl<P: ClapPlugin> Wrapper<P> {
    /// # Safety
    ///
    /// `host_callback` needs to outlive the returned object.
    pub unsafe fn new(host_callback: *const clap_host) -> Arc<Self> {
        let mut plugin = P::default();
        let task_executor = Mutex::new(plugin.task_executor());

        // This is used to allow the plugin to restore preset data from its editor, see the comment
        // on `Self::updated_state_sender`
        let (updated_state_sender, updated_state_receiver) = channel::bounded(0);

        let plugin_descriptor: Box<PluginDescriptor> =
            Box::new(PluginDescriptor::for_plugin::<P>());

        // We're not allowed to query any extensions until the init function has been called, so we
        // need a bunch of AtomicRefCells instead
        assert!(!host_callback.is_null());
        let host_callback = unsafe { ClapPtr::new(host_callback) };

        // This is a mapping from the parameter IDs specified by the plugin to pointers to those
        // parameters. These pointers are assumed to be safe to dereference as long as
        // `wrapper.plugin` is alive. The plugin API identifiers these parameters by hashes, which
        // we'll calculate from the string ID specified by the plugin. These parameters should also
        // remain in the same order as the one returned by the plugin.
        let params = plugin.params();
        let param_id_hashes_ptrs_groups: Vec<_> = params
            .param_map()
            .into_iter()
            .map(|(id, ptr, group)| {
                let hash = hash_param_id(&id);
                (id, hash, ptr, group)
            })
            .collect();
        let param_hashes = param_id_hashes_ptrs_groups
            .iter()
            .map(|(_, hash, _, _)| *hash)
            .collect();
        let param_by_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(_, hash, ptr, _)| (*hash, *ptr))
            .collect();
        let param_id_by_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(id, hash, _, _)| (*hash, id.clone()))
            .collect();
        let param_group_by_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(_, hash, _, group)| (*hash, group.clone()))
            .collect();
        let param_id_to_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(id, hash, _, _)| (id.clone(), *hash))
            .collect();
        let param_ptr_to_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(_, hash, ptr, _)| (*ptr, *hash))
            .collect();
        let poly_mod_ids_by_hash: HashMap<u32, u32> = param_id_hashes_ptrs_groups
            .iter()
            .filter_map(|(_, hash, ptr, _)| unsafe {
                ptr.poly_modulation_id().map(|id| (*hash, id))
            })
            .collect();

        if cfg!(debug_assertions) {
            let param_map = params.param_map();
            let param_ids: HashSet<_> = param_id_hashes_ptrs_groups
                .iter()
                .map(|(id, _, _, _)| id.clone())
                .collect();
            nih_debug_assert_eq!(
                param_map.len(),
                param_ids.len(),
                "The plugin has duplicate parameter IDs, weird things may happen. Consider using \
                 6 character parameter IDs to avoid collisions."
            );

            let poly_mod_ids: HashSet<u32> = poly_mod_ids_by_hash.values().copied().collect();
            nih_debug_assert_eq!(
                poly_mod_ids_by_hash.len(),
                poly_mod_ids.len(),
                "The plugin has duplicate poly modulation IDs. Polyphonic modulation will not be \
                 routed to the correct parameter."
            );

            let mut bypass_param_exists = false;
            for (_, _, ptr, _) in &param_id_hashes_ptrs_groups {
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

        // Support for the remote controls extension
        let mut remote_control_pages = Vec::new();
        RemoteControlPages::define_remote_control_pages(
            &plugin,
            &mut remote_control_pages,
            &param_ptr_to_hash,
        );

        let wrapper = Self {
            this: AtomicRefCell::new(Weak::new()),

            plugin: Mutex::new(plugin),
            task_executor,
            params,
            // Initialized later as it needs a reference to the wrapper for the async executor
            editor: AtomicRefCell::new(None),
            editor_handle: Mutex::new(None),
            editor_scaling_factor: AtomicF32::new(1.0),

            is_processing: AtomicBool::new(false),
            current_audio_io_layout: AtomicCell::new(
                P::AUDIO_IO_LAYOUTS.first().copied().unwrap_or_default(),
            ),
            current_buffer_config: AtomicCell::new(None),
            current_process_mode: AtomicCell::new(ProcessMode::Realtime),
            input_events: AtomicRefCell::new(VecDeque::with_capacity(512)),
            output_events: AtomicRefCell::new(VecDeque::with_capacity(512)),
            last_process_status: AtomicCell::new(ProcessStatus::Normal),
            current_latency: AtomicU32::new(0),
            // This is initialized just before calling `Plugin::initialize()` so that during the
            // process call buffers can be initialized without any allocations
            buffer_manager: AtomicRefCell::new(BufferManager::for_audio_io_layout(
                0,
                AudioIOLayout::default(),
            )),
            updated_state_sender,
            updated_state_receiver,

            host_callback,

            clap_plugin: AtomicRefCell::new(clap_plugin {
                // This needs to live on the heap because the plugin object contains a direct
                // reference to the manifest as a value. We could share this between instances of
                // the plugin using an `Arc`, but this doesn't consume a lot of memory so it's not a
                // huge deal.
                desc: plugin_descriptor.clap_plugin_descriptor(),
                // This pointer will be set to point at our wrapper instance later
                plugin_data: std::ptr::null_mut(),
                init: Some(Self::init),
                destroy: Some(Self::destroy),
                activate: Some(Self::activate),
                deactivate: Some(Self::deactivate),
                start_processing: Some(Self::start_processing),
                stop_processing: Some(Self::stop_processing),
                reset: Some(Self::reset),
                process: Some(Self::process),
                get_extension: Some(Self::get_extension),
                on_main_thread: Some(Self::on_main_thread),
            }),
            _plugin_descriptor: plugin_descriptor,

            clap_plugin_audio_ports_config: clap_plugin_audio_ports_config {
                count: Some(Self::ext_audio_ports_config_count),
                get: Some(Self::ext_audio_ports_config_get),
                select: Some(Self::ext_audio_ports_config_select),
            },

            clap_plugin_audio_ports: clap_plugin_audio_ports {
                count: Some(Self::ext_audio_ports_count),
                get: Some(Self::ext_audio_ports_get),
            },

            clap_plugin_gui: clap_plugin_gui {
                is_api_supported: Some(Self::ext_gui_is_api_supported),
                get_preferred_api: Some(Self::ext_gui_get_preferred_api),
                create: Some(Self::ext_gui_create),
                destroy: Some(Self::ext_gui_destroy),
                set_scale: Some(Self::ext_gui_set_scale),
                get_size: Some(Self::ext_gui_get_size),
                can_resize: Some(Self::ext_gui_can_resize),
                get_resize_hints: Some(Self::ext_gui_get_resize_hints),
                adjust_size: Some(Self::ext_gui_adjust_size),
                set_size: Some(Self::ext_gui_set_size),
                set_parent: Some(Self::ext_gui_set_parent),
                set_transient: Some(Self::ext_gui_set_transient),
                suggest_title: Some(Self::ext_gui_suggest_title),
                show: Some(Self::ext_gui_show),
                hide: Some(Self::ext_gui_hide),
            },
            host_gui: AtomicRefCell::new(None),

            clap_plugin_latency: clap_plugin_latency {
                get: Some(Self::ext_latency_get),
            },
            host_latency: AtomicRefCell::new(None),

            clap_plugin_note_ports: clap_plugin_note_ports {
                count: Some(Self::ext_note_ports_count),
                get: Some(Self::ext_note_ports_get),
            },

            clap_plugin_params: clap_plugin_params {
                count: Some(Self::ext_params_count),
                get_info: Some(Self::ext_params_get_info),
                get_value: Some(Self::ext_params_get_value),
                value_to_text: Some(Self::ext_params_value_to_text),
                text_to_value: Some(Self::ext_params_text_to_value),
                flush: Some(Self::ext_params_flush),
            },
            host_params: AtomicRefCell::new(None),
            param_hashes,
            param_by_hash,
            param_id_by_hash,
            param_group_by_hash,
            param_id_to_hash,
            param_ptr_to_hash,
            poly_mod_ids_by_hash,
            output_parameter_events: ArrayQueue::new(OUTPUT_EVENT_QUEUE_CAPACITY),

            host_thread_check: AtomicRefCell::new(None),

            clap_plugin_remote_controls: clap_plugin_remote_controls {
                count: Some(Self::ext_remote_controls_count),
                get: Some(Self::ext_remote_controls_get),
            },
            remote_control_pages,

            clap_plugin_render: clap_plugin_render {
                has_hard_realtime_requirement: Some(Self::ext_render_has_hard_realtime_requirement),
                set: Some(Self::ext_render_set),
            },

            clap_plugin_state: clap_plugin_state {
                save: Some(Self::ext_state_save),
                load: Some(Self::ext_state_load),
            },

            clap_plugin_tail: clap_plugin_tail {
                get: Some(Self::ext_tail_get),
            },

            clap_plugin_voice_info: clap_plugin_voice_info {
                get: Some(Self::ext_voice_info_get),
            },
            host_voice_info: AtomicRefCell::new(None),
            current_voice_capacity: AtomicU32::new(
                P::CLAP_POLY_MODULATION_CONFIG
                    .map(|c| {
                        nih_debug_assert!(
                            c.max_voice_capacity >= 1,
                            "The maximum voice capacity cannot be zero"
                        );
                        c.max_voice_capacity
                    })
                    .unwrap_or(1),
            ),

            tasks: ArrayQueue::new(TASK_QUEUE_CAPACITY),
            main_thread_id: thread::current().id(),
            // Initialized later as it needs a reference to the wrapper for the executor
            background_thread: AtomicRefCell::new(None),
        };

        // Finally, the wrapper needs to contain a reference to itself so we can create GuiContexts
        // when opening plugin editors
        let wrapper = Arc::new(wrapper);
        *wrapper.this.borrow_mut() = Arc::downgrade(&wrapper);

        // The `clap_plugin::plugin_data` field needs to point to this wrapper so we can access it
        // from the vtable functions
        wrapper.clap_plugin.borrow_mut().plugin_data = Arc::as_ptr(&wrapper) as *mut _;

        // The editor also needs to be initialized later so the Async executor can work.
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
            .map(Mutex::new);

        // Same with the background thread
        *wrapper.background_thread.borrow_mut() =
            Some(BackgroundThread::get_or_create(Arc::downgrade(&wrapper)));

        wrapper
    }

    fn make_gui_context(self: Arc<Self>) -> Arc<WrapperGuiContext<P>> {
        Arc::new(WrapperGuiContext {
            wrapper: self,
            #[cfg(debug_assertions)]
            param_gesture_checker: Default::default(),
        })
    }

    /// # Note
    ///
    /// The lock on the plugin must be dropped before this object is dropped to avoid deadlocks
    /// caused by reentrant function calls.
    fn make_init_context(&self) -> WrapperInitContext<'_, P> {
        WrapperInitContext {
            wrapper: self,
            pending_requests: Default::default(),
        }
    }

    fn make_process_context(&self, transport: Transport) -> WrapperProcessContext<'_, P> {
        WrapperProcessContext {
            wrapper: self,
            input_events_guard: self.input_events.borrow_mut(),
            output_events_guard: self.output_events.borrow_mut(),
            transport,
        }
    }

    /// Get a parameter's ID based on a `ParamPtr`. Used in the `GuiContext` implementation for the
    /// gesture checks.
    #[allow(unused)]
    pub fn param_id_from_ptr(&self, param: ParamPtr) -> Option<&str> {
        self.param_ptr_to_hash
            .get(&param)
            .and_then(|hash| self.param_id_by_hash.get(hash))
            .map(|s| s.as_str())
    }

    /// Queue a parameter output event to be sent to the host at the end of the audio processing
    /// cycle, and request a parameter flush from the host if the plugin is not currently processing
    /// audio. The parameter's actual value will only be updated at that point so the value won't
    /// change in the middle of a processing call.
    ///
    /// Returns `false` if the parameter value queue was full and the update will not be sent to the
    /// host (it will still be set on the plugin either way).
    pub fn queue_parameter_event(&self, event: OutputParamEvent) -> bool {
        let result = self.output_parameter_events.push(event).is_ok();

        // Requesting a flush is fine even during audio processing. This avoids a race condition.
        match &*self.host_params.borrow() {
            Some(host_params) => {
                unsafe_clap_call! { host_params=>request_flush(&*self.host_callback) }
            }
            None => nih_debug_assert_failure!("The host does not support parameters? What?"),
        }

        result
    }

    /// Request a resize based on the editor's current reported size. As of CLAP 0.24 this can
    /// safely be called from any thread. If this returns `false`, then the plugin should reset its
    /// size back to the previous value.
    pub fn request_resize(&self) -> bool {
        match (
            self.host_gui.borrow().as_ref(),
            self.editor.borrow().as_ref(),
        ) {
            (Some(host_gui), Some(editor)) => {
                let (unscaled_width, unscaled_height) = editor.lock().size();
                let scaling_factor = self.editor_scaling_factor.load(Ordering::Relaxed);

                unsafe_clap_call! {
                    host_gui=>request_resize(
                        &*self.host_callback,
                        (unscaled_width as f32 * scaling_factor).round() as u32,
                        (unscaled_height as f32 * scaling_factor).round() as u32,
                    )
                }
            }
            _ => false,
        }
    }

    /// Convenience function for setting a value for a parameter as triggered by a VST3 parameter
    /// update. The same rate is for updating parameter smoothing.
    ///
    /// After calling this function, you should call
    /// [`notify_param_values_changed()`][Self::notify_param_values_changed()] to allow the editor
    /// to update itself. This needs to be done separately so you can process parameter changes in
    /// batches.
    ///
    /// # Note
    ///
    /// These values are CLAP plain values, which include a step count multiplier for discrete
    /// parameter values.
    pub fn update_plain_value_by_hash(
        &self,
        hash: u32,
        update_type: ClapParamUpdate,
        sample_rate: Option<f32>,
    ) -> bool {
        match self.param_by_hash.get(&hash) {
            Some(param_ptr) => {
                match update_type {
                    ClapParamUpdate::PlainValueSet(clap_plain_value) => {
                        let normalized_value = clap_plain_value as f32
                            / unsafe { param_ptr.step_count() }.unwrap_or(1) as f32;

                        if unsafe { param_ptr.set_normalized_value(normalized_value) } {
                            if let Some(sample_rate) = sample_rate {
                                unsafe { param_ptr.update_smoother(sample_rate, false) };
                            }

                            // The GUI needs to be informed about the changed parameter value. This
                            // triggers an `Editor::param_value_changed()` call on the GUI thread.
                            let task_posted = self
                                .schedule_gui(Task::ParameterValueChanged(hash, normalized_value));
                            nih_debug_assert!(
                                task_posted,
                                "The task queue is full, dropping task..."
                            );
                        }

                        true
                    }
                    ClapParamUpdate::PlainValueMod(clap_plain_delta) => {
                        let normalized_delta = clap_plain_delta as f32
                            / unsafe { param_ptr.step_count() }.unwrap_or(1) as f32;

                        if unsafe { param_ptr.modulate_value(normalized_delta) } {
                            if let Some(sample_rate) = sample_rate {
                                unsafe { param_ptr.update_smoother(sample_rate, false) };
                            }

                            let task_posted = self.schedule_gui(Task::ParameterModulationChanged(
                                hash,
                                normalized_delta,
                            ));
                            nih_debug_assert!(
                                task_posted,
                                "The task queue is full, dropping task..."
                            );
                        }

                        true
                    }
                }
            }
            _ => false,
        }
    }

    /// Handle all incoming events from an event queue. This will clear `self.input_events` first.
    ///
    /// # Safety
    ///
    /// `in_` must contain only pointers to valid data (Clippy insists on there being a safety
    /// section here).
    pub unsafe fn handle_in_events(
        &self,
        in_: &clap_input_events,
        current_sample_idx: usize,
        total_buffer_len: usize,
    ) {
        let mut input_events = self.input_events.borrow_mut();
        input_events.clear();

        let num_events = clap_call! { in_=>size(in_) };
        for event_idx in 0..num_events {
            let event = clap_call! { in_=>get(in_, event_idx) };
            self.handle_in_event(
                event,
                &mut input_events,
                None,
                current_sample_idx,
                total_buffer_len,
            );
        }
    }

    /// Similar to [`handle_in_events()`][Self::handle_in_events()], but will stop just before an
    /// event if the predicate returns true for that events. This predicate is only called for
    /// events that occur after `current_sample_idx`. This is used to stop before a tempo or time
    /// signature change, or before next parameter change event with `raw_event.time >
    /// current_sample_idx` and return the **absolute** (relative to the entire buffer that's being
    /// split) sample index of that event along with the its index in the event queue as a
    /// `(sample_idx, event_idx)` tuple. This allows for splitting the audio buffer into segments
    /// with distinct sample values to enable sample accurate automation without modifications to the
    /// wrapped plugin.
    ///
    /// # Safety
    ///
    /// `in_` must contain only pointers to valid data (Clippy insists on there being a safety
    /// section here).
    pub unsafe fn handle_in_events_until(
        &self,
        in_: &clap_input_events,
        transport_info: &mut *const clap_event_transport,
        current_sample_idx: usize,
        total_buffer_len: usize,
        resume_from_event_idx: usize,
        stop_predicate: impl Fn(*const clap_event_header) -> bool,
    ) -> Option<(usize, usize)> {
        let mut input_events = self.input_events.borrow_mut();
        input_events.clear();

        // To achieve this, we'll always read one event ahead
        let num_events = clap_call! { in_=>size(in_) };
        if num_events == 0 {
            return None;
        }

        let start_idx = resume_from_event_idx as u32;
        let mut event: *const clap_event_header = clap_call! { in_=>get(in_, start_idx) };
        for next_event_idx in (start_idx + 1)..num_events {
            self.handle_in_event(
                event,
                &mut input_events,
                Some(transport_info),
                current_sample_idx,
                total_buffer_len,
            );

            // Stop just before the next parameter change or transport information event at a sample
            // after the current sample
            let next_event: *const clap_event_header = clap_call! { in_=>get(in_, next_event_idx) };
            if (*next_event).time > current_sample_idx as u32 && stop_predicate(next_event) {
                return Some(((*next_event).time as usize, next_event_idx as usize));
            }

            event = next_event;
        }

        // Don't forget about the last event
        self.handle_in_event(
            event,
            &mut input_events,
            Some(transport_info),
            current_sample_idx,
            total_buffer_len,
        );

        None
    }

    /// Write the unflushed parameter changes to the host's output event queue. The sample index is
    /// used as part of splitting up the input buffer for sample accurate automation changes. This
    /// will also modify the actual parameter values, since we should only do that while the wrapped
    /// plugin is not actually processing audio.
    ///
    /// The `total_buffer_len` argument is used to clamp out of bounds events to the buffer's length.
    ///
    /// # Safety
    ///
    /// `out` must be a valid object (Clippy insists on there being a safety section here).
    pub unsafe fn handle_out_events(
        &self,
        out: &clap_output_events,
        current_sample_idx: usize,
        total_buffer_len: usize,
    ) {
        // We'll always write these events to the first sample, so even when we add note output we
        // shouldn't have to think about interleaving events here
        let sample_rate = self.current_buffer_config.load().map(|c| c.sample_rate);
        while let Some(change) = self.output_parameter_events.pop() {
            let push_successful = match change {
                OutputParamEvent::BeginGesture { param_hash } => {
                    let event = clap_event_param_gesture {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_param_gesture>() as u32,
                            time: current_sample_idx as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_PARAM_GESTURE_BEGIN,
                            flags: CLAP_EVENT_IS_LIVE,
                        },
                        param_id: param_hash,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                OutputParamEvent::SetValue {
                    param_hash,
                    clap_plain_value,
                } => {
                    self.update_plain_value_by_hash(
                        param_hash,
                        ClapParamUpdate::PlainValueSet(clap_plain_value),
                        sample_rate,
                    );

                    let event = clap_event_param_value {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_param_value>() as u32,
                            time: current_sample_idx as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_PARAM_VALUE,
                            flags: CLAP_EVENT_IS_LIVE,
                        },
                        param_id: param_hash,
                        cookie: std::ptr::null_mut(),
                        port_index: -1,
                        note_id: -1,
                        channel: -1,
                        key: -1,
                        value: clap_plain_value,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                OutputParamEvent::EndGesture { param_hash } => {
                    let event = clap_event_param_gesture {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_param_gesture>() as u32,
                            time: current_sample_idx as u32,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_PARAM_GESTURE_END,
                            flags: CLAP_EVENT_IS_LIVE,
                        },
                        param_id: param_hash,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
            };

            nih_debug_assert!(push_successful);
        }

        // Also send all note events generated by the plugin
        let mut output_events = self.output_events.borrow_mut();
        while let Some(event) = output_events.pop_front() {
            // Out of bounds events are clamped to the buffer's size
            let time = clamp_output_event_timing(
                event.timing() + current_sample_idx as u32,
                total_buffer_len as u32,
            );

            let push_successful = match event {
                NoteEvent::NoteOn {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    velocity,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_ON,
                            // We don't have a way to denote live events
                            flags: 0,
                        },
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        velocity: velocity as f64,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::NoteOff {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    velocity,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_OFF,
                            flags: 0,
                        },
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        velocity: velocity as f64,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                // NOTE: This is gated behind `P::MIDI_INPUT`, because this is a merely a hint event
                //       for the host. It is not output to any other plugin or device.
                NoteEvent::VoiceTerminated {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                } if P::MIDI_INPUT >= MidiConfig::Basic => {
                    let event = clap_event_note {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_END,
                            flags: 0,
                        },
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        velocity: 0.0,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::PolyPressure {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    pressure,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note_expression {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note_expression>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        expression_id: CLAP_NOTE_EXPRESSION_PRESSURE,
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        value: pressure as f64,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::PolyVolume {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    gain,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note_expression {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note_expression>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        expression_id: CLAP_NOTE_EXPRESSION_VOLUME,
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        value: gain as f64,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::PolyPan {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    pan,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note_expression {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note_expression>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        expression_id: CLAP_NOTE_EXPRESSION_PAN,
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        value: (pan as f64 + 1.0) / 2.0,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::PolyTuning {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    tuning,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note_expression {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note_expression>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        expression_id: CLAP_NOTE_EXPRESSION_TUNING,
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        value: tuning as f64,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::PolyVibrato {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    vibrato,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note_expression {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note_expression>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        expression_id: CLAP_NOTE_EXPRESSION_VIBRATO,
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        value: vibrato as f64,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::PolyExpression {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    expression,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note_expression {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note_expression>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        expression_id: CLAP_NOTE_EXPRESSION_EXPRESSION,
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        value: expression as f64,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::PolyBrightness {
                    timing: _,
                    voice_id,
                    channel,
                    note,
                    brightness,
                } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                    let event = clap_event_note_expression {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_note_expression>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        expression_id: CLAP_NOTE_EXPRESSION_BRIGHTNESS,
                        note_id: voice_id.unwrap_or(-1),
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        value: brightness as f64,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                midi_event @ (NoteEvent::MidiChannelPressure { .. }
                | NoteEvent::MidiPitchBend { .. }
                | NoteEvent::MidiCC { .. }
                | NoteEvent::MidiProgramChange { .. })
                    if P::MIDI_OUTPUT >= MidiConfig::MidiCCs =>
                {
                    // NIH-plug already includes MIDI conversion functions, so we'll reuse those for
                    // the MIDI events
                    let midi_data = match midi_event.as_midi() {
                        Some(MidiResult::Basic(midi_data)) => midi_data,
                        Some(MidiResult::SysEx(_, _)) => unreachable!(
                            "Basic MIDI event read as SysEx, something's gone horribly wrong"
                        ),
                        None => unreachable!("Missing MIDI conversion for MIDI event"),
                    };

                    let event = clap_event_midi {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_midi>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_MIDI,
                            flags: 0,
                        },
                        port_index: 0,
                        data: midi_data,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                NoteEvent::MidiSysEx { timing: _, message }
                    if P::MIDI_OUTPUT >= MidiConfig::Basic =>
                {
                    // SysEx is supported on the basic MIDI config so this is separate
                    let (padded_sysex_buffer, length) = message.to_buffer();
                    let padded_sysex_buffer = padded_sysex_buffer.borrow();
                    nih_debug_assert!(padded_sysex_buffer.len() >= length);
                    let sysex_buffer = &padded_sysex_buffer[..length];

                    let event = clap_event_midi_sysex {
                        header: clap_event_header {
                            size: mem::size_of::<clap_event_midi_sysex>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_MIDI_SYSEX,
                            flags: 0,
                        },
                        port_index: 0,
                        // The host _should_ be making a copy of the data if it accepts the event. Should...
                        buffer: sysex_buffer.as_ptr(),
                        size: sysex_buffer.len() as u32,
                    };

                    clap_call! { out=>try_push(out, &event.header) }
                }
                _ => {
                    nih_debug_assert_failure!(
                        "Invalid output event for the current MIDI_OUTPUT setting"
                    );
                    continue;
                }
            };

            nih_debug_assert!(push_successful, "Could not send note event");
        }
    }

    /// Handle an incoming CLAP event. The sample index is provided to support block splitting for
    /// sample accurate automation. [`input_events`][Self::input_events] must be cleared at the
    /// start of each process block.
    ///
    /// To save on mutex operations when handing MIDI events, the lock guard for the input events
    /// need to be passed into this function.
    ///
    /// If the event was a transport event and the `transport_info` argument is not `None`, then the
    /// pointer will be changed to point to the transport information from this event.
    ///
    /// # Safety
    ///
    /// `in_` must contain only pointers to valid data (Clippy insists on there being a safety
    /// section here).
    pub unsafe fn handle_in_event(
        &self,
        event: *const clap_event_header,
        input_events: &mut AtomicRefMut<VecDeque<PluginNoteEvent<P>>>,
        transport_info: Option<&mut *const clap_event_transport>,
        current_sample_idx: usize,
        total_buffer_len: usize,
    ) {
        let raw_event = &*event;

        // Out of bounds events are clamped to the buffer's size
        let timing = clamp_input_event_timing(
            raw_event.time - current_sample_idx as u32,
            total_buffer_len as u32,
        );

        match (raw_event.space_id, raw_event.type_) {
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE) => {
                let event = &*(event as *const clap_event_param_value);
                self.update_plain_value_by_hash(
                    event.param_id,
                    ClapParamUpdate::PlainValueSet(event.value),
                    self.current_buffer_config.load().map(|c| c.sample_rate),
                );

                // If the parameter supports polyphonic modulation, then the plugin needs to be
                // informed that the parameter has been monophonically automated. This allows the
                // plugin to update all of its polyphonic modulation values, since polyphonic
                // modulation acts as an offset to the monophonic value.
                if let Some(poly_modulation_id) = self.poly_mod_ids_by_hash.get(&event.param_id) {
                    // The modulation offset needs to be normalized to account for modulated
                    // integer or enum parameters
                    let param_ptr = self.param_by_hash[&event.param_id];
                    let normalized_value =
                        event.value as f32 / param_ptr.step_count().unwrap_or(1) as f32;

                    input_events.push_back(NoteEvent::MonoAutomation {
                        timing,
                        poly_modulation_id: *poly_modulation_id,
                        normalized_value,
                    });
                }
            }
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_MOD) => {
                let event = &*(event as *const clap_event_param_mod);

                if event.note_id != -1 && P::MIDI_INPUT >= MidiConfig::Basic {
                    match self.poly_mod_ids_by_hash.get(&event.param_id) {
                        Some(poly_modulation_id) => {
                            // The modulation offset needs to be normalized to account for modulated
                            // integer or enum parameters
                            let param_ptr = self.param_by_hash[&event.param_id];
                            let normalized_offset =
                                event.amount as f32 / param_ptr.step_count().unwrap_or(1) as f32;

                            // The host may also add key and channel information here, but it may
                            // also pass -1. So not having that information here at all seems like
                            // the safest choice.
                            input_events.push_back(NoteEvent::PolyModulation {
                                timing,
                                voice_id: event.note_id,
                                poly_modulation_id: *poly_modulation_id,
                                normalized_offset,
                            });

                            return;
                        }
                        None => nih_debug_assert_failure!(
                            "Polyphonic modulation sent for a parameter without a poly modulation \
                             ID"
                        ),
                    }
                }

                self.update_plain_value_by_hash(
                    event.param_id,
                    ClapParamUpdate::PlainValueMod(event.amount),
                    self.current_buffer_config.load().map(|c| c.sample_rate),
                );
            }
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_TRANSPORT) => {
                let event = &*(event as *const clap_event_transport);
                if let Some(transport_info) = transport_info {
                    *transport_info = event;
                }
            }
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_ON) => {
                if P::MIDI_INPUT >= MidiConfig::Basic {
                    let event = &*(event as *const clap_event_note);
                    input_events.push_back(NoteEvent::NoteOn {
                        // When splitting up the buffer for sample accurate automation all events
                        // should be relative to the block
                        timing,
                        voice_id: if event.note_id != -1 {
                            Some(event.note_id)
                        } else {
                            None
                        },
                        channel: event.channel as u8,
                        note: event.key as u8,
                        velocity: event.velocity as f32,
                    });
                }
            }
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_OFF) => {
                if P::MIDI_INPUT >= MidiConfig::Basic {
                    let event = &*(event as *const clap_event_note);
                    input_events.push_back(NoteEvent::NoteOff {
                        timing,
                        voice_id: if event.note_id != -1 {
                            Some(event.note_id)
                        } else {
                            None
                        },
                        channel: event.channel as u8,
                        note: event.key as u8,
                        velocity: event.velocity as f32,
                    });
                }
            }
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_CHOKE) => {
                if P::MIDI_INPUT >= MidiConfig::Basic {
                    let event = &*(event as *const clap_event_note);
                    input_events.push_back(NoteEvent::Choke {
                        timing,
                        voice_id: if event.note_id != -1 {
                            Some(event.note_id)
                        } else {
                            None
                        },
                        // FIXME: These values are also allowed to be -1, we need to support that
                        channel: event.channel as u8,
                        note: event.key as u8,
                    });
                }
            }
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_EXPRESSION) => {
                if P::MIDI_INPUT >= MidiConfig::Basic {
                    // TODO: Add support for the other expression types
                    let event = &*(event as *const clap_event_note_expression);
                    match event.expression_id {
                        CLAP_NOTE_EXPRESSION_PRESSURE => {
                            input_events.push_back(NoteEvent::PolyPressure {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.key as u8,
                                pressure: event.value as f32,
                            });
                        }
                        CLAP_NOTE_EXPRESSION_VOLUME => {
                            input_events.push_back(NoteEvent::PolyVolume {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.key as u8,
                                gain: event.value as f32,
                            });
                        }
                        CLAP_NOTE_EXPRESSION_PAN => {
                            input_events.push_back(NoteEvent::PolyPan {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.key as u8,
                                // In CLAP this value goes from [0, 1] instead of [-1, 1]
                                pan: (event.value as f32 * 2.0) - 1.0,
                            });
                        }
                        CLAP_NOTE_EXPRESSION_TUNING => {
                            input_events.push_back(NoteEvent::PolyTuning {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.key as u8,
                                tuning: event.value as f32,
                            });
                        }
                        CLAP_NOTE_EXPRESSION_VIBRATO => {
                            input_events.push_back(NoteEvent::PolyVibrato {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.key as u8,
                                vibrato: event.value as f32,
                            });
                        }
                        CLAP_NOTE_EXPRESSION_EXPRESSION => {
                            input_events.push_back(NoteEvent::PolyExpression {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.key as u8,
                                expression: event.value as f32,
                            });
                        }
                        CLAP_NOTE_EXPRESSION_BRIGHTNESS => {
                            input_events.push_back(NoteEvent::PolyBrightness {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.key as u8,
                                brightness: event.value as f32,
                            });
                        }
                        n => nih_debug_assert_failure!("Unhandled note expression ID {}", n),
                    }
                }
            }
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI) => {
                // In the Basic note port type, we'll still handle note on, note off, and polyphonic
                // pressure events if the host sents us those. But we'll throw away any other MIDI
                // messages to stay consistent with the VST3 wrapper.
                let event = &*(event as *const clap_event_midi);

                match NoteEvent::from_midi(timing, &event.data) {
                    Ok(
                        note_event @ (NoteEvent::NoteOn { .. }
                        | NoteEvent::NoteOff { .. }
                        | NoteEvent::PolyPressure { .. }),
                    ) if P::MIDI_INPUT >= MidiConfig::Basic => {
                        input_events.push_back(note_event);
                    }
                    Ok(note_event) if P::MIDI_INPUT >= MidiConfig::MidiCCs => {
                        input_events.push_back(note_event);
                    }
                    Ok(_) => (),
                    Err(n) => nih_debug_assert_failure!("Unhandled MIDI message type {}", n),
                };
            }
            (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI_SYSEX)
                if P::MIDI_INPUT >= MidiConfig::Basic =>
            {
                let event = &*(event as *const clap_event_midi_sysex);

                // `NoteEvent::from_midi` prints some tracing if parsing fails, which is not
                // necessarily an error
                assert!(!event.buffer.is_null());
                let sysex_buffer = std::slice::from_raw_parts(event.buffer, event.size as usize);
                if let Ok(note_event) = NoteEvent::from_midi(timing, sysex_buffer) {
                    input_events.push_back(note_event);
                };
            }
            _ => {
                nih_trace!(
                    "Unhandled CLAP event type {} for namespace {}",
                    raw_event.type_,
                    raw_event.space_id
                );
            }
        }
    }

    /// Get the plugin's state object, may be called by the plugin's GUI as part of its own preset
    /// management. The wrapper doesn't use these functions and serializes and deserializes directly
    /// the JSON in the relevant plugin API methods instead.
    pub fn get_state_object(&self) -> PluginState {
        unsafe {
            state::serialize_object::<P>(
                self.params.clone(),
                state::make_params_iter(&self.param_by_hash, &self.param_id_to_hash),
            )
        }
    }

    /// Update the plugin's internal state, called by the plugin itself from the GUI thread. To
    /// prevent corrupting data and changing parameters during processing the actual state is only
    /// updated at the end of the audio processing cycle.
    pub fn set_state_object_from_gui(&self, mut state: PluginState) {
        // Use a loop and timeouts to handle the super rare edge case when this function gets called
        // between a process call and the host disabling the plugin
        loop {
            if self.is_processing.load(Ordering::SeqCst) {
                // If the plugin is currently processing audio, then we'll perform the restore
                // operation at the end of the audio call. This involves sending the state to the
                // audio thread, having the audio thread handle the state restore at the very end of
                // the process function, and then sending the state back to this thread so it can be
                // deallocated without blocking the audio thread.
                match self
                    .updated_state_sender
                    .send_timeout(state, Duration::from_secs(1))
                {
                    Ok(_) => {
                        // As mentioned above, the state object will be passed back to this thread
                        // so we can deallocate it without blocking.
                        let state = self.updated_state_receiver.recv();
                        drop(state);
                        break;
                    }
                    Err(SendTimeoutError::Timeout(value)) => {
                        state = value;
                        continue;
                    }
                    Err(SendTimeoutError::Disconnected(_)) => {
                        nih_debug_assert_failure!("State update channel got disconnected");
                        return;
                    }
                }
            } else {
                // Otherwise we'll set the state right here and now, since this function should be
                // called from a GUI thread
                self.set_state_inner(&mut state);
                break;
            }
        }

        // After the state has been updated, notify the host about the new parameter values
        let task_posted = self.schedule_gui(Task::RescanParamValues);
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    pub fn set_latency_samples(&self, samples: u32) {
        // Only make a callback if it's actually needed
        // XXX: For CLAP we could move this handling to the Plugin struct, but it may be worthwhile
        //      to keep doing it this way to stay consistent with VST3.
        let old_latency = self.current_latency.swap(samples, Ordering::SeqCst);
        if old_latency != samples {
            let task_posted = self.schedule_gui(Task::LatencyChanged);
            nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
        }
    }

    pub fn set_current_voice_capacity(&self, capacity: u32) {
        match P::CLAP_POLY_MODULATION_CONFIG {
            Some(config) => {
                let clamped_capacity = capacity.clamp(1, config.max_voice_capacity);
                nih_debug_assert_eq!(
                    capacity,
                    clamped_capacity,
                    "The current voice capacity must be between 1 and the maximum capacity"
                );

                if clamped_capacity != self.current_voice_capacity.load(Ordering::Relaxed) {
                    self.current_voice_capacity
                        .store(clamped_capacity, Ordering::Relaxed);
                    let task_posted = self.schedule_gui(Task::VoiceInfoChanged);
                    nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
                }
            }
            None => nih_debug_assert_failure!(
                "Configuring the current voice capacity is only possible when \
                 'ClapPlugin::CLAP_POLY_MODULATION_CONFIG' is set"
            ),
        }
    }

    /// Immediately set the plugin state. Returns `false` if the deserialization failed. The plugin
    /// state is set from a couple places, so this function aims to deduplicate that. Includes
    /// `permit_alloc()`s around the deserialization and initialization for the use case where
    /// `set_state_object_from_gui()` was called while the plugin is process audio.
    ///
    /// Implicitly emits `Task::ParameterValuesChanged`.
    ///
    /// # Notes
    ///
    /// `self.plugin` must _not_ be locked while calling this function or it will deadlock.
    pub fn set_state_inner(&self, state: &mut PluginState) -> bool {
        let audio_io_layout = self.current_audio_io_layout.load();
        let buffer_config = self.current_buffer_config.load();

        // FIXME: This is obviously not realtime-safe, but loading presets without doing this could
        //        lead to inconsistencies. It's the plugin's responsibility to not perform any
        //        realtime-unsafe work when the initialize function is called a second time if it
        //        supports runtime preset loading.  `state::deserialize_object()` normally never
        //        allocates, but if the plugin has persistent non-parameter data then its
        //        `deserialize_fields()` implementation may still allocate.
        let mut success = permit_alloc(|| unsafe {
            state::deserialize_object::<P>(
                state,
                self.params.clone(),
                state::make_params_getter(&self.param_by_hash, &self.param_id_to_hash),
                self.current_buffer_config.load().as_ref(),
            )
        });
        if !success {
            nih_debug_assert_failure!("Deserializing plugin state from a state object failed");
            return false;
        }

        // If the plugin was already initialized then it needs to be reinitialized
        if let Some(buffer_config) = buffer_config {
            // NOTE: This needs to be dropped after the `plugin` lock to avoid deadlocks
            let mut init_context = self.make_init_context();
            let mut plugin = self.plugin.lock();

            // See above
            success = permit_alloc(|| {
                plugin.initialize(&audio_io_layout, &buffer_config, &mut init_context)
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
        if self.editor_handle.lock().is_some() {
            self.request_resize();
        }

        success
    }

    unsafe extern "C" fn init(plugin: *const clap_plugin) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        // We weren't allowed to query these in the constructor, so we need to do it now instead.
        *wrapper.host_gui.borrow_mut() =
            query_host_extension::<clap_host_gui>(&wrapper.host_callback, CLAP_EXT_GUI);
        *wrapper.host_latency.borrow_mut() =
            query_host_extension::<clap_host_latency>(&wrapper.host_callback, CLAP_EXT_LATENCY);
        *wrapper.host_params.borrow_mut() =
            query_host_extension::<clap_host_params>(&wrapper.host_callback, CLAP_EXT_PARAMS);
        *wrapper.host_voice_info.borrow_mut() = query_host_extension::<clap_host_voice_info>(
            &wrapper.host_callback,
            CLAP_EXT_VOICE_INFO,
        );
        *wrapper.host_thread_check.borrow_mut() = query_host_extension::<clap_host_thread_check>(
            &wrapper.host_callback,
            CLAP_EXT_THREAD_CHECK,
        );

        true
    }

    unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
        assert!(!plugin.is_null() && !(*plugin).plugin_data.is_null());
        let this = Arc::from_raw((*plugin).plugin_data as *mut Self);
        nih_debug_assert_eq!(Arc::strong_count(&this), 1);

        drop(this);
    }

    unsafe extern "C" fn activate(
        plugin: *const clap_plugin,
        sample_rate: f64,
        min_frames_count: u32,
        max_frames_count: u32,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let audio_io_layout = wrapper.current_audio_io_layout.load();
        let buffer_config = BufferConfig {
            sample_rate: sample_rate as f32,
            min_buffer_size: Some(min_frames_count),
            max_buffer_size: max_frames_count,
            process_mode: wrapper.current_process_mode.load(),
        };

        // Before initializing the plugin, make sure all smoothers are set the the default values
        for param in wrapper.param_by_hash.values() {
            param.update_smoother(buffer_config.sample_rate, true);
        }

        // NOTE: This needs to be dropped after the `plugin` lock to avoid deadlocks
        let mut init_context = wrapper.make_init_context();
        let mut plugin = wrapper.plugin.lock();
        if plugin.initialize(&audio_io_layout, &buffer_config, &mut init_context) {
            // NOTE: `Plugin::reset()` is called in `clap_plugin::start_processing()` instead of in
            //       this function

            // This preallocates enough space so we can transform all of the host's raw channel
            // pointers into a set of `Buffer` objects for the plugin's main and auxiliary IO
            *wrapper.buffer_manager.borrow_mut() =
                BufferManager::for_audio_io_layout(max_frames_count as usize, audio_io_layout);

            // Also store this for later, so we can reinitialize the plugin after restoring state
            wrapper.current_buffer_config.store(Some(buffer_config));

            true
        } else {
            false
        }
    }

    unsafe extern "C" fn deactivate(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        wrapper.plugin.lock().deactivate();
    }

    unsafe extern "C" fn start_processing(plugin: *const clap_plugin) -> bool {
        // We just need to keep track of our processing state so we can request a flush when
        // updating parameters from the GUI while the processing loop isn't running
        check_null_ptr!(false, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        // Always reset the processing status when the plugin gets activated or deactivated
        wrapper.last_process_status.store(ProcessStatus::Normal);
        wrapper.is_processing.store(true, Ordering::SeqCst);

        // To be consistent with the VST3 wrapper, we'll also reset the buffers here in addition to
        // the dedicated `reset()` function.
        process_wrapper(|| wrapper.plugin.lock().reset());

        true
    }

    unsafe extern "C" fn stop_processing(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        wrapper.is_processing.store(false, Ordering::SeqCst);
    }

    unsafe extern "C" fn reset(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        process_wrapper(|| wrapper.plugin.lock().reset());
    }

    unsafe extern "C" fn process(
        plugin: *const clap_plugin,
        process: *const clap_process,
    ) -> clap_process_status {
        check_null_ptr!(CLAP_PROCESS_ERROR, plugin, (*plugin).plugin_data, process);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        // Panic on allocations if the `assert_process_allocs` feature has been enabled, and make
        // sure that FTZ is set up correctly
        process_wrapper(|| {
            // We need to handle incoming automation and MIDI events. Since we don't support sample
            // accuration automation yet and there's no way to get the last event for a parameter,
            // we'll process every incoming event.
            let process = &*process;
            let total_buffer_len = process.frames_count as usize;

            let current_audio_io_layout = wrapper.current_audio_io_layout.load();
            let has_main_input = current_audio_io_layout.main_input_channels.is_some();
            let has_main_output = current_audio_io_layout.main_output_channels.is_some();
            let aux_input_start_idx = if has_main_input { 1 } else { 0 };
            let aux_output_start_idx = if has_main_output { 1 } else { 0 };

            // If `P::SAMPLE_ACCURATE_AUTOMATION` is set, then we'll split up the audio buffer into
            // chunks whenever a parameter change occurs
            let mut block_start = 0;
            let mut block_end = total_buffer_len;
            let mut event_start_idx = 0;

            // The host may send new transport information as an event. In that case we'll also
            // split the buffer.
            let mut transport_info = process.transport;

            let result = loop {
                if !process.in_events.is_null() {
                    let split_result = wrapper.handle_in_events_until(
                        &*process.in_events,
                        &mut transport_info,
                        block_start,
                        total_buffer_len,
                        event_start_idx,
                        |next_event| {
                            // Always split the buffer on transport information changes (tempo, time
                            // signature, or position changes), and also split on parameter value
                            // changes after the current sample if sample accurate automation is
                            // enabled
                            if P::SAMPLE_ACCURATE_AUTOMATION {
                                match ((*next_event).space_id, (*next_event).type_) {
                                    (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE)
                                    | (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_TRANSPORT) => true,
                                    (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_MOD) => {
                                        let next_event =
                                            &*(next_event as *const clap_event_param_mod);

                                        // The buffer should not be split on polyphonic modulation
                                        // as those events will be converted to note events
                                        !(next_event.note_id != -1
                                            && wrapper
                                                .poly_mod_ids_by_hash
                                                .contains_key(&next_event.param_id))
                                    }
                                    _ => false,
                                }
                            } else {
                                matches!(
                                    ((*next_event).space_id, (*next_event).type_,),
                                    (CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_TRANSPORT)
                                )
                            }
                        },
                    );

                    // If there are any parameter changes after `block_start` and sample
                    // accurate automation is enabled or the host sends new transport
                    // information, then we'll process a new block just after that. Otherwise we can
                    // process all audio until the end of the buffer.
                    match split_result {
                        Some((next_param_change_sample_idx, next_param_change_event_idx)) => {
                            block_end = next_param_change_sample_idx;
                            event_start_idx = next_param_change_event_idx;
                        }
                        None => block_end = total_buffer_len,
                    }
                }

                // After processing the events we now know where/if the block should be split, and
                // we can start preparing audio processing
                let block_len = block_end - block_start;

                // The buffer manager preallocated buffer slices for all the IO and storage for any
                // axuiliary inputs.
                // TODO: The audio buffers have a latency field, should we use those?
                // TODO: Like with VST3, should we expose some way to access or set the silence/constant
                //       flags?
                let mut buffer_manager = wrapper.buffer_manager.borrow_mut();
                let buffers =
                    buffer_manager.create_buffers(block_start, block_len, |buffer_source| {
                        // Explicitly take plugins with no main output that does have auxiliary
                        // outputs into account. Shouldn't happen, but if we just start copying
                        // audio here then that would result in unsoundness.
                        if process.audio_outputs_count > 0
                            && !process.audio_outputs.is_null()
                            && !(*process.audio_outputs).data32.is_null()
                            && has_main_output
                        {
                            let audio_output = &*process.audio_outputs;
                            let ptrs = NonNull::new(audio_output.data32 as *mut *mut f32).unwrap();
                            let num_channels = audio_output.channel_count as usize;

                            *buffer_source.main_output_channel_pointers =
                                Some(ChannelPointers { ptrs, num_channels });
                        }

                        if process.audio_inputs_count > 0
                            && !process.audio_inputs.is_null()
                            && !(*process.audio_inputs).data32.is_null()
                            && has_main_input
                        {
                            let audio_input = &*process.audio_inputs;
                            let ptrs = NonNull::new(audio_input.data32 as *mut *mut f32).unwrap();
                            let num_channels = audio_input.channel_count as usize;

                            *buffer_source.main_input_channel_pointers =
                                Some(ChannelPointers { ptrs, num_channels });
                        }

                        if !process.audio_inputs.is_null() {
                            for (aux_input_no, aux_input_channel_pointers) in buffer_source
                                .aux_input_channel_pointers
                                .iter_mut()
                                .enumerate()
                            {
                                let aux_input_idx = aux_input_no + aux_input_start_idx;
                                if aux_input_idx > process.audio_inputs_count as usize {
                                    break;
                                }

                                let audio_input = &*process.audio_inputs.add(aux_input_idx);
                                match NonNull::new(audio_input.data32 as *mut *mut f32) {
                                    Some(ptrs) => {
                                        let num_channels = audio_input.channel_count as usize;

                                        *aux_input_channel_pointers =
                                            Some(ChannelPointers { ptrs, num_channels });
                                    }
                                    None => continue,
                                }
                            }
                        }

                        if !process.audio_outputs.is_null() {
                            for (aux_output_no, aux_output_channel_pointers) in buffer_source
                                .aux_output_channel_pointers
                                .iter_mut()
                                .enumerate()
                            {
                                let aux_output_idx = aux_output_no + aux_output_start_idx;
                                if aux_output_idx > process.audio_outputs_count as usize {
                                    break;
                                }

                                let audio_output = &*process.audio_outputs.add(aux_output_idx);
                                match NonNull::new(audio_output.data32 as *mut *mut f32) {
                                    Some(ptrs) => {
                                        let num_channels = audio_output.channel_count as usize;

                                        *aux_output_channel_pointers =
                                            Some(ChannelPointers { ptrs, num_channels });
                                    }
                                    None => continue,
                                }
                            }
                        }
                    });

                // If the host does not provide outputs or if it does not provide the required
                // number of channels (should not happen, but Ableton Live does this for bypassed
                // VST3 plugins) then we'll skip audio processing. In that case
                // `buffer_manager.create_buffers` will have set one or more of the output buffers
                // to empty slices since there is no storage to point them to. The auxiliary input
                // buffers always point to valid storage.
                let mut buffer_is_valid = true;
                for output_buffer_slice in buffers.main_buffer.as_slice_immutable().iter().chain(
                    buffers
                        .aux_outputs
                        .iter()
                        .flat_map(|buffer| buffer.as_slice_immutable().iter()),
                ) {
                    if output_buffer_slice.is_empty() {
                        buffer_is_valid = false;
                        break;
                    }
                }

                nih_debug_assert!(buffer_is_valid);

                // Some of the fields are left empty because CLAP does not provide this information,
                // but the methods on [`Transport`] can reconstruct these values from the other
                // fields
                let sample_rate = wrapper
                    .current_buffer_config
                    .load()
                    .expect("Process call without prior initialization call")
                    .sample_rate;
                let mut transport = Transport::new(sample_rate);
                if !transport_info.is_null() {
                    let context = &*transport_info;

                    transport.playing = context.flags & CLAP_TRANSPORT_IS_PLAYING != 0;
                    transport.recording = context.flags & CLAP_TRANSPORT_IS_RECORDING != 0;
                    transport.preroll_active =
                        Some(context.flags & CLAP_TRANSPORT_IS_WITHIN_PRE_ROLL != 0);
                    if context.flags & CLAP_TRANSPORT_HAS_TEMPO != 0 {
                        transport.tempo = Some(context.tempo);
                    }
                    if context.flags & CLAP_TRANSPORT_HAS_TIME_SIGNATURE != 0 {
                        transport.time_sig_numerator = Some(context.tsig_num as i32);
                        transport.time_sig_denominator = Some(context.tsig_denom as i32);
                    }
                    if context.flags & CLAP_TRANSPORT_HAS_BEATS_TIMELINE != 0 {
                        let beats = context.song_pos_beats as f64 / CLAP_BEATTIME_FACTOR as f64;

                        // This is a bit messy, but we'll try to compensate for the block splitting.
                        // We can't use the functions on the transport information object for this
                        // because we don't have any sample information.
                        if P::SAMPLE_ACCURATE_AUTOMATION
                            && block_start > 0
                            && (context.flags & CLAP_TRANSPORT_HAS_TEMPO != 0)
                        {
                            transport.pos_beats = Some(
                                beats
                                    + (block_start as f64 / sample_rate as f64 / 60.0
                                        * context.tempo),
                            );
                        } else {
                            transport.pos_beats = Some(beats);
                        }
                    }
                    if context.flags & CLAP_TRANSPORT_HAS_SECONDS_TIMELINE != 0 {
                        let seconds = context.song_pos_seconds as f64 / CLAP_SECTIME_FACTOR as f64;

                        // Same here
                        if P::SAMPLE_ACCURATE_AUTOMATION
                            && block_start > 0
                            && (context.flags & CLAP_TRANSPORT_HAS_TEMPO != 0)
                        {
                            transport.pos_seconds =
                                Some(seconds + (block_start as f64 / sample_rate as f64));
                        } else {
                            transport.pos_seconds = Some(seconds);
                        }
                    }
                    // TODO: CLAP does not mention whether this is behind a flag or not
                    if P::SAMPLE_ACCURATE_AUTOMATION && block_start > 0 {
                        transport.bar_start_pos_beats = match transport.bar_start_pos_beats() {
                            Some(updated) => Some(updated),
                            None => Some(context.bar_start as f64 / CLAP_BEATTIME_FACTOR as f64),
                        };
                        transport.bar_number = match transport.bar_number() {
                            Some(updated) => Some(updated),
                            None => Some(context.bar_number),
                        };
                    } else {
                        transport.bar_start_pos_beats =
                            Some(context.bar_start as f64 / CLAP_BEATTIME_FACTOR as f64);
                        transport.bar_number = Some(context.bar_number);
                    }
                    // TODO: They also aren't very clear about this, but presumably if the loop is
                    //       active and the corresponding song transport information is available then
                    //       this is also available
                    if context.flags & CLAP_TRANSPORT_IS_LOOP_ACTIVE != 0
                        && context.flags & CLAP_TRANSPORT_HAS_BEATS_TIMELINE != 0
                    {
                        transport.loop_range_beats = Some((
                            context.loop_start_beats as f64 / CLAP_BEATTIME_FACTOR as f64,
                            context.loop_end_beats as f64 / CLAP_BEATTIME_FACTOR as f64,
                        ));
                    }
                    if context.flags & CLAP_TRANSPORT_IS_LOOP_ACTIVE != 0
                        && context.flags & CLAP_TRANSPORT_HAS_SECONDS_TIMELINE != 0
                    {
                        transport.loop_range_seconds = Some((
                            context.loop_start_seconds as f64 / CLAP_SECTIME_FACTOR as f64,
                            context.loop_end_seconds as f64 / CLAP_SECTIME_FACTOR as f64,
                        ));
                    }
                }

                let result = if buffer_is_valid {
                    let mut plugin = wrapper.plugin.lock();
                    // SAFETY: Shortening these borrows is safe as even if the plugin overwrites the
                    //         slices (which it cannot do without using unsafe code), then they
                    //         would still be reset on the next iteration
                    let mut aux = AuxiliaryBuffers {
                        inputs: buffers.aux_inputs,
                        outputs: buffers.aux_outputs,
                    };
                    let mut context = wrapper.make_process_context(transport);
                    let result = plugin.process(buffers.main_buffer, &mut aux, &mut context);
                    wrapper.last_process_status.store(result);
                    result
                } else {
                    ProcessStatus::Normal
                };

                let clap_result = match result {
                    ProcessStatus::Error(err) => {
                        nih_debug_assert_failure!("Process error: {}", err);

                        return CLAP_PROCESS_ERROR;
                    }
                    ProcessStatus::Normal => CLAP_PROCESS_CONTINUE_IF_NOT_QUIET,
                    ProcessStatus::Tail(_) => CLAP_PROCESS_CONTINUE,
                    ProcessStatus::KeepAlive => CLAP_PROCESS_CONTINUE,
                };

                // After processing audio, send all spooled events to the host. This include note
                // events.
                if !process.out_events.is_null() {
                    wrapper.handle_out_events(&*process.out_events, block_start, total_buffer_len);
                }

                // If our block ends at the end of the buffer then that means there are no more
                // unprocessed (parameter) events. If there are more events, we'll just keep going
                // through this process until we've processed the entire buffer.
                if block_end == total_buffer_len {
                    break clap_result;
                } else {
                    block_start = block_end;
                }
            };

            // After processing audio, we'll check if the editor has sent us updated plugin state.
            // We'll restore that here on the audio thread to prevent changing the values during the
            // process call and also to prevent inconsistent state when the host also wants to load
            // plugin state.
            // FIXME: Zero capacity channels allocate on receiving, find a better alternative that
            //        doesn't do that
            let updated_state = permit_alloc(|| wrapper.updated_state_receiver.try_recv());
            if let Ok(mut state) = updated_state {
                wrapper.set_state_inner(&mut state);

                // We'll pass the state object back to the GUI thread so deallocation can happen
                // there without potentially blocking the audio thread
                if let Err(err) = wrapper.updated_state_sender.send(state) {
                    nih_debug_assert_failure!(
                        "Failed to send state object back to GUI thread: {}",
                        err
                    );
                };
            }

            result
        })
    }

    unsafe extern "C" fn get_extension(
        plugin: *const clap_plugin,
        id: *const c_char,
    ) -> *const c_void {
        check_null_ptr!(std::ptr::null(), plugin, (*plugin).plugin_data, id);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let id = CStr::from_ptr(id);

        if id == CLAP_EXT_AUDIO_PORTS_CONFIG {
            &wrapper.clap_plugin_audio_ports_config as *const _ as *const c_void
        } else if id == CLAP_EXT_AUDIO_PORTS {
            &wrapper.clap_plugin_audio_ports as *const _ as *const c_void
        } else if id == CLAP_EXT_GUI && wrapper.editor.borrow().is_some() {
            // Only report that we support this extension if the plugin has an editor
            &wrapper.clap_plugin_gui as *const _ as *const c_void
        } else if id == CLAP_EXT_LATENCY {
            &wrapper.clap_plugin_latency as *const _ as *const c_void
        } else if id == CLAP_EXT_NOTE_PORTS
            && (P::MIDI_INPUT >= MidiConfig::Basic || P::MIDI_OUTPUT >= MidiConfig::Basic)
        {
            &wrapper.clap_plugin_note_ports as *const _ as *const c_void
        } else if id == CLAP_EXT_PARAMS {
            &wrapper.clap_plugin_params as *const _ as *const c_void
        } else if id == CLAP_EXT_REMOTE_CONTROLS {
            &wrapper.clap_plugin_remote_controls as *const _ as *const c_void
        } else if id == CLAP_EXT_RENDER {
            &wrapper.clap_plugin_render as *const _ as *const c_void
        } else if id == CLAP_EXT_STATE {
            &wrapper.clap_plugin_state as *const _ as *const c_void
        } else if id == CLAP_EXT_TAIL {
            &wrapper.clap_plugin_tail as *const _ as *const c_void
        } else if id == CLAP_EXT_VOICE_INFO && P::CLAP_POLY_MODULATION_CONFIG.is_some() {
            &wrapper.clap_plugin_voice_info as *const _ as *const c_void
        } else {
            nih_trace!("Host tried to query unknown extension {:?}", id);
            std::ptr::null()
        }
    }

    unsafe extern "C" fn on_main_thread(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        // [Self::schedule_gui] posts a task to the queue and asks the host to call this function
        // on the main thread, so once that's done we can just handle all requests here
        while let Some(task) = wrapper.tasks.pop() {
            wrapper.execute(task, true);
        }
    }

    unsafe extern "C" fn ext_audio_ports_config_count(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, (*plugin).plugin_data);

        P::AUDIO_IO_LAYOUTS.len() as u32
    }

    unsafe extern "C" fn ext_audio_ports_config_get(
        plugin: *const clap_plugin,
        index: u32,
        config: *mut clap_audio_ports_config,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, config);

        // This function directly maps to `P::AUDIO_IO_LAYOUTS`, and we thus also don't need to
        // access the `wrapper` instance
        match P::AUDIO_IO_LAYOUTS.get(index as usize) {
            Some(audio_io_layout) => {
                let name = audio_io_layout.name();

                let main_input_channels = audio_io_layout.main_input_channels.map(NonZeroU32::get);
                let main_output_channels =
                    audio_io_layout.main_output_channels.map(NonZeroU32::get);
                let input_port_type = match main_input_channels {
                    Some(1) => CLAP_PORT_MONO.as_ptr(),
                    Some(2) => CLAP_PORT_STEREO.as_ptr(),
                    _ => std::ptr::null(),
                };
                let output_port_type = match main_output_channels {
                    Some(1) => CLAP_PORT_MONO.as_ptr(),
                    Some(2) => CLAP_PORT_STEREO.as_ptr(),
                    _ => std::ptr::null(),
                };

                *config = std::mem::zeroed();

                let config = &mut *config;
                config.id = index;
                strlcpy(&mut config.name, &name);
                config.input_port_count = (if main_input_channels.is_some() { 1 } else { 0 }
                    + audio_io_layout.aux_input_ports.len())
                    as u32;
                config.output_port_count = (if main_output_channels.is_some() { 1 } else { 0 }
                    + audio_io_layout.aux_output_ports.len())
                    as u32;
                config.has_main_input = main_input_channels.is_some();
                config.main_input_channel_count = main_input_channels.unwrap_or_default();
                config.main_input_port_type = input_port_type;
                config.has_main_output = main_output_channels.is_some();
                config.main_output_channel_count = main_output_channels.unwrap_or_default();
                config.main_output_port_type = output_port_type;

                true
            }
            None => {
                nih_debug_assert_failure!(
                    "Host tried to query out of bounds audio port config {}",
                    index
                );

                false
            }
        }
    }

    unsafe extern "C" fn ext_audio_ports_config_select(
        plugin: *const clap_plugin,
        config_id: clap_id,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        // We use the vector indices for the config ID
        match P::AUDIO_IO_LAYOUTS.get(config_id as usize) {
            Some(audio_io_layout) => {
                wrapper.current_audio_io_layout.store(*audio_io_layout);

                true
            }
            None => {
                nih_debug_assert_failure!(
                    "Host tried to select out of bounds audio port config {}",
                    config_id
                );

                false
            }
        }
    }

    unsafe extern "C" fn ext_audio_ports_count(plugin: *const clap_plugin, is_input: bool) -> u32 {
        check_null_ptr!(0, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let audio_io_layout = wrapper.current_audio_io_layout.load();
        if is_input {
            let main_ports = if audio_io_layout.main_input_channels.is_some() {
                1
            } else {
                0
            };
            let aux_ports = audio_io_layout.aux_input_ports.len();

            (main_ports + aux_ports) as u32
        } else {
            let main_ports = if audio_io_layout.main_output_channels.is_some() {
                1
            } else {
                0
            };
            let aux_ports = audio_io_layout.aux_output_ports.len();

            (main_ports + aux_ports) as u32
        }
    }

    unsafe extern "C" fn ext_audio_ports_get(
        plugin: *const clap_plugin,
        index: u32,
        is_input: bool,
        info: *mut clap_audio_port_info,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, info);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let num_input_ports = Self::ext_audio_ports_count(plugin, true);
        let num_output_ports = Self::ext_audio_ports_count(plugin, false);
        if (is_input && index >= num_input_ports) || (!is_input && index >= num_output_ports) {
            nih_debug_assert_failure!(
                "Host tried to query information for out of bounds audio port {} (input: {})",
                index,
                is_input
            );

            return false;
        }

        let current_audio_io_layout = wrapper.current_audio_io_layout.load();
        let has_main_input = current_audio_io_layout.main_input_channels.is_some();
        let has_main_output = current_audio_io_layout.main_output_channels.is_some();

        // Whether this port is a main port or an auxiliary (sidechain) port
        let is_main_port =
            index == 0 && ((is_input && has_main_input) || (!is_input && has_main_output));

        // We'll number the ports in a linear order from `0..num_input_ports` and
        // `num_input_ports..(num_input_ports + num_output_ports)`
        let stable_id = if is_input {
            index
        } else {
            index + num_input_ports
        };
        let pair_stable_id = match (is_input, is_main_port) {
            // Ports are named linearly with inputs coming before outputs, so this is the index of
            // the first output port
            (true, true) if has_main_output => num_input_ports,
            (false, true) if has_main_input => 0,
            _ => CLAP_INVALID_ID,
        };

        let channel_count = match (index, is_input) {
            (0, true) if has_main_input => {
                current_audio_io_layout.main_input_channels.unwrap().get()
            }
            (0, false) if has_main_output => {
                current_audio_io_layout.main_output_channels.unwrap().get()
            }
            // `index` is off by one for the auxiliary ports if the plugin has a main port
            (n, true) if has_main_input => {
                current_audio_io_layout.aux_input_ports[n as usize - 1].get()
            }
            (n, false) if has_main_output => {
                current_audio_io_layout.aux_output_ports[n as usize - 1].get()
            }
            (n, true) => current_audio_io_layout.aux_input_ports[n as usize].get(),
            (n, false) => current_audio_io_layout.aux_output_ports[n as usize].get(),
        };

        let port_type = match channel_count {
            1 => CLAP_PORT_MONO.as_ptr(),
            2 => CLAP_PORT_STEREO.as_ptr(),
            _ => std::ptr::null(),
        };

        *info = std::mem::zeroed();

        let info = &mut *info;
        info.id = stable_id;
        match (is_input, is_main_port) {
            (true, true) => strlcpy(&mut info.name, &current_audio_io_layout.main_input_name()),
            (false, true) => strlcpy(&mut info.name, &current_audio_io_layout.main_output_name()),
            (true, false) => {
                let aux_input_idx = if has_main_input { index - 1 } else { index } as usize;
                strlcpy(
                    &mut info.name,
                    &current_audio_io_layout
                        .aux_input_name(aux_input_idx)
                        .expect("Out of bounds auxiliary input port"),
                );
            }
            (false, false) => {
                let aux_output_idx = if has_main_output { index - 1 } else { index } as usize;
                strlcpy(
                    &mut info.name,
                    &current_audio_io_layout
                        .aux_output_name(aux_output_idx)
                        .expect("Out of bounds auxiliary output port"),
                );
            }
        };
        info.flags = if is_main_port {
            CLAP_AUDIO_PORT_IS_MAIN
        } else {
            0
        };
        info.channel_count = channel_count;
        info.port_type = port_type;
        info.in_place_pair = pair_stable_id;

        true
    }

    unsafe extern "C" fn ext_gui_is_api_supported(
        _plugin: *const clap_plugin,
        api: *const c_char,
        is_floating: bool,
    ) -> bool {
        // We don't do standalone floating windows
        if is_floating {
            return false;
        }

        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        if CStr::from_ptr(api) == CLAP_WINDOW_API_X11 {
            return true;
        }
        #[cfg(target_os = "macos")]
        if CStr::from_ptr(api) == CLAP_WINDOW_API_COCOA {
            return true;
        }
        #[cfg(target_os = "windows")]
        if CStr::from_ptr(api) == CLAP_WINDOW_API_WIN32 {
            return true;
        }

        false
    }

    unsafe extern "C" fn ext_gui_get_preferred_api(
        _plugin: *const clap_plugin,
        api: *mut *const c_char,
        is_floating: *mut bool,
    ) -> bool {
        check_null_ptr!(false, api, is_floating);

        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        {
            *api = CLAP_WINDOW_API_X11.as_ptr();
        }
        #[cfg(target_os = "macos")]
        {
            *api = CLAP_WINDOW_API_COCOA.as_ptr();
        }
        #[cfg(target_os = "windows")]
        {
            *api = CLAP_WINDOW_API_WIN32.as_ptr();
        }

        // We don't do standalone floating windows yet
        *is_floating = false;

        true
    }

    unsafe extern "C" fn ext_gui_create(
        plugin: *const clap_plugin,
        api: *const c_char,
        is_floating: bool,
    ) -> bool {
        // Double check this in case the host didn't
        if !Self::ext_gui_is_api_supported(plugin, api, is_floating) {
            return false;
        }

        // In CLAP creating the editor window and embedding it in another window are separate, and
        // those things are one and the same in our framework. So we'll just pretend we did
        // something here.
        check_null_ptr!(false, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let editor_handle = wrapper.editor_handle.lock();
        if editor_handle.is_none() {
            true
        } else {
            nih_debug_assert_failure!("Tried creating editor while the editor was already active");
            false
        }
    }

    unsafe extern "C" fn ext_gui_destroy(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let mut editor_handle = wrapper.editor_handle.lock();
        if editor_handle.is_some() {
            *editor_handle = None;
        } else {
            nih_debug_assert_failure!("Tried destroying editor while the editor was not active");
        }
    }

    unsafe extern "C" fn ext_gui_set_scale(plugin: *const clap_plugin, scale: f64) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        // On macOS scaling is done by the OS, and all window sizes are in logical pixels
        if cfg!(target_os = "macos") {
            nih_debug_assert_failure!("Ignoring host request to set explicit DPI scaling factor");
            return false;
        }

        if wrapper
            .editor
            .borrow()
            .as_ref()
            .unwrap()
            .lock()
            .set_scale_factor(scale as f32)
        {
            wrapper
                .editor_scaling_factor
                .store(scale as f32, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    unsafe extern "C" fn ext_gui_get_size(
        plugin: *const clap_plugin,
        width: *mut u32,
        height: *mut u32,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, width, height);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        // For macOS the scaling factor is always 1
        let (unscaled_width, unscaled_height) =
            wrapper.editor.borrow().as_ref().unwrap().lock().size();
        let scaling_factor = wrapper.editor_scaling_factor.load(Ordering::Relaxed);
        (*width, *height) = (
            (unscaled_width as f32 * scaling_factor).round() as u32,
            (unscaled_height as f32 * scaling_factor).round() as u32,
        );

        true
    }

    unsafe extern "C" fn ext_gui_can_resize(_plugin: *const clap_plugin) -> bool {
        // TODO: Implement Host->Plugin GUI resizing
        false
    }

    unsafe extern "C" fn ext_gui_get_resize_hints(
        _plugin: *const clap_plugin,
        _hints: *mut clap_gui_resize_hints,
    ) -> bool {
        // TODO: Implement Host->Plugin GUI resizing
        false
    }

    unsafe extern "C" fn ext_gui_adjust_size(
        _plugin: *const clap_plugin,
        _width: *mut u32,
        _height: *mut u32,
    ) -> bool {
        // TODO: Implement Host->Plugin GUI resizing
        false
    }

    unsafe extern "C" fn ext_gui_set_size(
        plugin: *const clap_plugin,
        width: u32,
        height: u32,
    ) -> bool {
        // TODO: Implement Host->Plugin GUI resizing
        // TODO: The host will also call this if an asynchronous (on Linux) resize request fails
        check_null_ptr!(false, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let (unscaled_width, unscaled_height) =
            wrapper.editor.borrow().as_ref().unwrap().lock().size();
        let scaling_factor = wrapper.editor_scaling_factor.load(Ordering::Relaxed);
        let (editor_width, editor_height) = (
            (unscaled_width as f32 * scaling_factor).round() as u32,
            (unscaled_height as f32 * scaling_factor).round() as u32,
        );

        width == editor_width && height == editor_height
    }

    unsafe extern "C" fn ext_gui_set_parent(
        plugin: *const clap_plugin,
        window: *const clap_window,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, window);
        // For this function we need the underlying Arc so we can pass it to the editor
        let wrapper = Arc::from_raw((*plugin).plugin_data as *const Self);

        let window = &*window;

        let result = {
            let mut editor_handle = wrapper.editor_handle.lock();
            if editor_handle.is_none() {
                let api = CStr::from_ptr(window.api);
                let parent_handle = if api == CLAP_WINDOW_API_X11 {
                    ParentWindowHandle::X11Window(window.specific.x11 as u32)
                } else if api == CLAP_WINDOW_API_COCOA {
                    ParentWindowHandle::AppKitNsView(window.specific.cocoa)
                } else if api == CLAP_WINDOW_API_WIN32 {
                    ParentWindowHandle::Win32Hwnd(window.specific.win32)
                } else {
                    nih_debug_assert_failure!("Host passed an invalid API");
                    return false;
                };

                // This extension is only exposed when we have an editor
                *editor_handle = Some(
                    wrapper
                        .editor
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .lock()
                        .spawn(parent_handle, wrapper.clone().make_gui_context()),
                );

                true
            } else {
                nih_debug_assert_failure!(
                    "Host tried to attach editor while the editor is already attached"
                );

                false
            }
        };

        // Leak the Arc again since we only needed a clone to pass to the GuiContext
        let _ = Arc::into_raw(wrapper);

        result
    }

    unsafe extern "C" fn ext_gui_set_transient(
        _plugin: *const clap_plugin,
        _window: *const clap_window,
    ) -> bool {
        // This is only relevant for floating windows
        false
    }

    unsafe extern "C" fn ext_gui_suggest_title(_plugin: *const clap_plugin, _title: *const c_char) {
        // This is only relevant for floating windows
    }

    unsafe extern "C" fn ext_gui_show(_plugin: *const clap_plugin) -> bool {
        // TODO: Does this get used? Is this only for the free-standing window extension? (which we
        //       don't implement) This wouldn't make any sense for embedded editors.
        false
    }

    unsafe extern "C" fn ext_gui_hide(_plugin: *const clap_plugin) -> bool {
        // TODO: Same as the above
        false
    }

    unsafe extern "C" fn ext_latency_get(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        wrapper.current_latency.load(Ordering::SeqCst)
    }

    unsafe extern "C" fn ext_note_ports_count(_plugin: *const clap_plugin, is_input: bool) -> u32 {
        match is_input {
            true if P::MIDI_INPUT >= MidiConfig::Basic => 1,
            false if P::MIDI_OUTPUT >= MidiConfig::Basic => 1,
            _ => 0,
        }
    }

    unsafe extern "C" fn ext_note_ports_get(
        _plugin: *const clap_plugin,
        index: u32,
        is_input: bool,
        info: *mut clap_note_port_info,
    ) -> bool {
        match (index, is_input) {
            (0, true) if P::MIDI_INPUT >= MidiConfig::Basic => {
                *info = std::mem::zeroed();

                let info = &mut *info;
                info.id = 0;
                // NOTE: REAPER won't send us SysEx if we don't support the MIDI dialect
                // TODO: Implement MPE (would just be a toggle for the plugin to expose it) and MIDI2
                info.supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
                info.preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
                strlcpy(&mut info.name, "Note Input");

                true
            }
            (0, false) if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                *info = std::mem::zeroed();

                let info = &mut *info;
                info.id = 0;
                // If `P::MIDI_OUTPUT < MidiConfig::MidiCCs` we'll throw away MIDI CCs, pitch bend
                // messages, and other messages that are not basic note on, off and polyphonic
                // pressure messages. This way the behavior is the same as the VST3 wrapper.
                info.supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
                info.preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
                strlcpy(&mut info.name, "Note Output");

                true
            }
            _ => false,
        }
    }

    unsafe extern "C" fn ext_params_count(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        wrapper.param_hashes.len() as u32
    }

    unsafe extern "C" fn ext_params_get_info(
        plugin: *const clap_plugin,
        param_index: u32,
        param_info: *mut clap_param_info,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, param_info);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        if param_index > Self::ext_params_count(plugin) {
            return false;
        }

        let param_hash = &wrapper.param_hashes[param_index as usize];
        let param_group = &wrapper.param_group_by_hash[param_hash];
        let param_ptr = &wrapper.param_by_hash[param_hash];
        let default_value = param_ptr.default_normalized_value();
        let step_count = param_ptr.step_count();
        let flags = param_ptr.flags();
        let automatable = !flags.contains(ParamFlags::NON_AUTOMATABLE);
        let hidden = flags.contains(ParamFlags::HIDDEN);
        let is_bypass = flags.contains(ParamFlags::BYPASS);

        *param_info = std::mem::zeroed();

        // TODO: We don't use the cookies at this point. In theory this would be faster than the ID
        //       hashmap lookup, but for now we'll stay consistent with the VST3 implementation.
        let param_info = &mut *param_info;
        param_info.id = *param_hash;
        // TODO: Somehow expose per note/channel/port modulation
        param_info.flags = 0;
        if automatable && !hidden {
            param_info.flags |= CLAP_PARAM_IS_AUTOMATABLE | CLAP_PARAM_IS_MODULATABLE;
            if wrapper.poly_mod_ids_by_hash.contains_key(param_hash) {
                param_info.flags |= CLAP_PARAM_IS_MODULATABLE_PER_NOTE_ID;
            }
        }
        if hidden {
            param_info.flags |= CLAP_PARAM_IS_HIDDEN | CLAP_PARAM_IS_READONLY;
        }
        if is_bypass {
            param_info.flags |= CLAP_PARAM_IS_BYPASS
        }
        if step_count.is_some() {
            param_info.flags |= CLAP_PARAM_IS_STEPPED
        }
        param_info.cookie = std::ptr::null_mut();
        strlcpy(&mut param_info.name, param_ptr.name());
        strlcpy(&mut param_info.module, param_group);
        // We don't use the actual minimum and maximum values here because that would not scale
        // with skewed integer ranges. Instead, just treat all parameters as `[0, 1]` normalized
        // parameters multiplied by the step size.
        param_info.min_value = 0.0;
        // Stepped parameters are unnormalized float parameters since there's no separate step
        // range option
        // TODO: This should probably be encapsulated in some way so we don't forget about this in one place
        param_info.max_value = step_count.unwrap_or(1) as f64;
        param_info.default_value = default_value as f64 * step_count.unwrap_or(1) as f64;

        true
    }

    unsafe extern "C" fn ext_params_get_value(
        plugin: *const clap_plugin,
        param_id: clap_id,
        value: *mut f64,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, value);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        match wrapper.param_by_hash.get(&param_id) {
            Some(param_ptr) => {
                *value = param_ptr.modulated_normalized_value() as f64
                    * param_ptr.step_count().unwrap_or(1) as f64;

                true
            }
            _ => false,
        }
    }

    unsafe extern "C" fn ext_params_value_to_text(
        plugin: *const clap_plugin,
        param_id: clap_id,
        value: f64,
        display: *mut c_char,
        size: u32,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, display);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let dest = std::slice::from_raw_parts_mut(display, size as usize);

        match wrapper.param_by_hash.get(&param_id) {
            Some(param_ptr) => {
                strlcpy(
                    dest,
                    // CLAP does not have a separate unit, so we'll include the unit here
                    &param_ptr.normalized_value_to_string(
                        value as f32 / param_ptr.step_count().unwrap_or(1) as f32,
                        true,
                    ),
                );

                true
            }
            _ => false,
        }
    }

    unsafe extern "C" fn ext_params_text_to_value(
        plugin: *const clap_plugin,
        param_id: clap_id,
        display: *const c_char,
        value: *mut f64,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, display, value);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let display = match CStr::from_ptr(display).to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };

        match wrapper.param_by_hash.get(&param_id) {
            Some(param_ptr) => {
                let normalized_value = match param_ptr.string_to_normalized_value(display) {
                    Some(v) => v as f64,
                    None => return false,
                };
                *value = normalized_value * param_ptr.step_count().unwrap_or(1) as f64;

                true
            }
            _ => false,
        }
    }

    unsafe extern "C" fn ext_params_flush(
        plugin: *const clap_plugin,
        in_: *const clap_input_events,
        out: *const clap_output_events,
    ) {
        check_null_ptr!((), plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        if !in_.is_null() {
            wrapper.handle_in_events(&*in_, 0, 0);
        }

        if !out.is_null() {
            wrapper.handle_out_events(&*out, 0, 0);
        }
    }

    unsafe extern "C" fn ext_remote_controls_count(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        wrapper.remote_control_pages.len() as u32
    }

    unsafe extern "C" fn ext_remote_controls_get(
        plugin: *const clap_plugin,
        page_index: u32,
        page: *mut clap_remote_controls_page,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, page);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        nih_debug_assert!(page_index as usize <= wrapper.remote_control_pages.len());
        match wrapper.remote_control_pages.get(page_index as usize) {
            Some(p) => {
                *page = *p;
                true
            }
            None => false,
        }
    }

    unsafe extern "C" fn ext_render_has_hard_realtime_requirement(
        _plugin: *const clap_plugin,
    ) -> bool {
        P::HARD_REALTIME_ONLY
    }

    unsafe extern "C" fn ext_render_set(
        plugin: *const clap_plugin,
        mode: clap_plugin_render_mode,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let mode = match mode {
            CLAP_RENDER_REALTIME => ProcessMode::Realtime,
            // Even if the plugin has a hard realtime requirement, we'll still honor this
            CLAP_RENDER_OFFLINE => ProcessMode::Offline,
            n => {
                nih_debug_assert_failure!("Unknown rendering mode '{}', defaulting to realtime", n);
                ProcessMode::Realtime
            }
        };
        wrapper.current_process_mode.store(mode);

        true
    }

    unsafe extern "C" fn ext_state_save(
        plugin: *const clap_plugin,
        stream: *const clap_ostream,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, stream);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        let serialized = state::serialize_json::<P>(
            wrapper.params.clone(),
            state::make_params_iter(&wrapper.param_by_hash, &wrapper.param_id_to_hash),
        );
        match serialized {
            Ok(serialized) => {
                // CLAP does not provide a way to tell how much data there is left in a stream, so
                // we need to prepend it to our actual state data.
                let length_bytes = (serialized.len() as u64).to_le_bytes();
                if !write_stream(&*stream, &length_bytes) {
                    nih_debug_assert_failure!(
                        "Error or end of stream while writing the state length to the stream."
                    );
                    return false;
                }
                if !write_stream(&*stream, &serialized) {
                    nih_debug_assert_failure!(
                        "Error or end of stream while writing the state buffer to the stream."
                    );
                    return false;
                }

                nih_trace!("Saved state ({} bytes)", serialized.len());

                true
            }
            Err(err) => {
                nih_debug_assert_failure!("Could not save state: {:#}", err);
                false
            }
        }
    }

    unsafe extern "C" fn ext_state_load(
        plugin: *const clap_plugin,
        stream: *const clap_istream,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, stream);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        // CLAP does not have a way to tell how much data there is left in a stream, so we've
        // prepended the size in front of our JSON state
        let mut length_bytes = [0u8; 8];
        if !read_stream(&*stream, length_bytes.as_mut_slice()) {
            nih_debug_assert_failure!(
                "Error or end of stream while reading the state length from the stream."
            );
            return false;
        }
        let length = u64::from_le_bytes(length_bytes);

        let mut read_buffer: Vec<u8> = Vec::with_capacity(length as usize);
        if !read_stream(&*stream, read_buffer.spare_capacity_mut()) {
            nih_debug_assert_failure!(
                "Error or end of stream while reading the state buffer from the stream."
            );
            return false;
        }
        read_buffer.set_len(length as usize);

        match state::deserialize_json(&read_buffer) {
            Some(mut state) => {
                let success = wrapper.set_state_inner(&mut state);
                if success {
                    nih_trace!("Loaded state ({} bytes)", read_buffer.len());
                }

                success
            }
            None => false,
        }
    }

    unsafe extern "C" fn ext_tail_get(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, (*plugin).plugin_data);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        match wrapper.last_process_status.load() {
            ProcessStatus::Tail(samples) => samples,
            ProcessStatus::KeepAlive => u32::MAX,
            _ => 0,
        }
    }

    unsafe extern "C" fn ext_voice_info_get(
        plugin: *const clap_plugin,
        info: *mut clap_voice_info,
    ) -> bool {
        check_null_ptr!(false, plugin, (*plugin).plugin_data, info);
        let wrapper = &*((*plugin).plugin_data as *const Self);

        match P::CLAP_POLY_MODULATION_CONFIG {
            Some(config) => {
                *info = clap_voice_info {
                    voice_count: wrapper.current_voice_capacity.load(Ordering::Relaxed),
                    voice_capacity: config.max_voice_capacity,
                    flags: if config.supports_overlapping_voices {
                        CLAP_VOICE_INFO_SUPPORTS_OVERLAPPING_NOTES
                    } else {
                        0
                    },
                };

                true
            }
            None => false,
        }
    }
}

/// Convenience function to query an extension from the host.
///
/// # Safety
///
/// The extension type `T` must match the extension's name `name`.
unsafe fn query_host_extension<T>(
    host_callback: &ClapPtr<clap_host>,
    name: &CStr,
) -> Option<ClapPtr<T>> {
    let extension_ptr =
        clap_call! { host_callback=>get_extension(&**host_callback, name.as_ptr()) };
    if !extension_ptr.is_null() {
        Some(ClapPtr::new(extension_ptr as *const T))
    } else {
        None
    }
}
