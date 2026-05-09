//! AU wrapper, Phase 3.5.
//!
//! Hosts the plugin's `Params` so:
//!   - `kAudioUnitProperty_ParameterList` returns the AU parameter ID array
//!   - `kAudioUnitProperty_ParameterInfo` returns name / range / default / unit
//!   - `AudioUnitGet/SetParameter` hand off to the underlying nih-plug `ParamPtr`
//!
//! AU parameter IDs are simply the index into `param_map()` — stable across
//! one binary build (the nih-plug derive guarantees field declaration order).
//!
//! # Concurrency model
//!
//! AU hosts call us from at least two threads concurrently:
//!   - **main thread** — `SetProperty`, `GetProperty`, `SetParameter` (UI),
//!     `Initialize` / `Uninitialize`
//!   - **audio thread** — `Render`, `SetParameter` (sample-accurate automation)
//!
//! The wrapper exposes only `&Self` from `from_ptr` so we can never construct
//! two `&mut Self` simultaneously. Mutable state is split into three buckets:
//!
//!   1. **host-setup atomics** — `sample_rate`, `n_channels`,
//!      `max_frames_per_slice`, `latency_seconds`, `initialized`. Written by
//!      main thread before `Initialize`, read by the audio thread thereafter.
//!      `AtomicU32`/`AtomicU64` (latency/sr packed as f64 bits).
//!
//!   2. **audio-thread-owned scratch** — `input_scratch`. Inside `UnsafeCell`,
//!      only ever touched from `render()`. AU guarantees render is not
//!      re-entered, so a `&mut` borrow inside that scope is sound.
//!
//!   3. **main↔audio shared state** — `input_callback`. `Mutex<Option<…>>`;
//!      main thread updates rarely, audio thread snapshots into a local `Copy`
//!      at the top of `render()`. The mutex is held only for the snapshot.

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use au_sys as au;

use crate::buffer::Buffer;
use crate::context::process::Transport;
use crate::params::internals::ParamPtr;
use crate::params::Params;
use crate::plugin::au::AuPlugin;
use crate::prelude::{AudioIOLayout, AuxiliaryBuffers, BufferConfig, ProcessMode};

use super::context::{AuInitContext, AuProcessContext, ContextSink};

/// One AU plugin instance. Owned by Apple's component manager via the
/// `AudioComponentPlugInInterface` pointer returned from the factory.
///
/// The first field MUST be the vtable, since Apple's component manager
/// dispatches through `instance->vtable->Lookup(selector)`.
#[repr(C)]
pub struct Wrapper<P: AuPlugin> {
    /// Apple-required vtable. MUST be the first field.
    vtable: au::AudioComponentPlugInInterface,

    /// Host's `AudioUnit` opaque handle, set in `Open` (main thread, once).
    /// Stored as raw ptr behind atomic so we can read from any thread.
    instance: AtomicU64,

    /// Sample rate set via `kAudioUnitProperty_StreamFormat`. f64 bits.
    sample_rate_bits: AtomicU64,

    /// Maximum frames per `AudioUnitRender` slice.
    max_frames_per_slice: AtomicU32,

    /// Channel count set via `StreamFormat`.
    n_channels: AtomicU32,

    /// Latency reported via `kAudioUnitProperty_Latency`. f64 bits.
    latency_seconds_bits: AtomicU64,

    /// Whether `Plugin::initialize()` has run successfully since the last
    /// `Uninitialize` / first construction. Render is a no-op when false.
    initialized: AtomicBool,

    /// The plugin instance. Kept for the entire lifetime of the wrapper so
    /// the `ParamPtr` raw pointers in `params_by_id` remain valid.
    ///
    /// Wrapped in `UnsafeCell` because `Plugin::initialize` / `process` /
    /// `reset` / `deactivate` take `&mut self`, but we only ever expose `&Self`
    /// from `from_ptr`. AU's threading model guarantees these methods are not
    /// concurrent with each other (Initialize/Uninitialize/Reset run on main,
    /// process runs on audio thread, and the host serialises lifecycle vs.
    /// render via `Initialize`/`Uninitialize` boundaries — render can only
    /// happen between them).
    plugin: UnsafeCell<Box<P>>,

    /// Parameter handles in declaration order. The AU parameter ID is the
    /// index into this vec. Built once in `new()`, then read-only.
    params_by_id: Vec<ParamEntry>,

    /// Strong reference back to the `Params` object referenced by every
    /// `ParamPtr` in `params_by_id`.
    _params_arc: Arc<dyn Params>,

    /// Scratch sink shared between Init/Process contexts. Lets the plugin
    /// report a latency change back via `set_latency_samples()`.
    sink: Arc<ContextSink>,

    /// Input render callback registered by the host via
    /// `kAudioUnitProperty_SetRenderCallback`. We invoke this at the top of
    /// every `render()` call to pull input audio for the effect.
    ///
    /// `AURenderCallbackStruct` is `Copy`. Audio thread snapshots out under
    /// the mutex at the start of render and releases immediately.
    input_callback: Mutex<Option<au::AURenderCallbackStruct>>,

    /// Per-channel scratch buffers for the input pull. Sized in `Initialize`
    /// to `[max_frames_per_slice]` so the render path is allocation-free.
    /// One `Vec<f32>` per channel; we hand the host a synthesised
    /// `AudioBufferList` whose `mData` pointers point into these vectors.
    ///
    /// `UnsafeCell` because only `render()` touches it after `Initialize`,
    /// and AU does not re-enter render.
    input_scratch: UnsafeCell<Vec<Vec<f32>>>,
}

struct ParamEntry {
    /// Stable string ID — currently unused but kept for future state save/load.
    #[allow(dead_code)]
    id_str: String,
    ptr: ParamPtr,
}

/// `Send` + `Sync` justification:
///
/// - All publicly mutable fields are atomic or behind a `Mutex`.
/// - `plugin` and `input_scratch` are `UnsafeCell`-wrapped, accessed only
///   through `&mut` borrows constructed during a single audio-thread render
///   call (`input_scratch`) or during main-thread lifecycle calls that AU
///   serialises against render (`plugin`). The host contract forbids
///   `Initialize` / `Uninitialize` / `Reset` concurrent with `Render`.
/// - `ParamPtr` in `params_by_id` is built once and read-only thereafter;
///   per nih-plug contract, `set_normalized_value` and the value getters
///   are themselves thread-safe.
/// - The `vtable` and `_params_arc` are immutable after construction.
unsafe impl<P: AuPlugin> Send for Wrapper<P> {}
unsafe impl<P: AuPlugin> Sync for Wrapper<P> {}

#[inline]
fn pack_f64(v: f64) -> u64 {
    v.to_bits()
}
#[inline]
fn unpack_f64(b: u64) -> f64 {
    f64::from_bits(b)
}

impl<P: AuPlugin> Wrapper<P> {
    pub fn new() -> *mut au::AudioComponentPlugInInterface {
        let plugin = Box::new(P::default());
        let params_arc = plugin.params();

        let params_by_id: Vec<ParamEntry> = params_arc
            .param_map()
            .into_iter()
            .map(|(id_str, ptr, _group)| ParamEntry { id_str, ptr })
            .collect();

        let boxed = Box::new(Wrapper::<P> {
            vtable: au::AudioComponentPlugInInterface {
                Open: Self::open,
                Close: Self::close,
                Lookup: Self::lookup,
                reserved: ptr::null_mut(),
            },
            instance: AtomicU64::new(0),
            sample_rate_bits: AtomicU64::new(pack_f64(44_100.0)),
            max_frames_per_slice: AtomicU32::new(1024),
            n_channels: AtomicU32::new(2),
            latency_seconds_bits: AtomicU64::new(pack_f64(0.0)),
            initialized: AtomicBool::new(false),
            plugin: UnsafeCell::new(plugin),
            params_by_id,
            _params_arc: params_arc,
            sink: ContextSink::new(),
            input_callback: Mutex::new(None),
            input_scratch: UnsafeCell::new(Vec::new()),
        });
        Box::into_raw(boxed) as *mut au::AudioComponentPlugInInterface
    }

    /// Returns a shared reference to the wrapper. We never hand out `&mut Self`
    /// because AU's main + audio threads can both be inside the wrapper at
    /// the same time and `&mut Self` aliasing would be UB.
    #[inline]
    unsafe fn from_ptr<'a>(ptr: *mut c_void) -> &'a Self {
        unsafe { &*(ptr as *const Self) }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Field accessors (atomic)
    // ─────────────────────────────────────────────────────────────────────

    #[inline]
    fn sample_rate(&self) -> f64 {
        unpack_f64(self.sample_rate_bits.load(Ordering::Acquire))
    }
    #[inline]
    fn set_sample_rate(&self, sr: f64) {
        self.sample_rate_bits.store(pack_f64(sr), Ordering::Release);
    }
    #[inline]
    fn latency_seconds(&self) -> f64 {
        unpack_f64(self.latency_seconds_bits.load(Ordering::Acquire))
    }
    #[inline]
    fn set_latency_seconds(&self, l: f64) {
        self.latency_seconds_bits
            .store(pack_f64(l), Ordering::Release);
    }
    #[inline]
    fn n_channels(&self) -> u32 {
        self.n_channels.load(Ordering::Acquire)
    }
    #[inline]
    fn max_frames_per_slice(&self) -> u32 {
        self.max_frames_per_slice.load(Ordering::Acquire)
    }
    #[inline]
    fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    /// SAFETY: caller must guarantee no other reference (mut or shared) to
    /// `*self.plugin.get()` is alive. AU's contract gives this for the
    /// `Initialize` / `Uninitialize` / `Reset` / `Render` paths because the
    /// host serialises lifecycle calls vs. render and never re-enters render.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn plugin_mut(&self) -> &mut P {
        unsafe { &mut **self.plugin.get() }
    }

    /// SAFETY: same as `plugin_mut` — only call from `render()`. Returns the
    /// scratch vector for in-place mutation by the audio thread.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn input_scratch_mut(&self) -> &mut Vec<Vec<f32>> {
        unsafe { &mut *self.input_scratch.get() }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Component manager vtable
    // ─────────────────────────────────────────────────────────────────────

    unsafe extern "C" fn open(self_ptr: *mut c_void, instance: au::AudioUnit) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        this.instance
            .store(instance as usize as u64, Ordering::Release);
        au::noErr
    }

    unsafe extern "C" fn close(self_ptr: *mut c_void) -> au::OSStatus {
        unsafe {
            let _ = Box::from_raw(self_ptr as *mut Self);
        }
        au::noErr
    }

    unsafe extern "C" fn lookup(selector: au::SInt16) -> Option<au::AudioComponentMethod> {
        let method: au::AudioComponentMethod = match selector {
            au::kAudioUnitInitializeSelect => unsafe {
                std::mem::transmute::<au::AudioUnitInitializeProc, _>(Self::initialize)
            },
            au::kAudioUnitUninitializeSelect => unsafe {
                std::mem::transmute::<au::AudioUnitUninitializeProc, _>(Self::uninitialize)
            },
            au::kAudioUnitGetPropertyInfoSelect => unsafe {
                std::mem::transmute::<au::AudioUnitGetPropertyInfoProc, _>(
                    Self::get_property_info,
                )
            },
            au::kAudioUnitGetPropertySelect => unsafe {
                std::mem::transmute::<au::AudioUnitGetPropertyProc, _>(Self::get_property)
            },
            au::kAudioUnitSetPropertySelect => unsafe {
                std::mem::transmute::<au::AudioUnitSetPropertyProc, _>(Self::set_property)
            },
            au::kAudioUnitGetParameterSelect => unsafe {
                std::mem::transmute::<au::AudioUnitGetParameterProc, _>(Self::get_parameter)
            },
            au::kAudioUnitSetParameterSelect => unsafe {
                std::mem::transmute::<au::AudioUnitSetParameterProc, _>(Self::set_parameter)
            },
            au::kAudioUnitResetSelect => unsafe {
                std::mem::transmute::<au::AudioUnitResetProc, _>(Self::reset)
            },
            au::kAudioUnitRenderSelect => unsafe {
                std::mem::transmute::<au::AudioUnitRenderProc, _>(Self::render)
            },
            au::kAudioUnitAddPropertyListenerSelect => unsafe {
                std::mem::transmute::<au::AudioUnitAddPropertyListenerProc, _>(
                    Self::add_property_listener,
                )
            },
            au::kAudioUnitRemovePropertyListenerWithUserDataSelect => unsafe {
                std::mem::transmute::<au::AudioUnitRemovePropertyListenerWithUserDataProc, _>(
                    Self::remove_property_listener_with_user_data,
                )
            },
            _ => return None,
        };
        Some(method)
    }

    /// Called by the host once the audio configuration has been fully
    /// queried — sample rate, stream format, max frames per slice are all
    /// already set. Build the nih-plug `BufferConfig` / `AudioIOLayout`
    /// from those and forward to `Plugin::initialize`.
    unsafe extern "C" fn initialize(self_ptr: *mut c_void) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };

        let n_ch = this.n_channels().max(1);
        let chans = NonZeroU32::new(n_ch);
        let io_layout = AudioIOLayout {
            main_input_channels: chans,
            main_output_channels: chans,
            ..AudioIOLayout::const_default()
        };
        let max_frames = this.max_frames_per_slice();
        let sr = this.sample_rate();
        let buffer_config = BufferConfig {
            sample_rate: sr as f32,
            min_buffer_size: None,
            max_buffer_size: max_frames,
            process_mode: ProcessMode::Realtime,
        };

        let mut ctx = AuInitContext::<P> {
            sink: this.sink.clone(),
            _marker: PhantomData,
        };
        // SAFETY: AU forbids Initialize concurrent with Render.
        let plugin = unsafe { this.plugin_mut() };
        let ok = plugin.initialize(&io_layout, &buffer_config, &mut ctx);
        if !ok {
            return au::kAudioUnitErr_FailedInitialization;
        }
        plugin.reset();

        // Allocate input scratch space: one Vec<f32> per channel, each
        // sized to max_frames_per_slice so the render path is allocation
        // free.
        // SAFETY: same — render is serialised vs. Initialize.
        let scratch = unsafe { this.input_scratch_mut() };
        scratch.clear();
        scratch.reserve(n_ch as usize);
        for _ in 0..n_ch as usize {
            scratch.push(vec![0.0_f32; max_frames as usize]);
        }

        let latency = this.sink.latency_samples.load(Ordering::Relaxed);
        if latency > 0 && sr > 0.0 {
            this.set_latency_seconds(latency as f64 / sr);
        }

        this.initialized.store(true, Ordering::Release);
        au::noErr
    }

    unsafe extern "C" fn uninitialize(self_ptr: *mut c_void) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        if this
            .initialized
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            // SAFETY: AU forbids Uninitialize concurrent with Render.
            unsafe { this.plugin_mut() }.deactivate();
        }
        au::noErr
    }

    unsafe extern "C" fn reset(
        self_ptr: *mut c_void,
        _scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
    ) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        // SAFETY: AU calls Reset on the main thread, serialised against render.
        unsafe { this.plugin_mut() }.reset();
        au::noErr
    }

    unsafe extern "C" fn get_property_info(
        self_ptr: *mut c_void,
        id: au::AudioUnitPropertyID,
        scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
        out_data_size: *mut au::UInt32,
        out_writable: *mut au::Boolean,
    ) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };

        let respond = |size: au::UInt32, writable: bool| -> au::OSStatus {
            unsafe {
                if !out_data_size.is_null() {
                    *out_data_size = size;
                }
                if !out_writable.is_null() {
                    *out_writable = if writable { 1 } else { 0 };
                }
            }
            au::noErr
        };

        match id {
            au::kAudioUnitProperty_SampleRate
                if scope == au::kAudioUnitScope_Input
                    || scope == au::kAudioUnitScope_Output =>
            {
                respond(std::mem::size_of::<au::Float64>() as u32, true)
            }
            au::kAudioUnitProperty_StreamFormat => respond(
                std::mem::size_of::<au::AudioStreamBasicDescription>() as u32,
                true,
            ),
            au::kAudioUnitProperty_ElementCount => {
                respond(std::mem::size_of::<au::UInt32>() as u32, false)
            }
            au::kAudioUnitProperty_Latency if scope == au::kAudioUnitScope_Global => {
                respond(std::mem::size_of::<au::Float64>() as u32, false)
            }
            au::kAudioUnitProperty_TailTime if scope == au::kAudioUnitScope_Global => {
                respond(std::mem::size_of::<au::Float64>() as u32, false)
            }
            au::kAudioUnitProperty_MaximumFramesPerSlice
                if scope == au::kAudioUnitScope_Global =>
            {
                respond(std::mem::size_of::<au::UInt32>() as u32, true)
            }
            au::kAudioUnitProperty_ParameterList if scope == au::kAudioUnitScope_Global => {
                let n_params = this.params_by_id.len() as u32;
                respond(
                    n_params * std::mem::size_of::<au::AudioUnitParameterID>() as u32,
                    false,
                )
            }
            au::kAudioUnitProperty_ParameterInfo if scope == au::kAudioUnitScope_Global => {
                respond(
                    std::mem::size_of::<au::AudioUnitParameterInfo>() as u32,
                    false,
                )
            }
            au::kAudioUnitProperty_SupportedNumChannels
                if scope == au::kAudioUnitScope_Global =>
            {
                respond(std::mem::size_of::<au::AUChannelInfo>() as u32, false)
            }
            au::kAudioUnitProperty_BypassEffect if scope == au::kAudioUnitScope_Global => {
                respond(std::mem::size_of::<au::UInt32>() as u32, true)
            }
            au::kAudioUnitProperty_LastRenderError if scope == au::kAudioUnitScope_Global => {
                respond(std::mem::size_of::<au::OSStatus>() as u32, false)
            }
            au::kAudioUnitProperty_SetRenderCallback if scope == au::kAudioUnitScope_Input => {
                respond(
                    std::mem::size_of::<au::AURenderCallbackStruct>() as u32,
                    true,
                )
            }
            au::kAudioUnitProperty_InPlaceProcessing => {
                respond(std::mem::size_of::<au::UInt32>() as u32, true)
            }
            _ => au::kAudioUnitErr_InvalidProperty,
        }
    }

    unsafe extern "C" fn get_property(
        self_ptr: *mut c_void,
        id: au::AudioUnitPropertyID,
        scope: au::AudioUnitScope,
        element: au::AudioUnitElement,
        out_data: *mut c_void,
        io_data_size: *mut au::UInt32,
    ) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };

        if out_data.is_null() || io_data_size.is_null() {
            return au::kAudioUnitErr_InvalidParameter;
        }

        match id {
            au::kAudioUnitProperty_SampleRate => {
                if (unsafe { *io_data_size } as usize) < std::mem::size_of::<au::Float64>() {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                unsafe {
                    *(out_data as *mut au::Float64) = this.sample_rate();
                    *io_data_size = std::mem::size_of::<au::Float64>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_ElementCount => {
                let count: au::UInt32 = match scope {
                    au::kAudioUnitScope_Input | au::kAudioUnitScope_Output => 1,
                    au::kAudioUnitScope_Global => 1,
                    _ => 0,
                };
                unsafe {
                    *(out_data as *mut au::UInt32) = count;
                    *io_data_size = std::mem::size_of::<au::UInt32>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_Latency if scope == au::kAudioUnitScope_Global => {
                unsafe {
                    *(out_data as *mut au::Float64) = this.latency_seconds();
                    *io_data_size = std::mem::size_of::<au::Float64>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_TailTime if scope == au::kAudioUnitScope_Global => {
                unsafe {
                    *(out_data as *mut au::Float64) = 0.0;
                    *io_data_size = std::mem::size_of::<au::Float64>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_MaximumFramesPerSlice => {
                unsafe {
                    *(out_data as *mut au::UInt32) = this.max_frames_per_slice();
                    *io_data_size = std::mem::size_of::<au::UInt32>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_StreamFormat => {
                if (unsafe { *io_data_size } as usize)
                    < std::mem::size_of::<au::AudioStreamBasicDescription>()
                {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let asbd = au::AudioStreamBasicDescription {
                    mSampleRate: this.sample_rate(),
                    mFormatID: au::kAudioFormatLinearPCM,
                    mFormatFlags: au::kAudioFormatFlagIsFloat
                        | au::kAudioFormatFlagIsPacked
                        | au::kAudioFormatFlagIsNonInterleaved,
                    mBytesPerPacket: 4,
                    mFramesPerPacket: 1,
                    mBytesPerFrame: 4,
                    mChannelsPerFrame: this.n_channels(),
                    mBitsPerChannel: 32,
                    mReserved: 0,
                };
                unsafe {
                    *(out_data as *mut au::AudioStreamBasicDescription) = asbd;
                    *io_data_size =
                        std::mem::size_of::<au::AudioStreamBasicDescription>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_ParameterList if scope == au::kAudioUnitScope_Global => {
                let n = this.params_by_id.len();
                let needed = (n * std::mem::size_of::<au::AudioUnitParameterID>()) as u32;
                if unsafe { *io_data_size } < needed {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let dst = out_data as *mut au::AudioUnitParameterID;
                for i in 0..n {
                    unsafe {
                        *dst.add(i) = i as au::AudioUnitParameterID;
                    }
                }
                unsafe {
                    *io_data_size = needed;
                }
                au::noErr
            }
            au::kAudioUnitProperty_ParameterInfo if scope == au::kAudioUnitScope_Global => {
                let idx = element as usize;
                let entry = match this.params_by_id.get(idx) {
                    Some(e) => e,
                    None => return au::kAudioUnitErr_InvalidParameter,
                };
                if (unsafe { *io_data_size } as usize)
                    < std::mem::size_of::<au::AudioUnitParameterInfo>()
                {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let info = build_parameter_info(entry);
                unsafe {
                    *(out_data as *mut au::AudioUnitParameterInfo) = info;
                    *io_data_size = std::mem::size_of::<au::AudioUnitParameterInfo>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_SupportedNumChannels
                if scope == au::kAudioUnitScope_Global =>
            {
                if (unsafe { *io_data_size } as usize) < std::mem::size_of::<au::AUChannelInfo>() {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                unsafe {
                    *(out_data as *mut au::AUChannelInfo) = au::AUChannelInfo {
                        inChannels: -1,
                        outChannels: -1,
                    };
                    *io_data_size = std::mem::size_of::<au::AUChannelInfo>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_BypassEffect if scope == au::kAudioUnitScope_Global => {
                unsafe {
                    *(out_data as *mut au::UInt32) = 0;
                    *io_data_size = std::mem::size_of::<au::UInt32>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_LastRenderError if scope == au::kAudioUnitScope_Global => {
                unsafe {
                    *(out_data as *mut au::OSStatus) = au::noErr;
                    *io_data_size = std::mem::size_of::<au::OSStatus>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_InPlaceProcessing => {
                unsafe {
                    *(out_data as *mut au::UInt32) = 1;
                    *io_data_size = std::mem::size_of::<au::UInt32>() as u32;
                }
                au::noErr
            }
            _ => au::kAudioUnitErr_InvalidProperty,
        }
    }

    unsafe extern "C" fn set_property(
        self_ptr: *mut c_void,
        id: au::AudioUnitPropertyID,
        _scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
        in_data: *const c_void,
        in_data_size: au::UInt32,
    ) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };

        match id {
            au::kAudioUnitProperty_SampleRate => {
                if (in_data_size as usize) < std::mem::size_of::<au::Float64>() {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let sr = unsafe { *(in_data as *const au::Float64) };
                this.set_sample_rate(sr);
                au::noErr
            }
            au::kAudioUnitProperty_StreamFormat => {
                if (in_data_size as usize)
                    < std::mem::size_of::<au::AudioStreamBasicDescription>()
                {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let asbd = unsafe { &*(in_data as *const au::AudioStreamBasicDescription) };
                this.set_sample_rate(asbd.mSampleRate);
                this.n_channels
                    .store(asbd.mChannelsPerFrame, Ordering::Release);
                au::noErr
            }
            au::kAudioUnitProperty_MaximumFramesPerSlice => {
                if (in_data_size as usize) < std::mem::size_of::<au::UInt32>() {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let v = unsafe { *(in_data as *const au::UInt32) };
                this.max_frames_per_slice.store(v, Ordering::Release);
                au::noErr
            }
            au::kAudioUnitProperty_BypassEffect => au::noErr,
            au::kAudioUnitProperty_SetRenderCallback => {
                if (in_data_size as usize)
                    < std::mem::size_of::<au::AURenderCallbackStruct>()
                {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let cb = unsafe { *(in_data as *const au::AURenderCallbackStruct) };
                let new_cb = if cb.inputProc.is_some() { Some(cb) } else { None };
                if let Ok(mut guard) = this.input_callback.lock() {
                    *guard = new_cb;
                }
                au::noErr
            }
            au::kAudioUnitProperty_InPlaceProcessing => au::noErr,
            _ => au::kAudioUnitErr_InvalidProperty,
        }
    }

    /// Read the current parameter value, projected from the plugin's
    /// normalised `[0, 1]` representation back to the plain (display) value.
    unsafe extern "C" fn get_parameter(
        self_ptr: *mut c_void,
        id: au::AudioUnitParameterID,
        _scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
        out_value: *mut au::AudioUnitParameterValue,
    ) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        let entry = match this.params_by_id.get(id as usize) {
            Some(e) => e,
            None => return au::kAudioUnitErr_InvalidParameter,
        };
        if out_value.is_null() {
            return au::kAudioUnitErr_InvalidParameter;
        }
        let normalized = unsafe { entry.ptr.unmodulated_normalized_value() };
        let plain = unsafe { entry.ptr.preview_plain(normalized) };
        unsafe {
            *out_value = plain;
        }
        au::noErr
    }

    /// Set a parameter from the host. AU sends the plain value; convert back
    /// to the [0, 1] normalised range nih-plug expects.
    unsafe extern "C" fn set_parameter(
        self_ptr: *mut c_void,
        id: au::AudioUnitParameterID,
        _scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
        value: au::AudioUnitParameterValue,
        _buffer_offset_in_frames: au::UInt32,
    ) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        let entry = match this.params_by_id.get(id as usize) {
            Some(e) => e,
            None => return au::kAudioUnitErr_InvalidParameter,
        };
        let normalized = unsafe { entry.ptr.preview_normalized(value) };
        unsafe {
            let _ = entry.ptr.set_normalized_value(normalized);
        }
        au::noErr
    }

    /// Phase 3 render. Convert the host's `AudioBufferList` into the
    /// nih-plug `Buffer` shape and dispatch to `Plugin::process`.
    ///
    /// Concurrency: this runs on the audio thread. We only touch the wrapper
    /// through `&Self` plus controlled `UnsafeCell` borrows for the things
    /// the audio thread owns (`plugin`, `input_scratch`).
    unsafe extern "C" fn render(
        self_ptr: *mut c_void,
        _io_action_flags: *mut au::AudioUnitRenderActionFlags,
        _in_time_stamp: *const au::AudioTimeStamp,
        _in_output_bus_number: au::UInt32,
        in_number_frames: au::UInt32,
        io_data: *mut au::AudioBufferList,
    ) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };

        if io_data.is_null() {
            return au::kAudioUnitErr_InvalidParameter;
        }

        if !this.is_initialized() {
            unsafe { zero_buffer_list(io_data, in_number_frames) };
            return au::noErr;
        }

        let n_frames_us = in_number_frames as usize;

        // Snapshot the input callback under the mutex (cheap struct copy).
        // Held only for the duration of the load so main-thread updates
        // never block render for long. Note: the lock itself can technically
        // contend with main thread; this is addressed in AUD-337 via a
        // lock-free swap. For now, the only writer is SetRenderCallback
        // which is rare.
        let callback_snapshot = this
            .input_callback
            .lock()
            .ok()
            .and_then(|g| *g);

        // ── 1) Pull input from the host (callback path). ─────────────────
        // SAFETY: render is not re-entered, so the audio thread is the sole
        // owner of input_scratch right now.
        let input_scratch = unsafe { this.input_scratch_mut() };
        let mut pulled_from_callback = false;
        if let Some(cb) = callback_snapshot {
            let n_ch = input_scratch.len();
            if n_ch > 0 && input_scratch[0].len() >= n_frames_us {
                let header_size = std::mem::size_of::<au::UInt32>();
                let buf_size = std::mem::size_of::<au::AudioBuffer>();
                let bl_bytes = header_size + buf_size * n_ch;
                let mut bl_storage: Vec<u8> = vec![0u8; bl_bytes];
                let bl_ptr = bl_storage.as_mut_ptr() as *mut au::AudioBufferList;
                unsafe {
                    (*bl_ptr).mNumberBuffers = n_ch as au::UInt32;
                }
                let buffers_ptr = unsafe {
                    bl_storage.as_mut_ptr().add(header_size) as *mut au::AudioBuffer
                };
                for ch in 0..n_ch {
                    let scratch_ptr = input_scratch[ch].as_mut_ptr();
                    unsafe {
                        *buffers_ptr.add(ch) = au::AudioBuffer {
                            mNumberChannels: 1,
                            mDataByteSize: (n_frames_us * std::mem::size_of::<f32>())
                                as au::UInt32,
                            mData: scratch_ptr as *mut c_void,
                        };
                    }
                }

                let ts: au::AudioTimeStamp = unsafe { std::mem::zeroed() };
                let mut flags: au::AudioUnitRenderActionFlags = 0;
                let proc = cb.inputProc.unwrap();
                let status = unsafe {
                    proc(
                        cb.inputProcRefCon,
                        &mut flags,
                        &ts,
                        0,
                        in_number_frames,
                        bl_ptr,
                    )
                };
                if status == au::noErr {
                    pulled_from_callback = true;
                }
            }
        }

        let n_frames = n_frames_us;

        // ── 2) Build process buffer slices over io_data. ─────────────────
        let bl = unsafe { &mut *io_data };
        let n_buffers = bl.mNumberBuffers as usize;
        let buffers_ptr = bl.mBuffers.as_mut_ptr();

        // If we pulled via callback, copy scratch → io_data so process()
        // sees the input audio.
        if pulled_from_callback {
            for i in 0..n_buffers.min(input_scratch.len()) {
                let buf = unsafe { &mut *buffers_ptr.add(i) };
                if buf.mData.is_null() {
                    continue;
                }
                let dst = unsafe {
                    std::slice::from_raw_parts_mut(buf.mData as *mut f32, n_frames)
                };
                let src = &input_scratch[i][..n_frames];
                dst.copy_from_slice(src);
            }
        }

        let mut channels: Vec<&'static mut [f32]> = Vec::with_capacity(n_buffers);
        for i in 0..n_buffers {
            let buf = unsafe { &mut *buffers_ptr.add(i) };
            if buf.mData.is_null() {
                continue;
            }
            let slice = unsafe {
                std::slice::from_raw_parts_mut(buf.mData as *mut f32, n_frames)
            };
            // SAFETY: erasing the lifetime is unsound in general; tracked in
            // AUD-333. The slice never outlives this render() call and the
            // resulting Buffer is consumed locally before return.
            let static_slice: &'static mut [f32] = unsafe {
                std::mem::transmute::<&mut [f32], &'static mut [f32]>(slice)
            };
            channels.push(static_slice);
        }

        let mut buffer = Buffer::default();
        unsafe {
            buffer.set_slices(n_frames, |dst| {
                *dst = channels;
            });
        }

        let mut aux = AuxiliaryBuffers {
            inputs: &mut [],
            outputs: &mut [],
        };
        let sr = this.sample_rate();
        let mut process_ctx = AuProcessContext::<P> {
            sink: this.sink.clone(),
            transport: Transport::new(sr as f32),
            _marker: PhantomData,
        };

        // SAFETY: render is not concurrent with Initialize/Uninitialize/Reset
        // and AU does not re-enter render, so this `&mut P` is unique.
        let _status = unsafe { this.plugin_mut() }.process(&mut buffer, &mut aux, &mut process_ctx);

        let latency = this.sink.latency_samples.load(Ordering::Relaxed);
        if latency > 0 && sr > 0.0 {
            this.set_latency_seconds(latency as f64 / sr);
        }

        au::noErr
    }

    unsafe extern "C" fn add_property_listener(
        _self_ptr: *mut c_void,
        _id: au::AudioUnitPropertyID,
        _proc: au::AudioUnitPropertyListenerProc,
        _user_data: *mut c_void,
    ) -> au::OSStatus {
        au::noErr
    }

    unsafe extern "C" fn remove_property_listener_with_user_data(
        _self_ptr: *mut c_void,
        _id: au::AudioUnitPropertyID,
        _proc: au::AudioUnitPropertyListenerProc,
        _user_data: *mut c_void,
    ) -> au::OSStatus {
        au::noErr
    }
}

/// Build the `AudioUnitParameterInfo` blob the host queries for each
/// parameter. Populates the legacy 52-byte name buffer; the CFString slot is
/// left null until AUD-339 (Phase 4 CoreFoundation bridge).
fn build_parameter_info(entry: &ParamEntry) -> au::AudioUnitParameterInfo {
    let mut info: au::AudioUnitParameterInfo = unsafe { std::mem::zeroed() };

    let name_str = unsafe { entry.ptr.name() };
    let max = info.name.len() - 1;
    let bytes = name_str.as_bytes();
    let n = bytes.len().min(max);
    for i in 0..n {
        info.name[i] = bytes[i] as std::os::raw::c_char;
    }

    let normalized_default = unsafe { entry.ptr.default_normalized_value() };
    let default_plain = unsafe { entry.ptr.preview_plain(normalized_default) };
    let min_plain = unsafe { entry.ptr.preview_plain(0.0) };
    let max_plain = unsafe { entry.ptr.preview_plain(1.0) };

    info.minValue = min_plain;
    info.maxValue = max_plain;
    info.defaultValue = default_plain;

    let unit = match unsafe { entry.ptr.step_count() } {
        Some(1) => au::kAudioUnitParameterUnit_Boolean,
        Some(_) => au::kAudioUnitParameterUnit_Indexed,
        None => {
            let unit_str = unsafe { entry.ptr.unit() };
            classify_unit(unit_str)
        }
    };
    info.unit = unit;
    info.unitName = std::ptr::null_mut();
    info.cfNameString = std::ptr::null_mut();
    info.clumpID = 0;

    info.flags = au::kAudioUnitParameterFlag_IsReadable
        | au::kAudioUnitParameterFlag_IsWritable
        | au::kAudioUnitParameterFlag_CanRamp;

    info
}

fn classify_unit(unit: &str) -> au::AudioUnitParameterUnit {
    let lower = unit.to_ascii_lowercase();
    if lower.contains("db") || lower.contains("decibel") {
        au::kAudioUnitParameterUnit_Decibels
    } else if lower.contains("hz") || lower.contains("hertz") || lower == "khz" {
        au::kAudioUnitParameterUnit_Hertz
    } else if lower.contains('%') || lower.contains("percent") {
        au::kAudioUnitParameterUnit_Percent
    } else if lower.contains("ms") || lower.contains("sec") || lower.contains("second") {
        au::kAudioUnitParameterUnit_Seconds
    } else {
        au::kAudioUnitParameterUnit_Generic
    }
}

pub use Wrapper as AuWrapper;

unsafe fn zero_buffer_list(bl: *mut au::AudioBufferList, n_frames: au::UInt32) {
    if bl.is_null() {
        return;
    }
    let bl = unsafe { &mut *bl };
    let n = bl.mNumberBuffers as usize;
    let buffers_ptr = bl.mBuffers.as_mut_ptr();
    for i in 0..n {
        let buf = unsafe { &mut *buffers_ptr.add(i) };
        if buf.mData.is_null() {
            continue;
        }
        let n_samples = n_frames as usize * buf.mNumberChannels as usize;
        unsafe { ptr::write_bytes(buf.mData as *mut f32, 0, n_samples) };
    }
}
