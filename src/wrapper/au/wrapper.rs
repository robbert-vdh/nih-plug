//! Phase 1 AU wrapper — minimum viable.
//!
//! Implements the `AudioComponentPlugInInterface` vtable so that
//! `auval -v <type> <subtype> <manufacturer>` can:
//!   1. Find the component (via the bundle's `Info.plist` + factory function).
//!   2. Open / close it (lifecycle).
//!   3. Query basic properties (sample rate, element counts, latency).
//!   4. Set the stream format (host configures channel count + rate).
//!
//! Render and parameters are not yet wired — `AudioUnitRender` returns
//! silence, `kAudioUnitProperty_ParameterList` returns 0 entries. Phase 2/3
//! will fill those in.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;
use std::sync::Arc;

use au_sys as au;

use crate::plugin::au::AuPlugin;

/// One AU plugin instance. Owned by Apple's component manager via the
/// `AudioComponentPlugInInterface` pointer returned from the factory.
///
/// The first field MUST be the vtable, since Apple's component manager
/// dispatches through `instance->vtable->Lookup(selector)` — i.e. the host
/// receives a pointer to this struct interpreted as `AudioComponentPlugInInterface*`,
/// then chases the function pointers stored at offset 0.
#[repr(C)]
pub struct Wrapper<P: AuPlugin> {
    /// Apple-required vtable. MUST be the first field.
    vtable: au::AudioComponentPlugInInterface,

    /// Reference back to the host's `AudioUnit` opaque handle, set in `Open`.
    instance: au::AudioUnit,

    /// Sample rate set via `kAudioUnitProperty_StreamFormat`. Defaults to 44100
    /// before the host explicitly sets it.
    sample_rate: f64,

    /// Maximum frames per `AudioUnitRender` slice. Hosts use this to size
    /// their internal buffers and we honour it as an upper bound.
    max_frames_per_slice: u32,

    /// Channel count in/out. Currently we mirror the input layout to output;
    /// AU effects with mismatched in/out channel counts aren't supported in
    /// Phase 1.
    n_channels: u32,

    /// Latency in seconds, reported via `kAudioUnitProperty_Latency`.
    latency_seconds: f64,

    /// Phantom marker so the type system knows about `P` even though Phase 1
    /// doesn't construct an actual `Plugin` instance yet (Phase 3 will).
    _plugin: PhantomData<P>,
}

impl<P: AuPlugin> Wrapper<P> {
    /// Allocates and returns a new instance, embedded in a `Box` and leaked
    /// for the host to manage. The host calls `Close` later and we re-`Box`
    /// to drop it.
    ///
    /// Returned pointer is `*mut AudioComponentPlugInInterface` because the
    /// vtable is the first field.
    pub fn new() -> *mut au::AudioComponentPlugInInterface {
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
            _plugin: PhantomData,
        });
        let ptr = Box::into_raw(boxed);
        // Casting to vtable pointer — the host will only ever access the first field.
        ptr as *mut au::AudioComponentPlugInInterface
    }

    /// Reconstruct `&mut Self` from the opaque `*mut c_void` pointer Apple's
    /// component manager passes through every dispatch call.
    ///
    /// SAFETY: Must only be called with a pointer originally returned by
    /// `Wrapper::new()`. The host's contract guarantees this.
    unsafe fn from_ptr<'a>(ptr: *mut c_void) -> &'a mut Self {
        &mut *(ptr as *mut Self)
    }

    // ─── Vtable: lifecycle ────────────────────────────────────────────────

    unsafe extern "C" fn open(self_ptr: *mut c_void, instance: au::AudioUnit) -> au::OSStatus {
        let this = unsafe { Self::from_ptr(self_ptr) };
        this.instance = instance;
        au::noErr
    }

    unsafe extern "C" fn close(self_ptr: *mut c_void) -> au::OSStatus {
        // Re-box so it's dropped.
        unsafe {
            let _ = Box::from_raw(self_ptr as *mut Self);
        }
        au::noErr
    }

    /// Selector dispatch table. Apple's component manager calls this once per
    /// distinct selector and caches the result, so it must return a function
    /// pointer or null for unsupported selectors.
    unsafe extern "C" fn lookup(selector: au::SInt16) -> Option<au::AudioComponentMethod> {
        // SAFETY: each branch returns a function with a different concrete
        // signature, so we transmute through `AudioComponentMethod` (which is
        // a variadic-style fn pointer) to satisfy the vtable's union-style
        // C ABI. This pattern is what Apple's own AUBase template uses.
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
            // Add property listeners: stubbed (we don't notify, but auval
            // expects the selector to exist).
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

    // ─── Vtable: AU dispatch methods ──────────────────────────────────────

    unsafe extern "C" fn initialize(_self_ptr: *mut c_void) -> au::OSStatus {
        au::noErr
    }

    unsafe extern "C" fn uninitialize(_self_ptr: *mut c_void) -> au::OSStatus {
        au::noErr
    }

    unsafe extern "C" fn reset(
        _self_ptr: *mut c_void,
        _scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
    ) -> au::OSStatus {
        au::noErr
    }

    /// Returns metadata about a property: its size and whether it can be set.
    /// `auval` calls this for almost every property to probe what the unit
    /// supports.
    unsafe extern "C" fn get_property_info(
        self_ptr: *mut c_void,
        id: au::AudioUnitPropertyID,
        scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
        out_data_size: *mut au::UInt32,
        out_writable: *mut au::Boolean,
    ) -> au::OSStatus {
        let _this = unsafe { Self::from_ptr(self_ptr) };

        // Helper to set the two output values when both pointers are present.
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
            au::kAudioUnitProperty_ParameterList => {
                // 0 parameters in Phase 1 — return 0 size.
                respond(0, false)
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
                respond(std::mem::size_of::<au::AURenderCallbackStruct>() as u32, true)
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
        _element: au::AudioUnitElement,
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
                    *io_data_size = std::mem::size_of::<au::AudioStreamBasicDescription>() as u32;
                }
                au::noErr
            }
            au::kAudioUnitProperty_SupportedNumChannels
                if scope == au::kAudioUnitScope_Global =>
            {
                if (unsafe { *io_data_size } as usize) < std::mem::size_of::<au::AUChannelInfo>() {
                    return au::kAudioUnitErr_InvalidPropertyValue;
                }
                // -1 / -1 means "any matching in/out channel count" — i.e. mono and stereo both work.
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
            au::kAudioUnitProperty_SetRenderCallback => au::noErr,
            au::kAudioUnitProperty_InPlaceProcessing => au::noErr,
            _ => au::kAudioUnitErr_InvalidProperty,
        }
    }

    unsafe extern "C" fn get_parameter(
        _self_ptr: *mut c_void,
        _id: au::AudioUnitParameterID,
        _scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
        _out_value: *mut au::AudioUnitParameterValue,
    ) -> au::OSStatus {
        // Phase 1: no parameters.
        au::kAudioUnitErr_InvalidParameter
    }

    unsafe extern "C" fn set_parameter(
        _self_ptr: *mut c_void,
        _id: au::AudioUnitParameterID,
        _scope: au::AudioUnitScope,
        _element: au::AudioUnitElement,
        _value: au::AudioUnitParameterValue,
        _buffer_offset_in_frames: au::UInt32,
    ) -> au::OSStatus {
        au::kAudioUnitErr_InvalidParameter
    }

    /// Phase 1 render: silence. Phase 3 will hook in the actual `Plugin::process`.
    unsafe extern "C" fn render(
        _self_ptr: *mut c_void,
        _io_action_flags: *mut au::AudioUnitRenderActionFlags,
        _in_time_stamp: *const au::AudioTimeStamp,
        _in_output_bus_number: au::UInt32,
        in_number_frames: au::UInt32,
        io_data: *mut au::AudioBufferList,
    ) -> au::OSStatus {
        if io_data.is_null() {
            return au::kAudioUnitErr_InvalidParameter;
        }
        unsafe {
            let bl = &*io_data;
            let n_buffers = bl.mNumberBuffers as usize;
            let buffers = bl.mBuffers.as_ptr();
            for i in 0..n_buffers {
                let buf = &*buffers.add(i);
                if buf.mData.is_null() {
                    continue;
                }
                let n_samples = in_number_frames as usize * buf.mNumberChannels as usize;
                ptr::write_bytes(buf.mData as *mut f32, 0, n_samples);
            }
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

/// Marker so `Wrapper<P>` is `Send` — Apple's host calls vtable methods from
/// arbitrary threads but only one thread at a time per instance.
unsafe impl<P: AuPlugin> Send for Wrapper<P> {}

/// Public re-exports for the `nih_export_au!` macro.
pub use Wrapper as AuWrapper;

/// Hold a strong reference to the plugin metadata so the macro can build a
/// `static PLUGIN_INFO` and thread it through the factory function.
#[allow(dead_code)]
pub struct PluginRef<P: AuPlugin>(Arc<()>, PhantomData<P>);
