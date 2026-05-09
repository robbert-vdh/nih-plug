//! AU wrapper, Phase 2.
//!
//! Hosts the plugin's `Params` so:
//!   - `kAudioUnitProperty_ParameterList` returns the AU parameter ID array
//!   - `kAudioUnitProperty_ParameterInfo` returns name / range / default / unit
//!   - `AudioUnitGet/SetParameter` hand off to the underlying nih-plug `ParamPtr`
//!
//! AU parameter IDs are simply the index into `param_map()` — stable across
//! one binary build (the nih-plug derive guarantees field declaration order).
//!
//! Render is still Phase 1 (silence). Phase 3 will wire the actual
//! `Plugin::process()` path.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use au_sys as au;

use crate::buffer::Buffer;
use crate::context::process::Transport;
use crate::params::internals::ParamPtr;
use crate::params::Params;
use crate::plugin::au::AuPlugin;
use crate::plugin::Plugin;
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

    /// Host's `AudioUnit` opaque handle, set in `Open`.
    instance: au::AudioUnit,

    /// Sample rate set via `kAudioUnitProperty_StreamFormat`.
    sample_rate: f64,

    /// Maximum frames per `AudioUnitRender` slice.
    max_frames_per_slice: u32,

    /// Channel count set via `StreamFormat`.
    n_channels: u32,

    /// Latency reported via `kAudioUnitProperty_Latency`.
    latency_seconds: f64,

    /// The plugin instance. Kept for the entire lifetime of the wrapper so
    /// the `ParamPtr` raw pointers in `params_by_id` remain valid.
    plugin: Box<P>,

    /// Parameter handles in declaration order. The AU parameter ID is the
    /// index into this vec.
    params_by_id: Vec<ParamEntry>,

    /// Strong reference back to the `Params` object referenced by every
    /// `ParamPtr` in `params_by_id`.
    _params_arc: Arc<dyn Params>,

    /// Whether `Plugin::initialize()` has run successfully since the last
    /// `Uninitialize` / first construction. Render is a no-op when false.
    initialized: bool,

    /// Scratch sink shared between Init/Process contexts. Lets the plugin
    /// report a latency change back via `set_latency_samples()`.
    sink: Arc<ContextSink>,

    /// Input render callback registered by the host via
    /// `kAudioUnitProperty_SetRenderCallback`. We invoke this at the top of
    /// every `render()` call to pull input audio for the effect.
    ///
    /// Hosts that don't use the callback model (or for in-place processing)
    /// leave this `None` and we just operate on whatever the host wrote
    /// into `io_data` ahead of the call.
    input_callback: Option<au::AURenderCallbackStruct>,

    /// Per-channel scratch buffers for the input pull. Sized in `Initialize`
    /// to `[max_frames_per_slice]` so the render path is allocation-free.
    /// One `Vec<f32>` per channel; we hand the host a synthesised
    /// `AudioBufferList` whose `mData` pointers point into these vectors.
    input_scratch: Vec<Vec<f32>>,
}

struct ParamEntry {
    /// Stable string ID — currently unused but kept for future state save/load.
    #[allow(dead_code)]
    id_str: String,
    ptr: ParamPtr,
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
            instance: ptr::null_mut(),
            sample_rate: 44_100.0,
            max_frames_per_slice: 1024,
            n_channels: 2,
            latency_seconds: 0.0,
            plugin,
            params_by_id,
            _params_arc: params_arc,
            initialized: false,
            sink: ContextSink::new(),
            input_callback: None,
            input_scratch: Vec::new(),
        });
        Box::into_raw(boxed) as *mut au::AudioComponentPlugInInterface
    }

    unsafe fn from_ptr<'a>(ptr: *mut c_void) -> &'a mut Self {
        &mut *(ptr as *mut Self)
    }

    unsafe extern "C" fn open(self_ptr: *mut c_void, instance: au::AudioUnit) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        this.instance = instance;
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

        // Pick the first matching layout from the plugin's declared options.
        // AU effects with mismatched in/out channels aren't supported in
        // Phase 3 — we just use the host's stream-format channel count for
        // both sides.
        let chans = NonZeroU32::new(this.n_channels.max(1));
        let io_layout = AudioIOLayout {
            main_input_channels: chans,
            main_output_channels: chans,
            ..AudioIOLayout::const_default()
        };
        let buffer_config = BufferConfig {
            sample_rate: this.sample_rate as f32,
            min_buffer_size: None,
            max_buffer_size: this.max_frames_per_slice,
            process_mode: ProcessMode::Realtime,
        };

        let mut ctx = AuInitContext::<P> {
            sink: this.sink.clone(),
            _marker: PhantomData,
        };
        let ok = this.plugin.initialize(&io_layout, &buffer_config, &mut ctx);
        if !ok {
            return au::kAudioUnitErr_FailedInitialization;
        }

        // `Plugin::initialize()` is always followed by `Plugin::reset()`.
        this.plugin.reset();

        // Allocate input scratch space: one Vec<f32> per channel, each
        // sized to max_frames_per_slice so the render path is allocation
        // free.
        let nch = this.n_channels.max(1) as usize;
        let cap = this.max_frames_per_slice as usize;
        this.input_scratch.clear();
        this.input_scratch.reserve(nch);
        for _ in 0..nch {
            this.input_scratch.push(vec![0.0_f32; cap]);
        }

        // Pull the latency the plugin reported (if any).
        let latency = this.sink.latency_samples.load(Ordering::Relaxed);
        if latency > 0 && this.sample_rate > 0.0 {
            this.latency_seconds = latency as f64 / this.sample_rate;
        }

        this.initialized = true;
        au::noErr
    }

    unsafe extern "C" fn uninitialize(self_ptr: *mut c_void) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        if this.initialized {
            this.plugin.deactivate();
            this.initialized = false;
        }
        au::noErr
    }

    unsafe extern "C" fn reset(
        self_ptr: *mut c_void,
        _scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
    ) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        this.plugin.reset();
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
                    *(out_data as *mut au::Float64) = this.sample_rate;
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
                    *(out_data as *mut au::Float64) = this.latency_seconds;
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
                    *(out_data as *mut au::UInt32) = this.max_frames_per_slice;
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
                    mSampleRate: this.sample_rate,
                    mFormatID: au::kAudioFormatLinearPCM,
                    mFormatFlags: au::kAudioFormatFlagIsFloat
                        | au::kAudioFormatFlagIsPacked
                        | au::kAudioFormatFlagIsNonInterleaved,
                    mBytesPerPacket: 4,
                    mFramesPerPacket: 1,
                    mBytesPerFrame: 4,
                    mChannelsPerFrame: this.n_channels,
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
                this.sample_rate = unsafe { *(in_data as *const au::Float64) };
                au::noErr
            }
            au::kAudioUnitProperty_StreamFormat => {
                if (in_data_size as usize)
                    < std::mem::size_of::<au::AudioStreamBasicDescription>()
                {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                let asbd = unsafe { &*(in_data as *const au::AudioStreamBasicDescription) };
                this.sample_rate = asbd.mSampleRate;
                this.n_channels = asbd.mChannelsPerFrame;
                au::noErr
            }
            au::kAudioUnitProperty_MaximumFramesPerSlice => {
                if (in_data_size as usize) < std::mem::size_of::<au::UInt32>() {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                this.max_frames_per_slice = unsafe { *(in_data as *const au::UInt32) };
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
                // Ignore null callbacks (host clearing the slot).
                this.input_callback = if cb.inputProc.is_some() {
                    Some(cb)
                } else {
                    None
                };
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
    /// The wrapper advertises non-interleaved float as the only stream
    /// format (`AudioStreamBasicDescription` in `kAudioUnitProperty_StreamFormat`),
    /// so each `AudioBuffer` in `io_data` corresponds to exactly one channel
    /// (`mNumberChannels == 1`) holding `in_number_frames` `f32` samples.
    /// We also advertise in-place processing, so for hosts that honour it
    /// `io_data` already contains the input audio when render is called.
    /// Hosts that disable in-place still pass output buffers — they're
    /// expected to have copied the input ahead of time.
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

        // Bail out cleanly if Initialize was never called: zero the buffers
        // and return.  This protects the plugin from being asked to process
        // before sample rate / max-frames / channel count are known.
        if !this.initialized {
            unsafe { zero_buffer_list(io_data, in_number_frames) };
            return au::noErr;
        }

        let n_frames_us = in_number_frames as usize;

        // ── 1) Pull input from the host. ─────────────────────────────────
        // If the host registered a SetRenderCallback for our input bus,
        // call it with a synthesised AudioBufferList whose buffers point
        // into `input_scratch`. After the call, `input_scratch[ch][..n_frames]`
        // holds the channel data we need to feed the plugin.
        //
        // If no callback was registered, we operate "in place" on whatever
        // the host wrote into `io_data` directly (as advertised via the
        // InPlaceProcessing property). For Ableton, Logic, Reaper this
        // path is the common one.
        let mut pulled_from_callback = false;
        if let Some(cb) = this.input_callback {
            // Build an AudioBufferList in scratch memory big enough for
            // `n_channels` buffers. The struct has a single `[AudioBuffer; 1]`
            // tail array; for >1 channel we need extra storage.
            let n_ch = this.input_scratch.len();
            if n_ch > 0 && this.input_scratch[0].len() >= n_frames_us {
                let header_size = std::mem::size_of::<au::UInt32>(); // mNumberBuffers
                let buf_size = std::mem::size_of::<au::AudioBuffer>();
                let bl_bytes = header_size + buf_size * n_ch;
                let mut bl_storage: Vec<u8> = vec![0u8; bl_bytes];
                let bl_ptr = bl_storage.as_mut_ptr() as *mut au::AudioBufferList;
                unsafe {
                    (*bl_ptr).mNumberBuffers = n_ch as au::UInt32;
                }
                let buffers_ptr = unsafe {
                    (bl_storage.as_mut_ptr().add(header_size)) as *mut au::AudioBuffer
                };
                for ch in 0..n_ch {
                    let scratch_ptr = this.input_scratch[ch].as_mut_ptr();
                    unsafe {
                        *buffers_ptr.add(ch) = au::AudioBuffer {
                            mNumberChannels: 1,
                            mDataByteSize: (n_frames_us * std::mem::size_of::<f32>()) as au::UInt32,
                            mData: scratch_ptr as *mut c_void,
                        };
                    }
                }

                // Build a default AudioTimeStamp (zero is fine for offline-style
                // pull). The host inputProc fills it; we provide a zeroed one.
                let ts: au::AudioTimeStamp = unsafe { std::mem::zeroed() };
                let mut flags: au::AudioUnitRenderActionFlags = 0;
                let proc = cb.inputProc.unwrap();
                let status = unsafe {
                    proc(
                        cb.inputProcRefCon,
                        &mut flags,
                        &ts,
                        0, // input bus number
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

        // ── 2) Build process buffer slices. ──────────────────────────────
        // Strategy: nih-plug's Plugin::process operates in place on its
        // `Buffer` slices, so we always feed it slices into the output
        // (io_data) buffers. If we pulled input via the host callback into
        // `input_scratch`, copy that into io_data first; otherwise the host
        // already provided in-place audio there.
        let bl = unsafe { &mut *io_data };
        let n_buffers = bl.mNumberBuffers as usize;
        let buffers_ptr = bl.mBuffers.as_mut_ptr();

        // If host pulled via callback, copy scratch → io_data so process() sees input.
        if pulled_from_callback {
            for i in 0..n_buffers.min(this.input_scratch.len()) {
                let buf = unsafe { &mut *buffers_ptr.add(i) };
                if buf.mData.is_null() {
                    continue;
                }
                let dst =
                    unsafe { std::slice::from_raw_parts_mut(buf.mData as *mut f32, n_frames) };
                let src = &this.input_scratch[i][..n_frames];
                dst.copy_from_slice(src);
            }
        }

        // Diagnostic: input RMS *after* the pull/copy step, gated to first 5 calls.
        unsafe {
            static mut DBG: u32 = 0;
            if DBG < 5 {
                let mut sum = 0.0_f64;
                let mut max = 0.0_f32;
                let mut cnt = 0_u64;
                for i in 0..n_buffers {
                    let buf = &*buffers_ptr.add(i);
                    if buf.mData.is_null() { continue; }
                    let s = std::slice::from_raw_parts(buf.mData as *const f32, n_frames);
                    for v in s.iter() {
                        sum += (*v as f64) * (*v as f64);
                        let a = v.abs();
                        if a > max { max = a; }
                        cnt += 1;
                    }
                }
                let rms = if cnt > 0 { (sum / cnt as f64).sqrt() } else { 0.0 };
                eprintln!(
                    "[NIH-AU] render n_frames={} buf_count={} pulled={} cb_set={} rms_in={:.6} max_in={:.6}",
                    in_number_frames, n_buffers, pulled_from_callback,
                    this.input_callback.is_some(), rms, max
                );
                DBG += 1;
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
            // SAFETY: erasing the lifetime is fine because the slice never
            // outlives this render() call — Buffer is consumed locally.
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
        let mut process_ctx = AuProcessContext::<P> {
            sink: this.sink.clone(),
            transport: Transport::new(this.sample_rate as f32),
            _marker: PhantomData,
        };

        let _status = this.plugin.process(&mut buffer, &mut aux, &mut process_ctx);

        // Pull any latency change the plugin requested during process.
        let latency = this.sink.latency_samples.load(Ordering::Relaxed);
        if latency > 0 && this.sample_rate > 0.0 {
            this.latency_seconds = latency as f64 / this.sample_rate;
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
/// left null until Phase 4 brings in the CoreFoundation bridge.
fn build_parameter_info(entry: &ParamEntry) -> au::AudioUnitParameterInfo {
    let mut info: au::AudioUnitParameterInfo = unsafe { std::mem::zeroed() };

    // Legacy 52-byte name buffer. AU tools and older hosts read this when the
    // modern CFString slot is absent.
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

    // Boolean parameters always have step_count == Some(1); enums Some(n>=2).
    // Both map to AU's `Indexed`. Floats have None — we then guess from the
    // unit string.
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

/// Heuristic mapping from nih-plug's `Param::unit()` display string to an
/// AU unit ID. Hosts use this to format the value in their generic UI.
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

unsafe impl<P: AuPlugin> Send for Wrapper<P> {}

pub use Wrapper as AuWrapper;

/// Write zeros into every non-null buffer in `bl`. Used as a safe fallback
/// when render is called before initialization has succeeded.
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
