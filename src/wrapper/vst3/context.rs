use parking_lot::RwLockWriteGuard;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use super::inner::{Task, WrapperInner};
use crate::context::{EventLoop, ProcessContext};
use crate::plugin::{NoteEvent, Plugin};

/// A [ProcessContext] implementation for the wrapper. This is a separate object so it can hold on
/// to lock guards for event queues. Otherwise reading these events would require constant
/// unnecessary atomic operations to lock the uncontested RwLocks.
pub(crate) struct WrapperProcessContext<'a, P: Plugin> {
    pub inner: &'a WrapperInner<P>,
    pub input_events_guard: RwLockWriteGuard<'a, VecDeque<NoteEvent>>,
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
