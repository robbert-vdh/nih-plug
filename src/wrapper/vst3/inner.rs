use crossbeam::atomic::AtomicCell;
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use vst3_sys::base::{kInvalidArgument, kResultOk, tresult};
use vst3_sys::vst::IComponentHandler;

use super::context::WrapperProcessContext;
use super::util::{VstPtr, BYPASS_PARAM_HASH, BYPASS_PARAM_ID};
use crate::buffer::Buffer;
use crate::context::{EventLoop, GuiContext, MainThreadExecutor, OsEventLoop};
use crate::param::internals::ParamPtr;
use crate::plugin::{BufferConfig, BusConfig, Editor, NoteEvent, Plugin, ProcessStatus};
use crate::wrapper::util::hash_param_id;

/// The actual wrapper bits. We need this as an `Arc<T>` so we can safely use our event loop API.
/// Since we can't combine that with VST3's interior reference counting this just has to be moved to
/// its own struct.
pub(crate) struct WrapperInner<P: Plugin> {
    /// The wrapped plugin instance.
    pub plugin: RwLock<P>,
    /// The plugin's editor, if it has one. This object does not do anything on its own, but we need
    /// to instantiate this in advance so we don't need to lock the entire [Plugin] object when
    /// creating an editor.
    pub editor: Option<Arc<dyn Editor>>,

    /// The host's `IComponentHandler` instance, if passed through
    /// `IEditController::set_component_handler`.
    pub component_handler: RwLock<Option<VstPtr<dyn IComponentHandler>>>,

    /// A realtime-safe task queue so the plugin can schedule tasks that need to be run later on the
    /// GUI thread.
    ///
    /// This RwLock is only needed because it has to be initialized late. There is no reason to
    /// mutably borrow the event loop, so reads will never be contested.
    ///
    /// TODO: Is there a better type for Send+Sync late initializaiton?
    pub event_loop: RwLock<MaybeUninit<OsEventLoop<Task, Self>>>,

    /// Whether the plugin is currently processing audio. In other words, the last state
    /// `IAudioProcessor::setActive()` has been called with.
    pub is_processing: AtomicBool,
    /// The current bus configuration, modified through `IAudioProcessor::setBusArrangements()`.
    pub current_bus_config: AtomicCell<BusConfig>,
    /// The current buffer configuration, containing the sample rate and the maximum block size.
    /// Will be set in `IAudioProcessor::setupProcessing()`.
    pub current_buffer_config: AtomicCell<Option<BufferConfig>>,
    /// Whether the plugin is currently bypassed. This is not yet integrated with the `Plugin`
    /// trait.
    pub bypass_state: AtomicBool,
    /// The last process status returned by the plugin. This is used for tail handling.
    pub last_process_status: AtomicCell<ProcessStatus>,
    /// The current latency in samples, as set by the plugin through the [ProcessContext].
    pub current_latency: AtomicU32,
    /// Contains slices for the plugin's outputs. You can't directly create a nested slice form
    /// apointer to pointers, so this needs to be preallocated in the setup call and kept around
    /// between process calls. This buffer owns the vector, because otherwise it would need to store
    /// a mutable reference to the data contained in this mutex.
    pub output_buffer: RwLock<Buffer<'static>>,
    /// The incoming events for the plugin, if `P::ACCEPTS_MIDI` is set.
    ///
    /// TODO: Maybe load these lazily at some point instead of needing to spool them all to this
    ///       queue first
    pub input_events: RwLock<VecDeque<NoteEvent>>,

    /// The keys from `param_map` in a stable order.
    pub param_hashes: Vec<u32>,
    /// A mapping from parameter ID hashes (obtained from the string parameter IDs) to pointers to
    /// parameters belonging to the plugin. As long as `plugin` does not get recreated, these
    /// addresses will remain stable, as they are obtained from a pinned object.
    pub param_by_hash: HashMap<u32, ParamPtr>,
    /// The default normalized parameter value for every parameter in `param_ids`. We need to store
    /// this in case the host requeries the parmaeter later. This is also indexed by the hash so we
    /// can retrieve them later for the UI if needed.
    pub param_defaults_normalized: HashMap<u32, f32>,
    /// Mappings from string parameter indentifiers to parameter hashes. Useful for debug logging
    /// and when storing and restorign plugin state.
    pub param_id_to_hash: HashMap<&'static str, u32>,
    /// The inverse mapping from [Self::param_by_hash]. This is needed to be able to have an
    /// ergonomic parameter setting API that uses references to the parameters instead of having to
    /// add a setter function to the parameter (or even worse, have it be completely untyped).
    pub param_ptr_to_hash: HashMap<ParamPtr, u32>,
}

/// Tasks that can be sent from the plugin to be executed on the main thread in a non-blocking
/// realtime safe way (either a random thread or `IRunLoop` on Linux, the OS' message loop on
/// Windows and macOS).
#[derive(Debug, Clone)]
pub enum Task {
    /// Trigger a restart with the given restart flags. This is a bit set of the flags from
    /// [vst3_sys::vst::RestartFlags].
    TriggerRestart(i32),
}

impl<P: Plugin> WrapperInner<P> {
    #[allow(unused_unsafe)]
    pub fn new() -> Arc<Self> {
        let plugin = RwLock::new(P::default());
        let editor = plugin.read().editor().map(Arc::from);

        let mut wrapper = Self {
            plugin,
            editor,

            component_handler: RwLock::new(None),

            event_loop: RwLock::new(MaybeUninit::uninit()),

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
            bypass_state: AtomicBool::new(false),
            last_process_status: AtomicCell::new(ProcessStatus::Normal),
            current_latency: AtomicU32::new(0),
            output_buffer: RwLock::new(Buffer::default()),
            input_events: RwLock::new(VecDeque::with_capacity(512)),

            param_hashes: Vec::new(),
            param_by_hash: HashMap::new(),
            param_defaults_normalized: HashMap::new(),
            param_id_to_hash: HashMap::new(),
            param_ptr_to_hash: HashMap::new(),
        };

        // This is a mapping from the parameter IDs specified by the plugin to pointers to thsoe
        // parameters. Since the object returned by `params()` is pinned, these pointers are safe to
        // dereference as long as `wrapper.plugin` is alive
        let param_map = wrapper.plugin.read().params().param_map();
        let param_ids = wrapper.plugin.read().params().param_ids();
        nih_debug_assert!(
            !param_map.contains_key(BYPASS_PARAM_ID),
            "The wrapper alread yadds its own bypass parameter"
        );

        // Only calculate these hashes once, and in the stable order defined by the plugin
        let param_id_hashes_ptrs: Vec<_> = param_ids
            .iter()
            .filter_map(|id| {
                let param_ptr = param_map.get(id)?;
                Some((id, hash_param_id(id), param_ptr))
            })
            .collect();
        wrapper.param_hashes = param_id_hashes_ptrs
            .iter()
            .map(|&(_, hash, _)| hash)
            .collect();
        wrapper.param_by_hash = param_id_hashes_ptrs
            .iter()
            .map(|&(_, hash, ptr)| (hash, *ptr))
            .collect();
        wrapper.param_defaults_normalized = param_id_hashes_ptrs
            .iter()
            .map(|&(_, hash, ptr)| (hash, unsafe { ptr.normalized_value() }))
            .collect();
        wrapper.param_id_to_hash = param_id_hashes_ptrs
            .iter()
            .map(|&(id, hash, _)| (*id, hash))
            .collect();
        wrapper.param_ptr_to_hash = param_id_hashes_ptrs
            .into_iter()
            .map(|(_, hash, ptr)| (*ptr, hash))
            .collect();

        // FIXME: Right now this is safe, but if we are going to have a singleton main thread queue
        //        serving multiple plugin instances, Arc can't be used because its reference count
        //        is separate from the internal COM-style reference count.
        let wrapper: Arc<WrapperInner<P>> = wrapper.into();
        *wrapper.event_loop.write() =
            MaybeUninit::new(OsEventLoop::new_and_spawn(Arc::downgrade(&wrapper)));

        wrapper
    }

    pub fn make_process_context(&self) -> WrapperProcessContext<'_, P> {
        WrapperProcessContext {
            inner: self,
            input_events_guard: self.input_events.write(),
        }
    }

    /// Convenience function for setting a value for a parameter as triggered by a VST3 parameter
    /// update. The same rate is for updating parameter smoothing.
    pub unsafe fn set_normalized_value_by_hash(
        &self,
        hash: u32,
        normalized_value: f32,
        sample_rate: Option<f32>,
    ) -> tresult {
        if hash == *BYPASS_PARAM_HASH {
            self.bypass_state
                .store(normalized_value >= 0.5, Ordering::SeqCst);

            kResultOk
        } else if let Some(param_ptr) = self.param_by_hash.get(&hash) {
            // Also update the parameter's smoothing if applicable
            match (param_ptr, sample_rate) {
                (_, Some(sample_rate)) => {
                    param_ptr.set_normalized_value(normalized_value);
                    param_ptr.update_smoother(sample_rate, false);
                }
                _ => param_ptr.set_normalized_value(normalized_value),
            }

            kResultOk
        } else {
            kInvalidArgument
        }
    }
}

// We can't use a nice standalone context object for this as we don't have any control over the
// lifetime since we currently try to stay GUI framework agnostic. Because of that, the only
// alternative is to pass an `Arc<Self as GuiContext>` to the plugin and hope it doesn't do anything
// weird with it.
impl<P: Plugin> GuiContext for WrapperInner<P> {
    // All of these functions are supposed to be called from the main thread, so we'll put some
    // trust in the caller and assume that this is indeed the case
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        match &*self.component_handler.read() {
            Some(handler) => match self.param_ptr_to_hash.get(&param) {
                Some(hash) => {
                    handler.begin_edit(*hash);
                }
                None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
            },
            None => nih_debug_assert_failure!("Component handler not yet set"),
        }
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        match &*self.component_handler.read() {
            Some(handler) => match self.param_ptr_to_hash.get(&param) {
                Some(hash) => {
                    // Only update the parameters manually if the host is not processing audio. If
                    // the plugin is currently processing audio, the host will pass this change back
                    // to the plugin in the audio callback. This also prevents the values from
                    // changing in the middle of the process callback, which would be unsound.
                    if !self.is_processing.load(Ordering::SeqCst) {
                        self.set_normalized_value_by_hash(
                            *hash,
                            normalized,
                            self.current_buffer_config.load().map(|c| c.sample_rate),
                        );
                    }

                    handler.perform_edit(*hash, normalized as f64);
                }
                None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
            },
            None => nih_debug_assert_failure!("Component handler not yet set"),
        }
    }

    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        match &*self.component_handler.read() {
            Some(handler) => match self.param_ptr_to_hash.get(&param) {
                Some(hash) => {
                    handler.end_edit(*hash);
                }
                None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
            },
            None => nih_debug_assert_failure!("Component handler not yet set"),
        }
    }

    unsafe fn raw_default_normalized_param_value(&self, param: ParamPtr) -> f32 {
        match self.param_ptr_to_hash.get(&param) {
            Some(hash) => self.param_defaults_normalized[hash],
            None => {
                nih_debug_assert_failure!("Unknown parameter: {:?}", param);
                0.5
            }
        }
    }
}

impl<P: Plugin> MainThreadExecutor<Task> for WrapperInner<P> {
    unsafe fn execute(&self, task: Task) {
        // This function is always called from the main thread
        // TODO: When we add GUI resizing and context menus, this should propagate those events to
        //       `IRunLoop` on Linux to keep REAPER happy. That does mean a double spool, but we can
        //       come up with a nicer solution to handle that later (can always add a separate
        //       function for checking if a to be scheduled task can be handled right ther and
        //       then).
        match task {
            Task::TriggerRestart(flags) => match &*self.component_handler.read() {
                Some(handler) => {
                    handler.restart_component(flags);
                }
                None => nih_debug_assert_failure!("Component handler not yet set"),
            },
        }
    }
}
