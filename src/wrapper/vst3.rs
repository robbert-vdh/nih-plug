// nih-plug: plugins, but rewritten in Rust
// Copyright (C) 2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

// The VST3 macro generates an `allocate()` function for initializing the struct, so Clippy will
// complain as soon as a struct has more than 8 fields
#![allow(clippy::too_many_arguments)]

use crossbeam::atomic::AtomicCell;
use lazy_static::lazy_static;
use parking_lot::{RwLock, RwLockWriteGuard};
use std::cmp;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::{self, MaybeUninit};
use std::pin::Pin;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use vst3_sys::base::{kInvalidArgument, kNoInterface, kResultFalse, kResultOk, tresult, TBool};
use vst3_sys::base::{IBStream, IPluginBase, IPluginFactory, IPluginFactory2, IPluginFactory3};
use vst3_sys::utils::SharedVstPtr;
use vst3_sys::vst::{
    IAudioProcessor, IComponent, IComponentHandler, IEditController, IEventList, IParamValueQueue,
    IParameterChanges, TChar,
};
use vst3_sys::VST3;
use widestring::U16CStr;

use crate::buffer::Buffer;
use crate::context::{EventLoop, MainThreadExecutor, OsEventLoop, ProcessContext};
use crate::param::internals::ParamPtr;
use crate::param::range::Range;
use crate::param::Param;
use crate::plugin::{BufferConfig, BusConfig, NoteEvent, Plugin, ProcessStatus, Vst3Plugin};
use crate::wrapper::state::{ParamValue, State};
use crate::wrapper::util::{hash_param_id, process_wrapper, strlcpy, u16strlcpy};

// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;

/// Re-export for the wrapper.
pub use vst3_sys::sys::GUID;

/// The VST3 SDK version this is roughtly based on.
const VST3_SDK_VERSION: &str = "VST 3.6.14";
/// Right now the wrapper adds its own bypass parameter.
///
/// TODO: Actually use this parameter.
const BYPASS_PARAM_ID: &str = "bypass";
lazy_static! {
    static ref BYPASS_PARAM_HASH: u32 = hash_param_id(BYPASS_PARAM_ID);
}

/// Early exit out of a VST3 function when one of the passed pointers is null
macro_rules! check_null_ptr {
    ($ptr:expr $(, $ptrs:expr)* $(, )?) => {
        check_null_ptr_msg!("Null pointer passed to function", $ptr $(, $ptrs)*)
    };
}

/// The same as [check_null_ptr!], but with a custom message.
macro_rules! check_null_ptr_msg {
    ($msg:expr, $ptr:expr $(, $ptrs:expr)* $(, )?) => {
        if $ptr.is_null() $(|| $ptrs.is_null())* {
            nih_debug_assert_failure!($msg);
            return kInvalidArgument;
        }
    };
}

/// The actual wrapper bits. We need this as an `Arc<T>` so we can safely use our event loop API.
/// Since we can't combine that with VST3's interior reference counting this just has to be moved to
/// its own struct.
struct WrapperInner<P: Plugin> {
    /// The wrapped plugin instance.
    plugin: Pin<Box<RwLock<P>>>,

    /// The host's `IComponentHandler` instance, if passed through
    /// `IEditController::set_component_handler`.
    component_handler: RwLock<Option<VstPtr<dyn IComponentHandler>>>,

    /// A realtime-safe task queue so the plugin can schedule tasks that need to be run later on the
    /// GUI thread.
    ///
    /// This RwLock is only needed because it has to be initialized late. There is no reason to
    /// mutably borrow the event loop, so reads will never be contested.
    ///
    /// TODO: Is there a better type for Send+Sync late initializaiton?
    event_loop: RwLock<MaybeUninit<OsEventLoop<Task, Self>>>,

    /// Whether the plugin is currently processing audio. In other words, the last state
    /// `IAudioProcessor::setActive()` has been called with.
    is_processing: AtomicBool,
    /// The current bus configuration, modified through `IAudioProcessor::setBusArrangements()`.
    current_bus_config: AtomicCell<BusConfig>,
    /// The current buffer configuration, containing the sample rate and the maximum block size.
    /// Will be set in `IAudioProcessor::setupProcessing()`.
    current_buffer_config: AtomicCell<Option<BufferConfig>>,
    /// Whether the plugin is currently bypassed. This is not yet integrated with the `Plugin`
    /// trait.
    bypass_state: AtomicBool,
    /// The last process status returned by the plugin. This is used for tail handling.
    last_process_status: AtomicCell<ProcessStatus>,
    /// The current latency in samples, as set by the plugin through the [ProcessContext].
    current_latency: AtomicU32,
    /// Contains slices for the plugin's outputs. You can't directly create a nested slice form
    /// apointer to pointers, so this needs to be preallocated in the setup call and kept around
    /// between process calls. This buffer owns the vector, because otherwise it would need to store
    /// a mutable reference to the data contained in this mutex.
    output_buffer: RwLock<Buffer<'static>>,
    /// The incoming events for the plugin, if `P::ACCEPTS_MIDI` is set.
    ///
    /// TODO: Maybe load these lazily at some point instead of needing to spool them all to this
    ///       queue first
    input_events: RwLock<VecDeque<NoteEvent>>,

    /// The keys from `param_map` in a stable order.
    param_hashes: Vec<u32>,
    /// A mapping from parameter ID hashes (obtained from the string parameter IDs) to pointers to
    /// parameters belonging to the plugin. As long as `plugin` does not get recreated, these
    /// addresses will remain stable, as they are obtained from a pinned object.
    param_by_hash: HashMap<u32, ParamPtr>,
    /// The default normalized parameter value for every parameter in `param_ids`. We need to store
    /// this in case the host requeries the parmaeter later.
    param_defaults_normalized: Vec<f32>,
    /// Mappings from string parameter indentifiers to parameter hashes. Useful for debug logging
    /// and when storing and restorign plugin state.
    param_id_to_hash: HashMap<&'static str, u32>,
}

#[VST3(implements(IComponent, IEditController, IAudioProcessor))]
pub(crate) struct Wrapper<P: Plugin> {
    inner: Arc<WrapperInner<P>>,
}

/// A [ProcessContext] implementation for the wrapper. This is a separate object so it can hold on
/// to lock guards for event queues. Otherwise reading these events would require constant
/// unnecessary atomic operations to lock the uncontested RwLocks.
struct WrapperProcessContext<'a, P: Plugin> {
    inner: &'a WrapperInner<P>,
    input_events_guard: RwLockWriteGuard<'a, VecDeque<NoteEvent>>,
}

impl<P: Plugin> ProcessContext for WrapperProcessContext<'_, P> {
    fn set_latency_samples(&self, samples: u32) {
        // Only trigger a restart if it's actually needed
        let old_latency = self.inner.current_latency.swap(samples, Ordering::SeqCst);
        if old_latency != samples {
            let task_posted = unsafe { self.inner.event_loop.read().assume_init_ref() }
                .do_maybe_async(Task::TriggerRestart(
                    vst3_sys::vst::RestartFlags::kLatencyChanged as i32,
                ));
            nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
        }
    }

    fn next_midi_event(&mut self) -> Option<NoteEvent> {
        self.input_events_guard.pop_front()
    }
}

/// Tasks that can be sent from the plugin to be executed on the main thread in a non-blocking
/// realtime safe way (either a random thread or `IRunLoop` on Linux, the OS' message loop on
/// Windows and macOS).
#[derive(Debug, Clone)]
enum Task {
    /// Trigger a restart with the given restart flags. This is a bit set of the flags from
    /// [vst3_sys::vst::RestartFlags].
    TriggerRestart(i32),
}

/// Send+Sync wrapper for these interface pointers.
#[repr(transparent)]
struct VstPtr<T: vst3_sys::ComInterface + ?Sized> {
    ptr: vst3_sys::VstPtr<T>,
}

impl<T: vst3_sys::ComInterface + ?Sized> std::ops::Deref for VstPtr<T> {
    type Target = vst3_sys::VstPtr<T>;

    fn deref(&self) -> &Self::Target {
        &self.ptr
    }
}

impl<T: vst3_sys::ComInterface + ?Sized> From<vst3_sys::VstPtr<T>> for VstPtr<T> {
    fn from(ptr: vst3_sys::VstPtr<T>) -> Self {
        Self { ptr }
    }
}

/// SAFETY: Sharing these pointers across thread is s safe as they have internal atomic reference
/// counting, so as long as a `VstPtr<T>` handle exists the object will stay alive.
unsafe impl<T: vst3_sys::ComInterface + ?Sized> Send for VstPtr<T> {}
unsafe impl<T: vst3_sys::ComInterface + ?Sized> Sync for VstPtr<T> {}

impl<P: Plugin> WrapperInner<P> {
    // XXX: The unsafe blocks in this function are unnecessary. but rust-analyzer gets a bit
    //      confused by all of these vtables
    #[allow(unused_unsafe)]
    pub fn new() -> Arc<Self> {
        let mut wrapper = Self {
            plugin: Box::pin(RwLock::default()),

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
            param_defaults_normalized: Vec::new(),
            param_id_to_hash: HashMap::new(),
        };

        // This is a mapping from the parameter IDs specified by the plugin to pointers to thsoe
        // parameters. Since the object returned by `params()` is pinned, these pointers are safe to
        // dereference as long as `wrapper.plugin` is alive
        // XXX: This unsafe block is unnecessary. rust-analyzer gets a bit confused and this this
        //      `read()` function is from `IBStream` which it definitely is not.
        let param_map = unsafe { wrapper.plugin.read() }.params().param_map();
        let param_ids = unsafe { wrapper.plugin.read() }.params().param_ids();
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
            .map(|&(_, _, ptr)| unsafe { ptr.normalized_value() })
            .collect();
        wrapper.param_id_to_hash = param_id_hashes_ptrs
            .into_iter()
            .map(|(id, hash, _)| (*id, hash))
            .collect();

        // FIXME: Right now this is safe, but if we are going to have a singleton main thread queue
        //        serving multiple plugin instances, Arc can't be used because its reference count
        //        is separate from the internal COM-style reference count.
        let wrapper: Arc<WrapperInner<P>> = wrapper.into();
        // XXX: This unsafe block is unnecessary. rust-analyzer gets a bit confused and this this
        //      `write()` function is from `IBStream` which it definitely is not.
        *unsafe { wrapper.event_loop.write() } =
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
    unsafe fn set_normalized_value_by_hash(
        &self,
        hash: u32,
        normalized_value: f64,
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
                    param_ptr.set_normalized_value(normalized_value as f32);
                    param_ptr.update_smoother(sample_rate, false);
                }
                _ => param_ptr.set_normalized_value(normalized_value as f32),
            }

            kResultOk
        } else {
            kInvalidArgument
        }
    }
}

impl<P: Plugin> Wrapper<P> {
    pub fn new() -> Box<Self> {
        Self::allocate(WrapperInner::new())
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

impl<P: Plugin> IPluginBase for Wrapper<P> {
    unsafe fn initialize(&self, _context: *mut c_void) -> tresult {
        // We currently don't need or allow any initialization logic
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl<P: Plugin> IComponent for Wrapper<P> {
    unsafe fn get_controller_class_id(&self, _tuid: *mut vst3_sys::IID) -> tresult {
        // We won't separate the edit controller to keep the implemetnation a bit smaller
        kNoInterface
    }

    unsafe fn set_io_mode(&self, _mode: vst3_sys::vst::IoMode) -> tresult {
        // This would need to integrate with the GUI, which we currently don't have
        kResultOk
    }

    unsafe fn get_bus_count(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
    ) -> i32 {
        // All plugins currently only have a single input and a single output bus
        match type_ {
            x if x == vst3_sys::vst::MediaTypes::kAudio as i32 => 1,
            x if x == vst3_sys::vst::MediaTypes::kEvent as i32
                && dir == vst3_sys::vst::BusDirections::kInput as i32
                && P::ACCEPTS_MIDI =>
            {
                1
            }
            _ => 0,
        }
    }

    unsafe fn get_bus_info(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
        index: i32,
        info: *mut vst3_sys::vst::BusInfo,
    ) -> tresult {
        check_null_ptr!(info);

        match (type_, dir, index) {
            (t, _, _) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_sys::vst::MediaTypes::kAudio as i32;
                info.bus_type = vst3_sys::vst::BusTypes::kMain as i32;
                info.flags = vst3_sys::vst::BusFlags::kDefaultActive as u32;
                match (dir, index) {
                    (d, 0) if d == vst3_sys::vst::BusDirections::kInput as i32 => {
                        info.direction = vst3_sys::vst::BusDirections::kInput as i32;
                        info.channel_count =
                            self.inner.current_bus_config.load().num_input_channels as i32;
                        u16strlcpy(&mut info.name, "Input");

                        kResultOk
                    }
                    (d, 0) if d == vst3_sys::vst::BusDirections::kOutput as i32 => {
                        info.direction = vst3_sys::vst::BusDirections::kOutput as i32;
                        info.channel_count =
                            self.inner.current_bus_config.load().num_output_channels as i32;
                        u16strlcpy(&mut info.name, "Output");

                        kResultOk
                    }
                    _ => kInvalidArgument,
                }
            }
            (t, d, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && d == vst3_sys::vst::BusDirections::kInput as i32
                    && P::ACCEPTS_MIDI =>
            {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_sys::vst::MediaTypes::kEvent as i32;
                info.direction = vst3_sys::vst::BusDirections::kInput as i32;
                info.channel_count = 16;
                u16strlcpy(&mut info.name, "MIDI");
                info.bus_type = vst3_sys::vst::BusTypes::kMain as i32;
                info.flags = vst3_sys::vst::BusFlags::kDefaultActive as u32;
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn get_routing_info(
        &self,
        in_info: *mut vst3_sys::vst::RoutingInfo,
        out_info: *mut vst3_sys::vst::RoutingInfo,
    ) -> tresult {
        check_null_ptr!(in_info, out_info);

        *out_info = mem::zeroed();

        let in_info = &*in_info;
        let out_info = &mut *out_info;
        match (in_info.media_type, in_info.bus_index) {
            (t, 0) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => {
                out_info.media_type = vst3_sys::vst::MediaTypes::kAudio as i32;
                out_info.bus_index = in_info.bus_index;
                out_info.channel = in_info.channel;

                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn activate_bus(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
        index: i32,
        _state: vst3_sys::base::TBool,
    ) -> tresult {
        // We don't need any special handling here
        match (type_, dir, index) {
            (t, _, 0) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => kResultOk,
            (t, d, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && d == vst3_sys::vst::BusDirections::kInput as i32
                    && P::ACCEPTS_MIDI =>
            {
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn set_active(&self, _state: TBool) -> tresult {
        // We don't need any special handling here
        kResultOk
    }

    unsafe fn set_state(&self, state: SharedVstPtr<dyn IBStream>) -> tresult {
        check_null_ptr!(state);

        let state = state.upgrade().unwrap();

        // We need to know how large the state is before we can read it. The current position can be
        // zero, but it can also be something else. Bitwig prepends the preset header in the stream,
        // while some other hosts don't expose that to the plugin.
        let mut current_pos = 0;
        let mut eof_pos = 0;
        if state.tell(&mut current_pos) != kResultOk
            || state.seek(0, vst3_sys::base::kIBSeekEnd, &mut eof_pos) != kResultOk
            || state.seek(current_pos, vst3_sys::base::kIBSeekSet, ptr::null_mut()) != kResultOk
        {
            nih_debug_assert_failure!("Could not get the stream length");
            return kResultFalse;
        }

        let stream_byte_size = (eof_pos - current_pos) as i32;
        let mut num_bytes_read = 0;
        let mut read_buffer: Vec<u8> = Vec::with_capacity(stream_byte_size as usize);
        state.read(
            read_buffer.as_mut_ptr() as *mut c_void,
            read_buffer.capacity() as i32,
            &mut num_bytes_read,
        );
        read_buffer.set_len(num_bytes_read as usize);

        // If the size is zero, some hsots will always return `kResultFalse` even if the read was
        // 'successful', so we can't check the return value but we can check the number of bytes
        // read.
        if read_buffer.len() != stream_byte_size as usize {
            nih_debug_assert_failure!("Unexpected stream length");
            return kResultFalse;
        }

        let state: State = match serde_json::from_slice(&read_buffer) {
            Ok(s) => s,
            Err(err) => {
                nih_debug_assert_failure!("Error while deserializing state: {}", err);
                return kResultFalse;
            }
        };

        let sample_rate = self
            .inner
            .current_buffer_config
            .load()
            .map(|c| c.sample_rate);
        for (param_id_str, param_value) in state.params {
            // Handle the bypass parameter separately
            if param_id_str == BYPASS_PARAM_ID {
                match param_value {
                    ParamValue::Bool(b) => self.inner.bypass_state.store(b, Ordering::SeqCst),
                    _ => nih_debug_assert_failure!(
                        "Invalid serialized value {:?} for parameter \"{}\"",
                        param_value,
                        param_id_str,
                    ),
                };
                continue;
            }

            let param_ptr = match self
                .inner
                .param_id_to_hash
                .get(param_id_str.as_str())
                .and_then(|hash| self.inner.param_by_hash.get(hash))
            {
                Some(ptr) => ptr,
                None => {
                    nih_debug_assert_failure!("Unknown parameter: {}", param_id_str);
                    continue;
                }
            };

            match (param_ptr, param_value) {
                (ParamPtr::FloatParam(p), ParamValue::F32(v)) => (**p).set_plain_value(v),
                (ParamPtr::IntParam(p), ParamValue::I32(v)) => (**p).set_plain_value(v),
                (ParamPtr::BoolParam(p), ParamValue::Bool(v)) => (**p).set_plain_value(v),
                (param_ptr, param_value) => {
                    nih_debug_assert_failure!(
                        "Invalid serialized value {:?} for parameter \"{}\" ({:?})",
                        param_value,
                        param_id_str,
                        param_ptr,
                    );
                }
            }

            // Make sure everything starts out in sync
            if let Some(sample_rate) = sample_rate {
                param_ptr.update_smoother(sample_rate, true);
            }
        }

        // The plugin can also persist arbitrary fields alongside its parameters. This is useful for
        // storing things like sample data.
        self.inner
            .plugin
            .read()
            .params()
            .deserialize_fields(&state.fields);

        // Reinitialize the plugin after loading state so it can respond to the new parmaeters
        let bus_config = self.inner.current_bus_config.load();
        if let Some(buffer_config) = self.inner.current_buffer_config.load() {
            self.inner.plugin.write().initialize(
                &bus_config,
                &buffer_config,
                &mut self.inner.make_process_context(),
            );
        }

        kResultOk
    }

    unsafe fn get_state(&self, state: SharedVstPtr<dyn IBStream>) -> tresult {
        check_null_ptr!(state);

        let state = state.upgrade().unwrap();

        // We'll serialize parmaeter values as a simple `string_param_id: display_value` map.
        let mut params: HashMap<_, _> = self
            .inner
            .param_id_to_hash
            .iter()
            .filter_map(|(param_id_str, hash)| {
                let param_ptr = self.inner.param_by_hash.get(hash)?;
                Some((param_id_str, param_ptr))
            })
            .map(|(&param_id_str, &param_ptr)| match param_ptr {
                ParamPtr::FloatParam(p) => (
                    param_id_str.to_string(),
                    ParamValue::F32((*p).plain_value()),
                ),
                ParamPtr::IntParam(p) => (
                    param_id_str.to_string(),
                    ParamValue::I32((*p).plain_value()),
                ),
                ParamPtr::BoolParam(p) => (
                    param_id_str.to_string(),
                    ParamValue::Bool((*p).plain_value()),
                ),
            })
            .collect();

        // Don't forget about the bypass parameter
        params.insert(
            BYPASS_PARAM_ID.to_string(),
            ParamValue::Bool(self.inner.bypass_state.load(Ordering::SeqCst)),
        );

        // The plugin can also persist arbitrary fields alongside its parameters. This is useful for
        // storing things like sample data.
        let fields = self.inner.plugin.read().params().serialize_fields();

        let plugin_state = State { params, fields };
        match serde_json::to_vec(&plugin_state) {
            Ok(serialized) => {
                let mut num_bytes_written = 0;
                let result = state.write(
                    serialized.as_ptr() as *const c_void,
                    serialized.len() as i32,
                    &mut num_bytes_written,
                );

                nih_debug_assert_eq!(result, kResultOk);
                nih_debug_assert_eq!(num_bytes_written as usize, serialized.len());
                kResultOk
            }
            Err(err) => {
                nih_debug_assert_failure!("Could not save state: {}", err);
                kResultFalse
            }
        }
    }
}

impl<P: Plugin> IEditController for Wrapper<P> {
    unsafe fn set_component_state(&self, _state: SharedVstPtr<dyn IBStream>) -> tresult {
        // We have a single file component, so we don't need to do anything here
        kResultOk
    }

    unsafe fn set_state(&self, _state: SharedVstPtr<dyn IBStream>) -> tresult {
        // We don't store any separate state here. The plugin's state will have been restored
        // through the component. Calling that same function here will likely lead to duplicate
        // state restores
        kResultOk
    }

    unsafe fn get_state(&self, _state: SharedVstPtr<dyn IBStream>) -> tresult {
        // Same for this function
        kResultOk
    }

    unsafe fn get_parameter_count(&self) -> i32 {
        // NOTE: We add a bypass parameter ourselves on index `self.param_ids.len()`, so these
        //       indices are all off by one
        self.inner.param_hashes.len() as i32 + 1
    }

    unsafe fn get_parameter_info(
        &self,
        param_index: i32,
        info: *mut vst3_sys::vst::ParameterInfo,
    ) -> tresult {
        check_null_ptr!(info);

        // Parameter index `self.param_ids.len()` is our own bypass parameter
        if param_index < 0 || param_index > self.inner.param_hashes.len() as i32 {
            return kInvalidArgument;
        }

        *info = std::mem::zeroed();

        let info = &mut *info;
        if param_index == self.inner.param_hashes.len() as i32 {
            info.id = *BYPASS_PARAM_HASH;
            u16strlcpy(&mut info.title, "Bypass");
            u16strlcpy(&mut info.short_title, "Bypass");
            u16strlcpy(&mut info.units, "");
            info.step_count = 1;
            info.default_normalized_value = 0.0;
            info.unit_id = vst3_sys::vst::kRootUnitId;
            info.flags = vst3_sys::vst::ParameterFlags::kCanAutomate as i32
                | vst3_sys::vst::ParameterFlags::kIsBypass as i32;
        } else {
            let param_hash = &self.inner.param_hashes[param_index as usize];
            let default_value = &self.inner.param_defaults_normalized[param_index as usize];
            let param_ptr = &self.inner.param_by_hash[param_hash];

            info.id = *param_hash;
            u16strlcpy(&mut info.title, param_ptr.name());
            u16strlcpy(&mut info.short_title, param_ptr.name());
            u16strlcpy(&mut info.units, param_ptr.unit());
            info.step_count = match param_ptr {
                ParamPtr::FloatParam(_) => 0,
                ParamPtr::IntParam(p) => match (**p).range {
                    Range::Linear { min, max } => max - min,
                    Range::Skewed { min, max, .. } => max - min,
                    Range::SymmetricalSkewed { min, max, .. } => max - min,
                },
                ParamPtr::BoolParam(_) => 1,
            };
            info.default_normalized_value = *default_value as f64;
            info.unit_id = vst3_sys::vst::kRootUnitId;
            info.flags = vst3_sys::vst::ParameterFlags::kCanAutomate as i32;
        }

        kResultOk
    }

    unsafe fn get_param_string_by_value(
        &self,
        id: u32,
        value_normalized: f64,
        string: *mut TChar,
    ) -> tresult {
        check_null_ptr!(string);

        // Somehow there's no length there, so we'll assume our own maximum
        let dest = &mut *(string as *mut [TChar; 128]);

        if id == *BYPASS_PARAM_HASH {
            if value_normalized > 0.5 {
                u16strlcpy(dest, "Bypassed")
            } else {
                u16strlcpy(dest, "Enabled")
            }

            kResultOk
        } else if let Some(param_ptr) = self.inner.param_by_hash.get(&id) {
            u16strlcpy(
                dest,
                &param_ptr.normalized_value_to_string(value_normalized as f32, false),
            );

            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn get_param_value_by_string(
        &self,
        id: u32,
        string: *const TChar,
        value_normalized: *mut f64,
    ) -> tresult {
        check_null_ptr!(string, value_normalized);

        let string = match U16CStr::from_ptr_str(string as *const u16).to_string() {
            Ok(s) => s,
            Err(_) => return kInvalidArgument,
        };

        if id == *BYPASS_PARAM_HASH {
            let value = match string.as_str() {
                "Bypassed" => 1.0,
                "Enabled" => 0.0,
                _ => return kResultFalse,
            };
            *value_normalized = value;

            kResultOk
        } else if let Some(param_ptr) = self.inner.param_by_hash.get(&id) {
            let value = match param_ptr.string_to_normalized_value(&string) {
                Some(v) => v as f64,
                None => return kResultFalse,
            };
            *value_normalized = value;

            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn normalized_param_to_plain(&self, id: u32, value_normalized: f64) -> f64 {
        if id == *BYPASS_PARAM_HASH {
            value_normalized
        } else if let Some(param_ptr) = self.inner.param_by_hash.get(&id) {
            param_ptr.preview_plain(value_normalized as f32) as f64
        } else {
            0.5
        }
    }

    unsafe fn plain_param_to_normalized(&self, id: u32, plain_value: f64) -> f64 {
        if id == *BYPASS_PARAM_HASH {
            plain_value.clamp(0.0, 1.0)
        } else if let Some(param_ptr) = self.inner.param_by_hash.get(&id) {
            param_ptr.preview_normalized(plain_value as f32) as f64
        } else {
            0.5
        }
    }

    unsafe fn get_param_normalized(&self, id: u32) -> f64 {
        if id == *BYPASS_PARAM_HASH {
            if self.inner.bypass_state.load(Ordering::SeqCst) {
                1.0
            } else {
                0.0
            }
        } else if let Some(param_ptr) = self.inner.param_by_hash.get(&id) {
            param_ptr.normalized_value() as f64
        } else {
            0.5
        }
    }

    unsafe fn set_param_normalized(&self, id: u32, value: f64) -> tresult {
        // If the plugin is currently processing audio, then this parameter change will also be sent
        // to the process function
        if self.inner.is_processing.load(Ordering::SeqCst) {
            return kResultOk;
        }

        let sample_rate = self
            .inner
            .current_buffer_config
            .load()
            .map(|c| c.sample_rate);
        self.inner
            .set_normalized_value_by_hash(id, value, sample_rate)
    }

    unsafe fn set_component_handler(
        &self,
        handler: SharedVstPtr<dyn vst3_sys::vst::IComponentHandler>,
    ) -> tresult {
        *self.inner.component_handler.write() = handler.upgrade().map(VstPtr::from);

        kResultOk
    }

    unsafe fn create_view(&self, _name: vst3_sys::base::FIDString) -> *mut c_void {
        // We currently don't support GUIs
        std::ptr::null_mut()
    }
}

impl<P: Plugin> IAudioProcessor for Wrapper<P> {
    unsafe fn set_bus_arrangements(
        &self,
        inputs: *mut vst3_sys::vst::SpeakerArrangement,
        num_ins: i32,
        outputs: *mut vst3_sys::vst::SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        check_null_ptr!(inputs, outputs);

        // We currently only do single audio bus IO configurations
        if num_ins != 1 || num_outs != 1 {
            return kInvalidArgument;
        }

        let input_channel_map = &*inputs;
        let output_channel_map = &*outputs;
        let proposed_config = BusConfig {
            num_input_channels: input_channel_map.count_ones(),
            num_output_channels: output_channel_map.count_ones(),
        };
        if self
            .inner
            .plugin
            .read()
            .accepts_bus_config(&proposed_config)
        {
            self.inner.current_bus_config.store(proposed_config);

            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn get_bus_arrangement(
        &self,
        dir: vst3_sys::vst::BusDirection,
        index: i32,
        arr: *mut vst3_sys::vst::SpeakerArrangement,
    ) -> tresult {
        check_null_ptr!(arr);

        let channel_count_to_map = |count| match count {
            0 => vst3_sys::vst::kEmpty,
            1 => vst3_sys::vst::kMono,
            2 => vst3_sys::vst::kStereo,
            5 => vst3_sys::vst::k50,
            6 => vst3_sys::vst::k51,
            7 => vst3_sys::vst::k70Cine,
            8 => vst3_sys::vst::k71Cine,
            n => {
                nih_debug_assert_failure!(
                    "No defined layout for {} channels, making something up on the spot...",
                    n
                );
                (1 << n) - 1
            }
        };

        let config = self.inner.current_bus_config.load();
        let num_channels = match (dir, index) {
            (d, 0) if d == vst3_sys::vst::BusDirections::kInput as i32 => config.num_input_channels,
            (d, 0) if d == vst3_sys::vst::BusDirections::kOutput as i32 => {
                config.num_output_channels
            }
            _ => return kInvalidArgument,
        };
        let channel_map = channel_count_to_map(num_channels);

        nih_debug_assert_eq!(num_channels, channel_map.count_ones());
        *arr = channel_map;

        kResultOk
    }

    unsafe fn can_process_sample_size(&self, symbolic_sample_size: i32) -> tresult {
        if symbolic_sample_size == vst3_sys::vst::SymbolicSampleSizes::kSample32 as i32 {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn get_latency_samples(&self) -> u32 {
        self.inner.current_latency.load(Ordering::SeqCst)
    }

    unsafe fn setup_processing(&self, setup: *const vst3_sys::vst::ProcessSetup) -> tresult {
        check_null_ptr!(setup);

        // There's no special handling for offline processing at the moment
        let setup = &*setup;
        nih_debug_assert_eq!(
            setup.symbolic_sample_size,
            vst3_sys::vst::SymbolicSampleSizes::kSample32 as i32
        );

        let bus_config = self.inner.current_bus_config.load();
        let buffer_config = BufferConfig {
            sample_rate: setup.sample_rate as f32,
            max_buffer_size: setup.max_samples_per_block as u32,
        };

        // Befure initializing the plugin, make sure all smoothers are set the the default values
        for param in self.inner.param_by_hash.values() {
            param.update_smoother(buffer_config.sample_rate, true);
        }

        if self.inner.plugin.write().initialize(
            &bus_config,
            &buffer_config,
            &mut self.inner.make_process_context(),
        ) {
            // Preallocate enough room in the output slices vector so we can convert a `*mut *mut
            // f32` to a `&mut [&mut f32]` in the process call
            self.inner
                .output_buffer
                .write()
                .as_raw_vec()
                .resize_with(bus_config.num_output_channels as usize, || &mut []);

            // Also store this for later, so we can reinitialize the plugin after restoring state
            self.inner.current_buffer_config.store(Some(buffer_config));

            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn set_processing(&self, state: TBool) -> tresult {
        // Always reset the processing status when the plugin gets activated or deactivated
        self.inner.last_process_status.store(ProcessStatus::Normal);
        self.inner.is_processing.store(state != 0, Ordering::SeqCst);

        // We don't have any special handling for suspending and resuming plugins, yet
        kResultOk
    }

    unsafe fn process(&self, data: *mut vst3_sys::vst::ProcessData) -> tresult {
        check_null_ptr!(data);

        // Panic on allocations if the `assert_process_allocs` feature has been enabled, and make
        // sure that FTZ is set up correctly
        process_wrapper(|| {
            // We need to handle incoming automation first
            let data = &*data;
            let sample_rate = self
                .inner
                .current_buffer_config
                .load()
                .map(|c| c.sample_rate);
            if let Some(param_changes) = data.input_param_changes.upgrade() {
                let num_param_queues = param_changes.get_parameter_count();
                for change_queue_idx in 0..num_param_queues {
                    if let Some(param_change_queue) =
                        param_changes.get_parameter_data(change_queue_idx).upgrade()
                    {
                        let param_hash = param_change_queue.get_parameter_id();
                        let num_changes = param_change_queue.get_point_count();

                        // TODO: Handle sample accurate parameter changes, possibly in a similar way to
                        //       the smoothing
                        let mut sample_offset = 0i32;
                        let mut value = 0.0f64;
                        if num_changes > 0
                            && param_change_queue.get_point(
                                num_changes - 1,
                                &mut sample_offset,
                                &mut value,
                            ) == kResultOk
                        {
                            self.inner
                                .set_normalized_value_by_hash(param_hash, value, sample_rate);
                        }
                    }
                }
            }

            // And also incoming note events if the plugin accepts MDII
            if P::ACCEPTS_MIDI {
                let mut input_events = self.inner.input_events.write();
                if let Some(events) = data.input_events.upgrade() {
                    let num_events = events.get_event_count();

                    input_events.clear();
                    let mut event: MaybeUninit<_> = MaybeUninit::uninit();
                    for i in 0..num_events {
                        nih_debug_assert_eq!(events.get_event(i, event.as_mut_ptr()), kResultOk);
                        let event = event.assume_init();
                        let timing = event.sample_offset as u32;
                        if event.type_ == vst3_sys::vst::EventTypes::kNoteOnEvent as u16 {
                            let event = event.event.note_on;
                            input_events.push_back(NoteEvent::NoteOn {
                                timing,
                                channel: event.channel as u8,
                                note: event.pitch as u8,
                                velocity: (event.velocity * 127.0).round() as u8,
                            });
                        } else if event.type_ == vst3_sys::vst::EventTypes::kNoteOffEvent as u16 {
                            let event = event.event.note_off;
                            input_events.push_back(NoteEvent::NoteOff {
                                timing,
                                channel: event.channel as u8,
                                note: event.pitch as u8,
                                velocity: (event.velocity * 127.0).round() as u8,
                            });
                        }
                    }
                }
            }

            // It's possible the host only wanted to send new parameter values
            if data.num_outputs == 0 {
                nih_log!("VST3 parameter flush");
                return kResultOk;
            }

            // The setups we suppport are:
            // - 1 input bus
            // - 1 output bus
            // - 1 input bus and 1 output bus
            nih_debug_assert!(
                data.num_inputs >= 0
                    && data.num_inputs <= 1
                    && data.num_outputs >= 0
                    && data.num_outputs <= 1,
                "The host provides more than one input or output bus"
            );
            nih_debug_assert_eq!(
                data.symbolic_sample_size,
                vst3_sys::vst::SymbolicSampleSizes::kSample32 as i32
            );
            nih_debug_assert!(data.num_samples >= 0);

            let num_output_channels = (*data.outputs).num_channels as usize;
            check_null_ptr_msg!(
                "Process output pointer is null",
                data.outputs,
                (*data.outputs).buffers,
            );

            // This vector has been reallocated to contain enough slices as there are output
            // channels
            let mut output_buffer = self.inner.output_buffer.write();
            {
                let output_slices = output_buffer.as_raw_vec();
                nih_debug_assert_eq!(num_output_channels, output_slices.len());
                for (output_channel_idx, output_channel_slice) in
                    output_slices.iter_mut().enumerate()
                {
                    // SAFETY: These pointers may not be valid outside of this function even though
                    // their lifetime is equal to this structs. This is still safe because they are
                    // only dereferenced here later as part of this process function.
                    *output_channel_slice = std::slice::from_raw_parts_mut(
                        *((*data.outputs).buffers as *mut *mut f32).add(output_channel_idx),
                        data.num_samples as usize,
                    );
                }
            }

            // Most hosts process data in place, in which case we don't need to do any copying
            // ourselves. If the pointers do not alias, then we'll do the copy here and then the
            // plugin can just do normal in place processing.
            if !data.inputs.is_null() {
                let num_input_channels = (*data.inputs).num_channels as usize;
                nih_debug_assert!(
                    num_input_channels <= num_output_channels,
                    "Stereo to mono and similar configurations are not supported"
                );
                for input_channel_idx in 0..cmp::min(num_input_channels, num_output_channels) {
                    let output_channel_ptr =
                        *((*data.outputs).buffers as *mut *mut f32).add(input_channel_idx);
                    let input_channel_ptr =
                        *((*data.inputs).buffers as *const *const f32).add(input_channel_idx);
                    if input_channel_ptr != output_channel_ptr {
                        ptr::copy_nonoverlapping(
                            input_channel_ptr,
                            output_channel_ptr,
                            data.num_samples as usize,
                        );
                    }
                }
            }

            match self
                .inner
                .plugin
                .write()
                // SAFETY: Same here
                .process(&mut output_buffer, &mut self.inner.make_process_context())
            {
                ProcessStatus::Error(err) => {
                    nih_debug_assert_failure!("Process error: {}", err);

                    kResultFalse
                }
                _ => kResultOk,
            }
        })
    }

    unsafe fn get_tail_samples(&self) -> u32 {
        // https://github.com/steinbergmedia/vst3_pluginterfaces/blob/2ad397ade5b51007860bedb3b01b8afd2c5f6fba/vst/ivstaudioprocessor.h#L145-L159
        match self.inner.last_process_status.load() {
            ProcessStatus::Tail(samples) => samples,
            ProcessStatus::KeepAlive => u32::MAX, // kInfiniteTail
            _ => 0,                               // kNoTail
        }
    }
}

#[doc(hidden)]
#[VST3(implements(IPluginFactory, IPluginFactory2, IPluginFactory3))]
pub struct Factory<P: Vst3Plugin> {
    /// The exposed plugin's GUID. Instead of generating this, we'll just let the programmer decide
    /// on their own.
    cid: GUID,
    /// The type will be used for constructing plugin instances later.
    _phantom: PhantomData<P>,
}

impl<P: Vst3Plugin> Factory<P> {
    pub fn new() -> Box<Self> {
        Self::allocate(
            GUID {
                data: P::VST3_CLASS_ID,
            },
            PhantomData::default(),
        )
    }
}

impl<P: Vst3Plugin> IPluginFactory for Factory<P> {
    unsafe fn get_factory_info(&self, info: *mut vst3_sys::base::PFactoryInfo) -> tresult {
        *info = mem::zeroed();

        let info = &mut *info;
        strlcpy(&mut info.vendor, P::VENDOR);
        strlcpy(&mut info.url, P::URL);
        strlcpy(&mut info.email, P::EMAIL);
        info.flags = vst3_sys::base::FactoryFlags::kUnicode as i32;

        kResultOk
    }

    unsafe fn count_classes(&self) -> i32 {
        // We don't do shell plugins, and good of an idea having separated components and edit
        // controllers in theory is, few software can use it, and doing that would make our simple
        // microframework a lot less simple
        1
    }

    unsafe fn get_class_info(&self, index: i32, info: *mut vst3_sys::base::PClassInfo) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid = self.cid;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        strlcpy(&mut info.name, P::NAME);

        kResultOk
    }

    unsafe fn create_instance(
        &self,
        cid: *const vst3_sys::IID,
        _iid: *const vst3_sys::IID,
        obj: *mut *mut vst3_sys::c_void,
    ) -> tresult {
        check_null_ptr!(cid, obj);

        if *cid != self.cid {
            return kNoInterface;
        }

        *obj = Box::into_raw(Wrapper::<P>::new()) as *mut vst3_sys::c_void;

        kResultOk
    }
}

impl<P: Vst3Plugin> IPluginFactory2 for Factory<P> {
    unsafe fn get_class_info2(
        &self,
        index: i32,
        info: *mut vst3_sys::base::PClassInfo2,
    ) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid = self.cid;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        strlcpy(&mut info.name, P::NAME);
        info.class_flags = 1 << 1; // kSimpleModeSupported
        strlcpy(&mut info.subcategories, P::VST3_CATEGORIES);
        strlcpy(&mut info.vendor, P::VENDOR);
        strlcpy(&mut info.version, P::VERSION);
        strlcpy(&mut info.sdk_version, VST3_SDK_VERSION);

        kResultOk
    }
}

impl<P: Vst3Plugin> IPluginFactory3 for Factory<P> {
    unsafe fn get_class_info_unicode(
        &self,
        index: i32,
        info: *mut vst3_sys::base::PClassInfoW,
    ) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid = self.cid;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        u16strlcpy(&mut info.name, P::NAME);
        info.class_flags = 1 << 1; // kSimpleModeSupported
        strlcpy(&mut info.subcategories, P::VST3_CATEGORIES);
        u16strlcpy(&mut info.vendor, P::VENDOR);
        u16strlcpy(&mut info.version, P::VERSION);
        u16strlcpy(&mut info.sdk_version, VST3_SDK_VERSION);

        kResultOk
    }

    unsafe fn set_host_context(&self, _context: *mut c_void) -> tresult {
        // We don't need to do anything with this
        kResultOk
    }
}

/// Export a VST3 plugin from this library using the provided plugin type.
#[macro_export]
macro_rules! nih_export_vst3 {
    ($plugin_ty:ty) => {
        #[no_mangle]
        pub extern "system" fn GetPluginFactory() -> *mut ::std::ffi::c_void {
            let factory = ::nih_plug::wrapper::vst3::Factory::<$plugin_ty>::new();

            Box::into_raw(factory) as *mut ::std::ffi::c_void
        }

        // We don't need any special initialization logic, so all of these module entry point
        // functions just return true all the time

        // These two entry points are used on Linux, and they would theoretically also be used on
        // the BSDs:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/c3948deb407bdbff89de8fb6ab8500ea4df9d6d9/source/main/linuxmain.cpp#L47-L52
        #[no_mangle]
        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        pub extern "C" fn ModuleEntry(_lib_handle: *mut ::std::ffi::c_void) -> bool {
            true
        }

        #[no_mangle]
        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        pub extern "C" fn ModuleExit() -> bool {
            true
        }

        // These two entry points are used on macOS:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/bc459feee68803346737901471441fd4829ec3f9/source/main/macmain.cpp#L60-L61
        #[no_mangle]
        #[cfg(target_os = "macos")]
        pub extern "C" fn bundleEntry(_lib_handle: *mut ::std::ffi::c_void) -> bool {
            true
        }

        #[no_mangle]
        #[cfg(target_os = "macos")]
        pub extern "C" fn bundleExit() -> bool {
            true
        }

        // And these two entry points are used on Windows:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/bc459feee68803346737901471441fd4829ec3f9/source/main/dllmain.cpp#L59-L60
        #[no_mangle]
        #[cfg(target_os = "windows")]
        pub extern "system" fn InitModule() -> bool {
            true
        }

        #[no_mangle]
        #[cfg(target_os = "windows")]
        pub extern "system" fn DeinitModule() -> bool {
            true
        }
    };
}
