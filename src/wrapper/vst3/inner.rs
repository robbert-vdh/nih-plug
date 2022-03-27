use atomic_refcell::AtomicRefCell;
use crossbeam::atomic::AtomicCell;
use parking_lot::RwLock;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use vst3_sys::base::{kInvalidArgument, kResultOk, tresult};
use vst3_sys::vst::IComponentHandler;

use super::context::{WrapperGuiContext, WrapperProcessContext};
use super::param_units::ParamUnits;
use super::util::{ObjectPtr, VstPtr};
use super::view::WrapperView;
use crate::buffer::Buffer;
use crate::context::Transport;
use crate::event_loop::{EventLoop, MainThreadExecutor, OsEventLoop};
use crate::param::internals::ParamPtr;
use crate::param::ParamFlags;
use crate::plugin::{BufferConfig, BusConfig, Editor, NoteEvent, ProcessStatus, Vst3Plugin};
use crate::wrapper::util::{hash_param_id, Bypass, BYPASS_PARAM_HASH, BYPASS_PARAM_ID};

/// The actual wrapper bits. We need this as an `Arc<T>` so we can safely use our event loop API.
/// Since we can't combine that with VST3's interior reference counting this just has to be moved to
/// its own struct.
pub(crate) struct WrapperInner<P: Vst3Plugin> {
    /// The wrapped plugin instance.
    pub plugin: RwLock<P>,
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
    /// GUI thread.
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
    /// Contains either a boolean indicating whether the plugin is currently bypassed, or a bypass
    /// parameter if the plugin has one.
    pub bypass: Bypass,
    /// The last process status returned by the plugin. This is used for tail handling.
    pub last_process_status: AtomicCell<ProcessStatus>,
    /// The current latency in samples, as set by the plugin through the [`ProcessContext`].
    pub current_latency: AtomicU32,
    /// Contains slices for the plugin's outputs. You can't directly create a nested slice form
    /// apointer to pointers, so this needs to be preallocated in the setup call and kept around
    /// between process calls. This buffer owns the vector, because otherwise it would need to store
    /// a mutable reference to the data contained in this mutex.
    pub output_buffer: AtomicRefCell<Buffer<'static>>,
    /// The incoming events for the plugin, if `P::ACCEPTS_MIDI` is set. If
    /// `P::SAMPLE_ACCURATE_AUTOMATION`, this is also read in lockstep with the parameter change
    /// block splitting.
    pub input_events: AtomicRefCell<VecDeque<NoteEvent>>,
    /// Unprocessed parameter changes sent by the host as pairs of `(sample_idx_in_buffer, change)`.
    /// Needed because VST3 does not have a single queue containing all parameter changes. If
    /// `P::SAMPLE_ACCURATE_AUTOMATION` is set, then all parameter changes will be read into this
    /// priority queue and the buffer will be processed in small chunks whenever there's a parameter
    /// change at a new sample index.
    pub input_param_changes: AtomicRefCell<BinaryHeap<Reverse<(usize, ParameterChange)>>>,

    /// The keys from `param_map` in a stable order.
    pub param_hashes: Vec<u32>,
    /// A mapping from parameter ID hashes (obtained from the string parameter IDs) to pointers to
    /// parameters belonging to the plugin. As long as `plugin` does not get recreated, these
    /// addresses will remain stable, as they are obtained from a pinned object.
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
}

/// An incoming parameter change sent by the host. Kept in a queue to support block-based sample
/// accurate automation.
#[derive(Debug, PartialEq, PartialOrd)]
pub struct ParameterChange {
    /// The parameter's hash, as used everywhere else.
    pub hash: u32,
    /// The normalized values, as provided by the host.
    pub normalized_value: f32,
}

// Instances needed for the binary heap, we'll just pray the host doesn't send NaN values
impl Eq for ParameterChange {}

#[allow(clippy::derive_ord_xor_partial_ord)]
impl Ord for ParameterChange {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl<P: Vst3Plugin> WrapperInner<P> {
    #[allow(unused_unsafe)]
    pub fn new() -> Arc<Self> {
        let plugin = RwLock::new(P::default());
        let editor = plugin.read().editor().map(Arc::from);

        // This is a mapping from the parameter IDs specified by the plugin to pointers to thsoe
        // parameters. These pointers are assumed to be safe to dereference as long as
        // `wrapper.plugin` is alive. The plugin API identifiers these parameters by hashes, which
        // we'll calculate from the string ID specified by the plugin. These parameters should also
        // remain in the same order as the one returned by the plugin.
        let param_id_hashes_ptrs_groups: Vec<_> = plugin
            .read()
            .params()
            .param_map()
            .into_iter()
            .map(|(id, ptr, group)| {
                let hash = hash_param_id(&id);
                (id, hash, ptr, group)
            })
            .collect();
        if cfg!(debug_assertions) {
            let param_map = plugin.read().params().param_map();
            let param_ids: HashSet<_> = param_id_hashes_ptrs_groups
                .iter()
                .map(|(id, _, _, _)| id.clone())
                .collect();
            nih_debug_assert_eq!(
                param_map.len(),
                param_ids.len(),
                "The plugin has duplicate parameter IDs, weird things may happen"
            );
        }

        // The plugin either supplies its own bypass parameter, or we'll create one for it
        let mut bypass = Bypass::Dummy(AtomicBool::new(false));
        for (id, _, ptr, _) in &param_id_hashes_ptrs_groups {
            let flags = unsafe { ptr.flags() };
            let is_bypass = flags.contains(ParamFlags::BYPASS);
            if cfg!(debug_assertions) {
                if id == BYPASS_PARAM_ID && !is_bypass {
                    nih_debug_assert_failure!("Bypass parameters need to be marked with `.make_bypass()`, weird things will happen");
                }
                if is_bypass && matches!(bypass, Bypass::Parameter(_)) {
                    nih_debug_assert_failure!(
                        "Duplicate bypass parameters found, using the first one"
                    );
                    continue;
                }
            }

            if is_bypass {
                bypass = Bypass::Parameter(*ptr);
                if !cfg!(debug_assertions) {
                    break;
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
            plugin,
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
                num_input_channels: P::DEFAULT_NUM_INPUTS,
                num_output_channels: P::DEFAULT_NUM_OUTPUTS,
            }),
            current_buffer_config: AtomicCell::new(None),
            bypass,
            last_process_status: AtomicCell::new(ProcessStatus::Normal),
            current_latency: AtomicU32::new(0),
            output_buffer: AtomicRefCell::new(Buffer::default()),
            input_events: AtomicRefCell::new(VecDeque::with_capacity(1024)),
            input_param_changes: AtomicRefCell::new(BinaryHeap::with_capacity(
                if P::SAMPLE_ACCURATE_AUTOMATION {
                    4096
                } else {
                    0
                },
            )),

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

    pub fn make_process_context(&self, transport: Transport) -> WrapperProcessContext<'_, P> {
        WrapperProcessContext {
            inner: self,
            input_events_guard: self.input_events.borrow_mut(),
            transport,
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
        match (&self.bypass, self.param_by_hash.get(&hash)) {
            (Bypass::Dummy(bypass_state), _) if hash == *BYPASS_PARAM_HASH => {
                bypass_state.store(normalized_value >= 0.5, Ordering::Relaxed);

                kResultOk
            }
            (_, Some(param_ptr)) => {
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
        }
    }
}
