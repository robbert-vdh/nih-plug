//! Minimal `InitContext` / `ProcessContext` impls for the AU wrapper.
//!
//! Phase 3 supplies just enough plumbing for `Plugin::initialize()` and
//! `Plugin::process()` to be called. Background tasks, MIDI events, voice
//! capacity, and latency feedback are stubbed — the AU wrapper does not yet
//! propagate them upstream. They become real in later phases as they are
//! needed by de-leak-rt or other plugins.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::context::init::InitContext;
use crate::context::process::{ProcessContext, Transport};
use crate::context::PluginApi;
use crate::prelude::PluginNoteEvent;
use crate::plugin::Plugin;

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
        // No background executor in Phase 3.
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
