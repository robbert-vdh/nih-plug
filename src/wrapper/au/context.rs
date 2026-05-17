//! Minimal `InitContext` / `ProcessContext` / `GuiContext` impls for the AU wrapper.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use au_sys as au;

use crate::context::gui::GuiContext;
use crate::context::init::InitContext;
use crate::context::process::{ProcessContext, Transport};
use crate::context::PluginApi;
use crate::params::internals::ParamPtr;
use crate::prelude::PluginNoteEvent;
use crate::plugin::Plugin;
use crate::wrapper::state::PluginState;

/// Cell shared between the wrapper and its `InitContext` / `ProcessContext`
/// so that calls like `set_latency_samples()` can stash a value the wrapper
/// reads after `initialize()` / `process()` returns.
pub(super) struct ContextSink {
    pub latency_samples: AtomicU32,
}

impl ContextSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            latency_samples: AtomicU32::new(0),
        })
    }
}

pub(super) struct AuInitContext<P: Plugin> {
    pub sink: Arc<ContextSink>,
    pub _marker: std::marker::PhantomData<P>,
}

impl<P: Plugin> InitContext<P> for AuInitContext<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Au
    }

    fn execute(&self, _task: P::BackgroundTask) {
        // No background executor yet.
    }

    fn set_latency_samples(&self, samples: u32) {
        self.sink.latency_samples.store(samples, Ordering::Relaxed);
    }

    fn set_current_voice_capacity(&self, _capacity: u32) {
        // CLAP-only.
    }
}

pub(super) struct AuProcessContext<P: Plugin> {
    pub sink: Arc<ContextSink>,
    pub transport: Transport,
    pub _marker: std::marker::PhantomData<P>,
}

impl<P: Plugin> ProcessContext<P> for AuProcessContext<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Au
    }

    fn execute_background(&self, _task: P::BackgroundTask) {}

    fn execute_gui(&self, _task: P::BackgroundTask) {}

    fn transport(&self) -> &Transport {
        &self.transport
    }

    fn next_event(&mut self) -> Option<PluginNoteEvent<P>> {
        None
    }

    fn send_event(&mut self, _event: PluginNoteEvent<P>) {}

    fn set_latency_samples(&self, samples: u32) {
        self.sink.latency_samples.store(samples, Ordering::Relaxed);
    }

    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

// ─── GuiContext ───────────────────────────────────────────────────────────────

/// Payload shared between `Wrapper` and `AuGuiContext` via `Arc`.
pub(super) struct AuGuiContextInner {
    /// Host's `AudioUnit` opaque handle, stored as raw bits so we can update
    /// it from `open()` without requiring `&mut`. Set once in `open()`.
    pub instance_bits: std::sync::atomic::AtomicU64,
    /// `params_arc` from the plugin — needed for get/set_state.
    pub params_arc: Arc<dyn crate::params::Params>,
    /// (ParamPtr → AU param ID) lookup. Index in `params_by_id` == AU param ID.
    pub params_by_ptr: Vec<(ParamPtr, au::AudioUnitParameterID)>,
}

unsafe impl Send for AuGuiContextInner {}
unsafe impl Sync for AuGuiContextInner {}

pub(super) struct AuGuiContext<P: Plugin> {
    pub inner: Arc<AuGuiContextInner>,
    pub _marker: std::marker::PhantomData<P>,
}

// SAFETY: AuGuiContext holds no P data; only Arc<AuGuiContextInner> which is Send+Sync.
unsafe impl<P: Plugin> Send for AuGuiContext<P> {}
unsafe impl<P: Plugin> Sync for AuGuiContext<P> {}

impl<P: Plugin> GuiContext for AuGuiContext<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Au
    }

    fn request_resize(&self) -> bool {
        // AUv2 has no host-resize API.
        false
    }

    unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {
        // AUv2 has no begin-gesture notification.
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        // Write the value directly (same path as AU SetParameter from the host).
        unsafe { param.set_normalized_value(normalized) };

        // Notify host listeners via AUParameterListenerNotify so automation
        // records the GUI-driven change.
        if let Some(&param_id) = self
            .inner
            .params_by_ptr
            .iter()
            .find(|(p, _)| *p == param)
            .map(|(_, id)| id)
        {
            let instance = self.inner.instance_bits.load(std::sync::atomic::Ordering::Acquire)
                as usize as au::AudioUnit;
            let au_param = AUParameter {
                mAudioUnit: instance,
                mParameterID: param_id,
                mScope: au::kAudioUnitScope_Global,
                mElement: 0,
            };
            // SAFETY: AUParameterListenerNotify is safe from any thread.
            unsafe { AUParameterListenerNotify(std::ptr::null_mut(), std::ptr::null_mut(), &au_param) };
        }
    }

    unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {
        // AUv2 has no end-gesture notification.
    }

    fn get_state(&self) -> PluginState {
        let params_arc = self.inner.params_arc.clone();
        let param_map = params_arc.param_map();
        unsafe {
            crate::wrapper::state::serialize_object::<P>(
                params_arc.clone(),
                param_map
                    .iter()
                    .map(|(id_str, ptr, _group)| (id_str, *ptr)),
            )
        }
    }

    fn set_state(&self, mut state: PluginState) {
        let params_arc = self.inner.params_arc.clone();
        let param_map = params_arc.param_map();
        let getter = move |id: &str| {
            param_map
                .iter()
                .find(|(id_str, _, _)| id_str.as_str() == id)
                .map(|(_, ptr, _)| *ptr)
        };
        unsafe {
            crate::wrapper::state::deserialize_object::<P>(
                &mut state,
                params_arc,
                getter,
                None,
            );
        }
    }
}

/// `AudioUnitParameter` — used by `AUParameterListenerNotify`.
#[repr(C)]
#[allow(non_snake_case)]
pub(super) struct AUParameter {
    pub mAudioUnit: au::AudioUnit,
    pub mParameterID: au::AudioUnitParameterID,
    pub mScope: au::AudioUnitScope,
    pub mElement: au::AudioUnitElement,
}

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    /// Notifies all listeners registered for the given parameter.
    /// Declared here because `au-sys` does not expose this AudioToolbox API.
    pub(super) fn AUParameterListenerNotify(
        inSendingListener: *mut std::ffi::c_void,
        inSendingObject: *mut std::ffi::c_void,
        inParameter: *const AUParameter,
    );
}
