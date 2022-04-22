use super::wrapper::Wrapper;
use crate::context::{PluginApi, ProcessContext, Transport};
use crate::midi::NoteEvent;
use crate::plugin::Plugin;

/// A [`ProcessContext`] implementation for the standalone wrapper. This is a separate object so it
/// can hold on to lock guards for event queues. Otherwise reading these events would require
/// constant unnecessary atomic operations to lock the uncontested RwLocks.
pub(crate) struct WrapperProcessContext<'a, P: Plugin> {
    pub(super) wrapper: &'a Wrapper<P>,
    // TODO: Events
    // pub(super) input_events_guard: AtomicRefMut<'a, VecDeque<NoteEvent>>,
    // pub(super) output_events_guard: AtomicRefMut<'a, VecDeque<NoteEvent>>,
    pub(super) transport: Transport,
}

impl<P: Plugin> ProcessContext for WrapperProcessContext<'_, P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }

    fn transport(&self) -> &Transport {
        &self.transport
    }

    fn next_event(&mut self) -> Option<NoteEvent> {
        nih_debug_assert_failure!("TODO: WrapperProcessContext::next_event()");

        // self.input_events_guard.pop_front()
        None
    }

    fn send_event(&mut self, event: NoteEvent) {
        nih_debug_assert_failure!("TODO: WrapperProcessContext::send_event()");

        // self.output_events_guard.push_back(event);
    }

    fn set_latency_samples(&self, samples: u32) {
        nih_debug_assert_failure!("TODO: WrapperProcessContext::set_latency_samples()");
    }
}
