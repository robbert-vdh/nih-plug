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
use std::mem;
use std::num::NonZeroU32;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use au_sys as au;

use crate::buffer::Buffer;
use crate::context::process::Transport;
use crate::params::internals::ParamPtr;
use crate::params::{ParamFlags, Params};
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

    /// Host-controlled bypass (`kAudioUnitProperty_BypassEffect`). When set,
    /// `render()` skips `Plugin::process()` and just passes input → output.
    /// If the plugin declares a `ParamFlags::BYPASS` parameter we keep it in
    /// sync so plugins that observe bypass state through their own param see
    /// the toggle too.
    bypass: AtomicBool,

    /// Index of the BYPASS-flagged param in `params_by_id`, if any.
    bypass_param_idx: Option<usize>,

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

    /// Audio-thread-owned render state: input scratch, BufferList scratch,
    /// and the persistent `Buffer<'static>` whose channel slot vector is
    /// pre-sized in `Initialize`.
    ///
    /// `UnsafeCell` because only `render()` touches it after `Initialize`,
    /// and AU does not re-enter render.
    render_state: UnsafeCell<RenderState>,
}

/// All audio-thread mutable state. Reused across render calls and grown
/// only inside `Initialize` (main thread, before render is allowed). The
/// render hot path only writes existing slots — no allocation.
struct RenderState {
    /// Per-channel scratch for input pulled via the host's render callback.
    /// One inner vec per channel, each pre-sized to `max_frames_per_slice`.
    input_scratch: Vec<Vec<f32>>,

    /// Backing storage for the synthesised `AudioBufferList` we hand to
    /// the host's input callback. Sized once in `Initialize` to fit the
    /// header + N `AudioBuffer` entries with correct alignment.
    /// Stored as `Vec<u64>` to guarantee 8-byte alignment, which matches
    /// `AudioBuffer`'s layout (one `u32` + one `u32` + one `*mut c_void`).
    bl_storage: Vec<u64>,

    /// Persistent `Buffer` whose `output_slices` vector has its capacity
    /// pre-grown to the channel count. Each render call rewrites the
    /// existing slots in place — no `Vec::push`/`with_capacity`.
    ///
    /// The `'static` lifetime parameter is a placeholder; the slices we
    /// stash inside live only as long as the surrounding `render()` call,
    /// and we always clear them before returning.
    buffer: Buffer<'static>,
}

impl RenderState {
    fn new() -> Self {
        Self {
            input_scratch: Vec::new(),
            bl_storage: Vec::new(),
            buffer: Buffer::default(),
        }
    }

    /// Provision storage for `n_channels` channels and `max_frames`
    /// frames per slice. Called from `Initialize` only.
    fn provision(&mut self, n_channels: usize, max_frames: usize) {
        self.input_scratch.clear();
        self.input_scratch.reserve_exact(n_channels);
        for _ in 0..n_channels {
            self.input_scratch.push(vec![0.0_f32; max_frames]);
        }

        // Layout the BufferList: header + N AudioBuffers, with the N-array
        // starting at the natural offset of `mBuffers` (8-byte aligned).
        let bl_bytes = bl_byte_size(n_channels);
        // Round up to u64 count.
        let words = bl_bytes.div_ceil(mem::size_of::<u64>());
        self.bl_storage.clear();
        self.bl_storage.resize(words, 0);

        // Pre-size the channel slot vector so render only rewrites slots.
        // SAFETY: empty slices carry no provenance; rewriting them in
        // render with valid host pointers is the same pattern used by
        // BufferManager::for_audio_io_layout.
        unsafe {
            self.buffer.set_slices(0, |slices| {
                slices.clear();
                slices.reserve_exact(n_channels);
                for _ in 0..n_channels {
                    slices.push(&mut []);
                }
            });
        }
    }
}

/// Compute the exact byte size of an `AudioBufferList` with `n` buffers,
/// honoring the offset of `mBuffers` (which the C compiler inserts padding
/// for so the array is correctly aligned).
#[inline]
fn bl_byte_size(n: usize) -> usize {
    // The platform's `AudioBufferList` declares `mBuffers: [AudioBuffer; 1]`.
    // Its `offset_of(mBuffers)` is the correct start of the array on this
    // ABI — typically 8 on x86_64/arm64 (4 bytes for `mNumberBuffers` + 4
    // bytes of padding before the 8-byte-aligned `AudioBuffer`).
    let header_offset = mem::offset_of!(au::AudioBufferList, mBuffers);
    header_offset + n * mem::size_of::<au::AudioBuffer>()
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

        // Find the BYPASS-flagged param, if the plugin declares one.
        let bypass_param_idx = params_by_id.iter().position(|e| {
            // SAFETY: ParamPtr accessors are documented thread-safe.
            unsafe { e.ptr.flags() }.contains(ParamFlags::BYPASS)
        });

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
            bypass: AtomicBool::new(false),
            bypass_param_idx,
            plugin: UnsafeCell::new(plugin),
            params_by_id,
            _params_arc: params_arc,
            sink: ContextSink::new(),
            input_callback: Mutex::new(None),
            render_state: UnsafeCell::new(RenderState::new()),
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

    /// SAFETY: same as `plugin_mut` — only call from `render()` or from
    /// `initialize()` (host serialises against render). Returns the
    /// `RenderState` for in-place mutation by the audio thread.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn render_state_mut(&self) -> &mut RenderState {
        unsafe { &mut *self.render_state.get() }
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

        // Provision the audio-thread render state so the hot path is
        // allocation-free. SAFETY: render is serialised vs. Initialize.
        let render_state = unsafe { this.render_state_mut() };
        render_state.provision(n_ch as usize, max_frames as usize);

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
                // Report the first declared layout's main channel count.
                // If multiple layouts are declared we currently only expose
                // one entry; auval accepts this as a conservative answer.
                let info = match P::AUDIO_IO_LAYOUTS.iter().next() {
                    Some(layout) => {
                        let in_ch = layout
                            .main_input_channels
                            .map(|n| n.get() as i16)
                            .unwrap_or(0);
                        let out_ch = layout
                            .main_output_channels
                            .map(|n| n.get() as i16)
                            .unwrap_or(0);
                        au::AUChannelInfo {
                            inChannels: in_ch,
                            outChannels: out_ch,
                        }
                    }
                    None => au::AUChannelInfo {
                        inChannels: -1,
                        outChannels: -1,
                    },
                };
                unsafe {
                    *(out_data as *mut au::AUChannelInfo) = info;
                    *io_data_size = std::mem::size_of::<au::AUChannelInfo>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_BypassEffect if scope == au::kAudioUnitScope_Global => {
                let on = if this.bypass.load(Ordering::Acquire) { 1 } else { 0 };
                unsafe {
                    *(out_data as *mut au::UInt32) = on;
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
                if this.is_initialized() {
                    return au::kAudioUnitErr_Initialized;
                }
                let sr = unsafe { *(in_data as *const au::Float64) };
                if !(sr > 0.0 && sr.is_finite()) {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                this.set_sample_rate(sr);
                au::noErr
            }
            au::kAudioUnitProperty_StreamFormat => {
                if (in_data_size as usize)
                    < std::mem::size_of::<au::AudioStreamBasicDescription>()
                {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                // AU spec: StreamFormat may not change while initialized.
                if this.is_initialized() {
                    return au::kAudioUnitErr_Initialized;
                }
                let asbd = unsafe { &*(in_data as *const au::AudioStreamBasicDescription) };

                // Reject malformed channel count.
                if asbd.mChannelsPerFrame == 0 {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                // Reject non-PCM / non-float / interleaved — we only ever
                // advertise non-interleaved 32-bit float in get_property.
                if asbd.mFormatID != au::kAudioFormatLinearPCM
                    || (asbd.mFormatFlags & au::kAudioFormatFlagIsFloat) == 0
                    || (asbd.mFormatFlags & au::kAudioFormatFlagIsNonInterleaved) == 0
                {
                    return au::kAudioUnitErr_FormatNotSupported;
                }
                if asbd.mBitsPerChannel != 32 {
                    return au::kAudioUnitErr_FormatNotSupported;
                }
                // Match the requested channel count against P::AUDIO_IO_LAYOUTS.
                // For an effect (in_ch == out_ch) we look for a layout where
                // both main_input and main_output match.
                let req_ch = asbd.mChannelsPerFrame;
                if !layout_supports::<P>(req_ch) {
                    return au::kAudioUnitErr_FormatNotSupported;
                }

                this.set_sample_rate(asbd.mSampleRate);
                this.n_channels.store(req_ch, Ordering::Release);
                au::noErr
            }
            au::kAudioUnitProperty_MaximumFramesPerSlice => {
                if (in_data_size as usize) < std::mem::size_of::<au::UInt32>() {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                if this.is_initialized() {
                    return au::kAudioUnitErr_Initialized;
                }
                let v = unsafe { *(in_data as *const au::UInt32) };
                if v == 0 {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                this.max_frames_per_slice.store(v, Ordering::Release);
                au::noErr
            }
            au::kAudioUnitProperty_BypassEffect => {
                if (in_data_size as usize) < std::mem::size_of::<au::UInt32>() {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let v = unsafe { *(in_data as *const au::UInt32) };
                let on = v != 0;
                this.bypass.store(on, Ordering::Release);
                // Mirror into the plugin's BYPASS-flagged param if present,
                // so plugins can observe the toggle through their own params.
                if let Some(idx) = this.bypass_param_idx {
                    if let Some(entry) = this.params_by_id.get(idx) {
                        // Boolean params use 0.0 / 1.0 normalised. set_normalized_value
                        // is documented thread-safe.
                        let n = if on { 1.0 } else { 0.0 };
                        unsafe {
                            let _ = entry.ptr.set_normalized_value(n);
                        }
                    }
                }
                au::noErr
            }
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

    /// Render hot path. Convert the host's `AudioBufferList` into the
    /// nih-plug `Buffer` shape and dispatch to `Plugin::process`.
    ///
    /// Concurrency: runs on the audio thread. The wrapper is borrowed as
    /// `&Self`; mutable access to the plugin and to the render scratch goes
    /// through `UnsafeCell` borrows that are unique by AU's host contract
    /// (no render re-entry, no concurrent Initialize/Uninitialize/Reset).
    ///
    /// Allocation: this function performs no heap allocations on the hot
    /// path. All scratch storage is provisioned in `Initialize`.
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

        let n_frames = in_number_frames as usize;

        // Snapshot the input callback under the mutex (cheap struct copy).
        // Released immediately so main-thread updates don't block render
        // for long. Lock-free swap is tracked in AUD-337.
        let callback_snapshot = this.input_callback.lock().ok().and_then(|g| *g);

        // SAFETY: render is not re-entered; we are the sole owner of
        // RenderState for the duration of this call.
        let rs = unsafe { this.render_state_mut() };

        // ── 1) Pull input via host render callback (if registered). ──────
        let mut pulled_from_callback = false;
        if let Some(cb) = callback_snapshot {
            let n_ch = rs.input_scratch.len();
            // Verify our pre-allocated BufferList scratch is big enough.
            // Should always hold since `provision` sized it for n_ch and
            // n_ch is fixed between Initialize calls.
            if n_ch > 0
                && rs.input_scratch[0].len() >= n_frames
                && rs.bl_storage.len() * mem::size_of::<u64>() >= bl_byte_size(n_ch)
            {
                let bl_ptr = rs.bl_storage.as_mut_ptr() as *mut au::AudioBufferList;
                unsafe {
                    (*bl_ptr).mNumberBuffers = n_ch as au::UInt32;
                }
                // The N-tuple of AudioBuffer entries lives at the natural
                // C `mBuffers` offset, which the compiler computes
                // accounting for any padding after `mNumberBuffers`.
                let header_offset =
                    mem::offset_of!(au::AudioBufferList, mBuffers);
                let buffers_ptr = unsafe {
                    (rs.bl_storage.as_mut_ptr() as *mut u8).add(header_offset)
                        as *mut au::AudioBuffer
                };
                for ch in 0..n_ch {
                    let scratch_ptr = rs.input_scratch[ch].as_mut_ptr();
                    unsafe {
                        *buffers_ptr.add(ch) = au::AudioBuffer {
                            mNumberChannels: 1,
                            mDataByteSize: (n_frames * mem::size_of::<f32>())
                                as au::UInt32,
                            mData: scratch_ptr as *mut c_void,
                        };
                    }
                }

                let ts: au::AudioTimeStamp = unsafe { mem::zeroed() };
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

        // ── 2) Wire host io_data slices into the persistent Buffer. ──────
        let bl = unsafe { &mut *io_data };
        let n_buffers = bl.mNumberBuffers as usize;
        let buffers_ptr = bl.mBuffers.as_mut_ptr();

        // If we pulled via callback, copy scratch → io_data so process()
        // sees the input audio.
        if pulled_from_callback {
            for i in 0..n_buffers.min(rs.input_scratch.len()) {
                let buf = unsafe { &mut *buffers_ptr.add(i) };
                if buf.mData.is_null() {
                    continue;
                }
                let dst = unsafe {
                    std::slice::from_raw_parts_mut(buf.mData as *mut f32, n_frames)
                };
                let src = &rs.input_scratch[i][..n_frames];
                dst.copy_from_slice(src);
            }
        }

        // Rewrite the persistent Buffer's slot vector. The slot count was
        // pre-grown in `provision()`, so this is purely in-place writes.
        // SAFETY: each slice points to host-owned memory that remains valid
        // for the duration of this render() call. The slices are cleared
        // back to `&mut []` at the end of this function so the `'static`
        // lifetime in the slot type can never outlive the host data.
        unsafe {
            rs.buffer.set_slices(n_frames, |slots| {
                let n = slots.len().min(n_buffers);
                for i in 0..n {
                    let buf = &mut *buffers_ptr.add(i);
                    if buf.mData.is_null() {
                        slots[i] = &mut [];
                        continue;
                    }
                    let raw = std::slice::from_raw_parts_mut(
                        buf.mData as *mut f32,
                        n_frames,
                    );
                    // The slot type carries `'static` because `RenderState`
                    // is itself field-stored; the slice we put here lives
                    // only until we clear it below. This mirrors the
                    // pattern in `BufferManager::create_buffers`.
                    slots[i] = mem::transmute::<&mut [f32], &'static mut [f32]>(raw);
                }
                // Defensively null any extra slots.
                for slot in &mut slots[n..] {
                    *slot = &mut [];
                }
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

        // Bypass: skip Plugin::process entirely. Input has already been
        // copied into io_data above (callback path) or sits there in-place
        // (host path), so the pass-through is implicit — we just don't run
        // the plugin's DSP.
        if !this.bypass.load(Ordering::Acquire) {
            // SAFETY: render is not concurrent with Initialize/Uninitialize/Reset
            // and AU does not re-enter render, so this `&mut P` is unique.
            let _status = unsafe { this.plugin_mut() }.process(
                &mut rs.buffer,
                &mut aux,
                &mut process_ctx,
            );
        }

        // Clear the slot slices so the `'static` lifetime can never escape
        // this render call via a stale `&mut [f32]`.
        unsafe {
            rs.buffer.set_slices(0, |slots| {
                for slot in slots.iter_mut() {
                    *slot = &mut [];
                }
            });
        }

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

/// True if the plugin advertises an `AudioIOLayout` whose main I/O matches
/// `req_ch`. We require both `main_input_channels` and `main_output_channels`
/// to match because AU effects use a single channel count for both sides.
fn layout_supports<P: AuPlugin>(req_ch: u32) -> bool {
    let req = match NonZeroU32::new(req_ch) {
        Some(n) => n,
        None => return false,
    };
    P::AUDIO_IO_LAYOUTS.iter().any(|layout| {
        layout.main_input_channels == Some(req)
            && layout.main_output_channels == Some(req)
    })
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
