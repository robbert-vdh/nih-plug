use atomic_refcell::AtomicRefCell;
use crossbeam::atomic::AtomicCell;
use crossbeam::channel::{self, SendTimeoutError};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use vst3_sys::base::{kInvalidArgument, kResultOk, tresult};
use vst3_sys::vst::{IComponentHandler, RestartFlags};

use super::context::{WrapperGuiContext, WrapperInitContext, WrapperProcessContext};
use super::note_expressions::NoteExpressionController;
use super::param_units::ParamUnits;
use super::util::{ObjectPtr, VstPtr, VST3_MIDI_PARAMS_END, VST3_MIDI_PARAMS_START};
use super::view::WrapperView;
use crate::buffer::Buffer;
use crate::context::Transport;
use crate::event_loop::{EventLoop, MainThreadExecutor, OsEventLoop};
use crate::midi::{MidiConfig, NoteEvent};
use crate::param::internals::{ParamPtr, Params};
use crate::param::ParamFlags;
use crate::plugin::{BufferConfig, BusConfig, Editor, ProcessMode, ProcessStatus, Vst3Plugin};
use crate::wrapper::state::{self, PluginState};
use crate::wrapper::util::{hash_param_id, process_wrapper};

/// The actual wrapper bits. We need this as an `Arc<T>` so we can safely use our event loop API.
/// Since we can't combine that with VST3's interior reference counting this just has to be moved to
/// its own struct.
pub(crate) struct WrapperInner<P: Vst3Plugin> {
    /// The wrapped plugin instance.
    pub plugin: RwLock<P>,
    /// The plugin's parameters. These are fetched once during initialization. That way the
    /// `ParamPtr`s are guaranteed to live at least as long as this object and we can interact with
    /// the `Params` object without having to acquire a lock on `plugin`.
    pub params: Arc<dyn Params>,
    /// The plugin's editor, if it has one. This object does not do anything on its own, but we need
    /// to instantiate this in advance so we don't need to lock the entire [`Plugin`] object when
    /// creating an editor.
    pub editor: Option<Arc<dyn Editor>>,

    /// The host's [`IComponentHandler`] instance, if passed through
    /// [`IEditController::set_component_handler`].
    pub component_handler: AtomicRefCell<Option<VstPtr<dyn IComponentHandler>>>,

    /// Our own [`IPlugView`] instance. This is set while the editor is actually visible (which is
    /// different form the lifetime of [`WrapperView`][super::WrapperView] itself).
    pub plug_view: RwLock<Option<ObjectPtr<WrapperView<P>>>>,

    /// A realtime-safe task queue so the plugin can schedule tasks that need to be run later on the
    /// GUI thread. This field should not be used directly for posting tasks. This should be done
    /// through [`Self::do_maybe_async()`] instead. That method posts the task to the host's
    /// `IRunLoop` instead of it's available.
    ///
    /// This RwLock is only needed because it has to be initialized late. There is no reason to
    /// mutably borrow the event loop, so reads will never be contested.
    ///
    /// TODO: Is there a better type for Send+Sync late initializaiton?
    pub event_loop: AtomicRefCell<MaybeUninit<OsEventLoop<Task, Self>>>,

    /// Whether the plugin is currently processing audio. In other words, the last state
    /// `IAudioProcessor::setActive()` has been called with.
    pub is_processing: AtomicBool,
    /// The current bus configuration, modified through `IAudioProcessor::setBusArrangements()`.
    pub current_bus_config: AtomicCell<BusConfig>,
    /// The current buffer configuration, containing the sample rate and the maximum block size.
    /// Will be set in `IAudioProcessor::setupProcessing()`.
    pub current_buffer_config: AtomicCell<Option<BufferConfig>>,
    /// The current audio processing mode. Set in `IAudioProcessor::setup_processing()`.
    pub current_process_mode: AtomicCell<ProcessMode>,
    /// The last process status returned by the plugin. This is used for tail handling.
    pub last_process_status: AtomicCell<ProcessStatus>,
    /// The current latency in samples, as set by the plugin through the [`ProcessContext`].
    pub current_latency: AtomicU32,
    /// Contains slices for the plugin's outputs. You can't directly create a nested slice form
    /// apointer to pointers, so this needs to be preallocated in the setup call and kept around
    /// between process calls. This buffer owns the vector, because otherwise it would need to store
    /// a mutable reference to the data contained in this mutex.
    pub output_buffer: AtomicRefCell<Buffer<'static>>,
    /// Stores sample data for every sidechain input the plugin has. Indexed by
    /// `[sidechain_input][channel][sample]` We'll copy the data to these buffers since modifying
    /// the host's sidechain input buffers may not be safe, and the plugin may want to be able to
    /// modify the buffers.
    pub aux_input_storage: AtomicRefCell<Vec<Vec<Vec<f32>>>>,
    /// Accompanying buffers for `aux_input_storage`. There is no way to do this in safe Rust, so
    /// the process function needs to make sure all channel pointers stored in these buffers are
    /// still correct before passing it to the plugin, hence the static lifetime.
    pub aux_input_buffers: AtomicRefCell<Vec<Buffer<'static>>>,
    /// Buffers for auxiliary plugin outputs, if the plugin has any. These reference the host's
    /// memory directly.
    pub aux_output_buffers: AtomicRefCell<Vec<Buffer<'static>>>,
    /// The incoming events for the plugin, if `P::ACCEPTS_MIDI` is set. If
    /// `P::SAMPLE_ACCURATE_AUTOMATION`, this is also read in lockstep with the parameter change
    /// block splitting.
    ///
    /// NOTE: Because with VST3 MIDI CC messages are sent as parameter changes and VST3 does not
    ///       interleave parameter changes and note events, this queue has to be sorted when
    ///       creating the process context
    pub input_events: AtomicRefCell<VecDeque<NoteEvent>>,
    /// Stores any events the plugin has output during the current processing cycle, analogous to
    /// `input_events`.
    pub output_events: AtomicRefCell<VecDeque<NoteEvent>>,
    /// VST3 has several useful predefined note expressions, but for some reason they are the only
    /// note event type that don't have MIDI note ID and channel fields. So we need to keep track of
    /// the msot recent VST3 note IDs we've seen, and then map those back to MIDI note IDs and
    /// channels as needed.
    pub note_expression_controller: AtomicRefCell<NoteExpressionController>,
    /// Unprocessed parameter changes and note events sent by the host during a process call.
    /// Parameter changes are sent as separate queues for each parameter, and note events are in
    /// another queue on top of that. And if `P::MIDI_INPUT >= MidiConfig::MidiCCs`, then we can
    /// also receive MIDI CC messages through special parameter changes. On top of that, we also
    /// support sample accurate automation through block splitting if
    /// `P::SAMPLE_ACCURATE_AUTOMATION` is set. To account for all of this, we'll read all of the
    /// parameter changes and events into a vector at the start of the process call, sort it, and
    /// then do the block splitting based on that. Note events need to have their timing adjusted to
    /// match the block start, since they're all read upfront.
    pub process_events: AtomicRefCell<Vec<ProcessEvent>>,
    /// The plugin is able to restore state through a method on the `GuiContext`. To avoid changing
    /// parameters mid-processing and running into garbled data if the host also tries to load state
    /// at the same time the restoring happens at the end of each processing call. If this zero
    /// capacity channel contains state data at that point, then the audio thread will take the
    /// state out of the channel, restore the state, and then send it back through the same channel.
    /// In other words, the GUI thread acts as a sender and then as a receiver, while the audio
    /// thread acts as a receiver and then as a sender. That way deallocation can happen on the GUI
    /// thread. All of this happens without any blocking on the audio thread.
    pub updated_state_sender: channel::Sender<PluginState>,
    /// The receiver belonging to [`new_state_sender`][Self::new_state_sender].
    pub updated_state_receiver: channel::Receiver<PluginState>,

    /// The keys from `param_map` in a stable order.
    pub param_hashes: Vec<u32>,
    /// A mapping from parameter ID hashes (obtained from the string parameter IDs) to pointers to
    /// parameters belonging to the plugin. These addresses will remain stable as long as the
    /// `params` object does not get deallocated.
    pub param_by_hash: HashMap<u32, ParamPtr>,
    pub param_units: ParamUnits,
    /// Mappings from string parameter indentifiers to parameter hashes. Useful for debug logging
    /// and when storing and restorign plugin state.
    pub param_id_to_hash: HashMap<String, u32>,
    /// The inverse mapping from [`param_by_hash`][Self::param_by_hash]. This is needed to be able
    /// to have an ergonomic parameter setting API that uses references to the parameters instead of
    /// having to add a setter function to the parameter (or even worse, have it be completely
    /// untyped).
    pub param_ptr_to_hash: HashMap<ParamPtr, u32>,
}

/// Tasks that can be sent from the plugin to be executed on the main thread in a non-blocking
/// realtime safe way (either a random thread or `IRunLoop` on Linux, the OS' message loop on
/// Windows and macOS).
#[derive(Debug, Clone)]
pub enum Task {
    /// Trigger a restart with the given restart flags. This is a bit set of the flags from
    /// [`vst3_sys::vst::RestartFlags`].
    TriggerRestart(i32),
    /// Request the editor to be resized according to its current size. Right now there is no way to
    /// handle denied resize requestsyet.
    RequestResize,
}

/// VST3 makes audio processing pretty complicated. In order to support both block splitting for
/// sample accurate automation and MIDI CC handling through parameters we need to put all parameter
/// changes and (translated) note events into a sorted array first.
#[derive(Debug, PartialEq)]
pub enum ProcessEvent {
    /// An incoming parameter change sent by the host. This will only be used when sample accurate
    /// automation has been enabled, and the parameters are only updated when we process this
    /// spooled event at the start of a block.
    ParameterChange {
        /// The event's sample offset within the buffer. Used for sorting.
        timing: u32,
        /// The parameter's hash, as used everywhere else.
        hash: u32,
        /// The normalized values, as provided by the host.
        normalized_value: f32,
    },
    /// An incoming parameter change sent by the host. This will only be used when sample accurate
    /// automation has been enabled, and the parameters are only updated when we process this
    /// spooled event at the start of a block.
    NoteEvent {
        /// The event's sample offset within the buffer. Used for sorting. The timing stored within
        /// the note event needs to have the block start index subtraced from it.
        timing: u32,
        /// The actual note event, make sure to subtract the block start index with
        /// [`NoteEvent::subtract_timing()`] before putting this into the input event queue.
        event: NoteEvent,
    },
}

impl<P: Vst3Plugin> WrapperInner<P> {
    #[allow(unused_unsafe)]
    pub fn new() -> Arc<Self> {
        let plugin = P::default();
        let editor = plugin.editor().map(Arc::from);

        // This is used to allow the plugin to restore preset data from its editor, see the comment
        // on `Self::updated_state_sender`
        let (updated_state_sender, updated_state_receiver) = channel::bounded(0);

        // This is a mapping from the parameter IDs specified by the plugin to pointers to thsoe
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
                 6 character parameter IDs to avoid collissions.."
            );

            let mut bypass_param_exists = false;
            for (id, hash, ptr, _) in &param_id_hashes_ptrs_groups {
                let flags = unsafe { ptr.flags() };
                let is_bypass = flags.contains(ParamFlags::BYPASS);

                if is_bypass && bypass_param_exists {
                    nih_debug_assert_failure!(
                        "Duplicate bypass parameters found, the host will only use the first one"
                    );
                }

                bypass_param_exists |= is_bypass;

                if P::MIDI_INPUT >= MidiConfig::MidiCCs
                    && (VST3_MIDI_PARAMS_START..VST3_MIDI_PARAMS_END).contains(hash)
                {
                    nih_debug_assert_failure!(
                        "Parameter '{}' collides with an automatically generated MIDI CC \
                         parameter, consider giving it a different ID",
                        id
                    );
                }
            }
        }

        let param_hashes = param_id_hashes_ptrs_groups
            .iter()
            .map(|(_, hash, _, _)| *hash)
            .collect();
        let param_by_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(_, hash, ptr, _)| (*hash, *ptr))
            .collect();
        let param_units = ParamUnits::from_param_groups(
            param_id_hashes_ptrs_groups
                .iter()
                .map(|(_, hash, _, group_name)| (*hash, group_name.as_str())),
        )
        .expect("Inconsistent parameter groups");
        let param_id_to_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(id, hash, _, _)| (id.clone(), *hash))
            .collect();
        let param_ptr_to_hash = param_id_hashes_ptrs_groups
            .into_iter()
            .map(|(_, hash, ptr, _)| (ptr, hash))
            .collect();

        let wrapper = Self {
            plugin: RwLock::new(plugin),
            params,
            editor,

            component_handler: AtomicRefCell::new(None),

            plug_view: RwLock::new(None),

            event_loop: AtomicRefCell::new(MaybeUninit::uninit()),

            is_processing: AtomicBool::new(false),
            // Some hosts, like the current version of Bitwig and Ardour at the time of writing,
            // will try using the plugin's default not yet initialized bus arrangement. Because of
            // that, we'll always initialize this configuration even before the host requests a
            // channel layout.
            current_bus_config: AtomicCell::new(BusConfig {
                num_input_channels: P::DEFAULT_INPUT_CHANNELS,
                num_output_channels: P::DEFAULT_OUTPUT_CHANNELS,
                aux_input_busses: P::DEFAULT_AUX_INPUTS.unwrap_or_default(),
                aux_output_busses: P::DEFAULT_AUX_OUTPUTS.unwrap_or_default(),
            }),
            current_buffer_config: AtomicCell::new(None),
            current_process_mode: AtomicCell::new(ProcessMode::Realtime),
            last_process_status: AtomicCell::new(ProcessStatus::Normal),
            current_latency: AtomicU32::new(0),
            output_buffer: AtomicRefCell::new(Buffer::default()),
            aux_input_storage: AtomicRefCell::new(Vec::new()),
            aux_input_buffers: AtomicRefCell::new(Vec::new()),
            aux_output_buffers: AtomicRefCell::new(Vec::new()),
            input_events: AtomicRefCell::new(VecDeque::with_capacity(1024)),
            output_events: AtomicRefCell::new(VecDeque::with_capacity(1024)),
            note_expression_controller: AtomicRefCell::new(NoteExpressionController::default()),
            process_events: AtomicRefCell::new(Vec::with_capacity(4096)),
            updated_state_sender,
            updated_state_receiver,

            param_hashes,
            param_by_hash,
            param_units,
            param_id_to_hash,
            param_ptr_to_hash,
        };

        // FIXME: Right now this is safe, but if we are going to have a singleton main thread queue
        //        serving multiple plugin instances, Arc can't be used because its reference count
        //        is separate from the internal COM-style reference count.
        let wrapper: Arc<WrapperInner<P>> = wrapper.into();
        *wrapper.event_loop.borrow_mut() =
            MaybeUninit::new(OsEventLoop::new_and_spawn(Arc::downgrade(&wrapper)));

        wrapper
    }

    pub fn make_gui_context(self: Arc<Self>) -> Arc<WrapperGuiContext<P>> {
        Arc::new(WrapperGuiContext { inner: self })
    }

    pub fn make_init_context(&self) -> WrapperInitContext<'_, P> {
        WrapperInitContext { inner: self }
    }

    pub fn make_process_context(&self, transport: Transport) -> WrapperProcessContext<'_, P> {
        WrapperProcessContext {
            inner: self,
            input_events_guard: self.input_events.borrow_mut(),
            output_events_guard: self.output_events.borrow_mut(),
            transport,
        }
    }

    /// Either posts the function to the task queue using [`EventLoop::do_maybe_async()`] so it can
    /// be delegated to the main thread, executes the task directly if this is the main thread, or
    /// runs the task on the host's `IRunLoop` if the GUI is open and it exposes one. This function
    ///
    /// If the task queue is full, then this will return false.
    #[must_use]
    pub fn do_maybe_async(&self, task: Task) -> bool {
        let event_loop = self.event_loop.borrow();
        let event_loop = unsafe { event_loop.assume_init_ref() };
        if event_loop.is_main_thread() {
            unsafe { self.execute(task) };
            true
        } else {
            // If the editor is open, and the host exposes the `IRunLoop` interface, then we'll run
            // the task on the host's GUI thread using that interface. Otherwise we'll use the
            // regular eent loop. If the editor gets dropped while there's still outstanding work
            // left in the run loop task queue, then those tasks will be posted to the regular event
            // loop so no work is lost.
            match &*self.plug_view.read() {
                Some(plug_view) => match plug_view.do_maybe_in_run_loop(task) {
                    Ok(()) => true,
                    Err(task) => event_loop.do_maybe_async(task),
                },
                None => event_loop.do_maybe_async(task),
            }
        }
    }

    /// If there's an editor open, let it know that parameter values have changed. This should be
    /// called whenever there's been a call or multiple calls to
    /// [`set_normalized_value_by_hash()[Self::set_normalized_value_by_hash()`].
    pub fn notify_param_values_changed(&self) {
        if let Some(editor) = &self.editor {
            editor.param_values_changed();
        }
    }

    /// Convenience function for setting a value for a parameter as triggered by a VST3 parameter
    /// update. The same rate is for updating parameter smoothing.
    ///
    /// After calling this function, you should call
    /// [`notify_param_values_changed()`][Self::notify_param_values_changed()] to allow the editor
    /// to update itself. This needs to be done seperately so you can process parameter changes in
    /// batches.
    pub fn set_normalized_value_by_hash(
        &self,
        hash: u32,
        normalized_value: f32,
        sample_rate: Option<f32>,
    ) -> tresult {
        match self.param_by_hash.get(&hash) {
            Some(param_ptr) => {
                // Also update the parameter's smoothing if applicable
                match (param_ptr, sample_rate) {
                    (_, Some(sample_rate)) => unsafe {
                        param_ptr.set_normalized_value(normalized_value);
                        param_ptr.update_smoother(sample_rate, false);
                    },
                    _ => unsafe { param_ptr.set_normalized_value(normalized_value) },
                }

                kResultOk
            }
            _ => kInvalidArgument,
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
    pub fn set_state_object(&self, mut state: PluginState) {
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
                unsafe {
                    state::deserialize_object(
                        &state,
                        self.params.clone(),
                        state::make_params_getter(&self.param_by_hash, &self.param_id_to_hash),
                        self.current_buffer_config.load().as_ref(),
                    );
                }

                self.notify_param_values_changed();
                let bus_config = self.current_bus_config.load();
                if let Some(buffer_config) = self.current_buffer_config.load() {
                    let mut plugin = self.plugin.write();
                    plugin.initialize(&bus_config, &buffer_config, &mut self.make_init_context());
                    process_wrapper(|| plugin.reset());
                }

                break;
            }
        }

        // After the state has been updated, notify the host about the new parameter values
        let task_posted = unsafe { self.event_loop.borrow().assume_init_ref() }.do_maybe_async(
            Task::TriggerRestart(RestartFlags::kParamValuesChanged as i32),
        );
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    pub fn set_latency_samples(&self, samples: u32) {
        // Only trigger a restart if it's actually needed
        let old_latency = self.current_latency.swap(samples, Ordering::SeqCst);
        if old_latency != samples {
            let task_posted =
                self.do_maybe_async(Task::TriggerRestart(RestartFlags::kLatencyChanged as i32));
            nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
        }
    }
}

impl<P: Vst3Plugin> MainThreadExecutor<Task> for WrapperInner<P> {
    unsafe fn execute(&self, task: Task) {
        // This function is always called from the main thread
        // TODO: When we add GUI resizing and context menus, this should propagate those events to
        //       `IRunLoop` on Linux to keep REAPER happy. That does mean a double spool, but we can
        //       come up with a nicer solution to handle that later (can always add a separate
        //       function for checking if a to be scheduled task can be handled right ther and
        //       then).
        match task {
            Task::TriggerRestart(flags) => match &*self.component_handler.borrow() {
                Some(handler) => {
                    handler.restart_component(flags);
                }
                None => nih_debug_assert_failure!("Component handler not yet set"),
            },
            Task::RequestResize => match &*self.plug_view.read() {
                Some(plug_view) => {
                    plug_view.request_resize();
                }
                None => nih_debug_assert_failure!("Can't resize a closed editor"),
            },
        }
    }
}
