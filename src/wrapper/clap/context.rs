use parking_lot::RwLockWriteGuard;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use super::plugin::{Task, Wrapper};
use crate::context::ProcessContext;
use crate::event_loop::EventLoop;
use crate::plugin::{ClapPlugin, NoteEvent};

/// A [ProcessContext] implementation for the wrapper. This is a separate object so it can hold on
/// to lock guards for event queues. Otherwise reading these events would require constant
/// unnecessary atomic operations to lock the uncontested RwLocks.
pub(crate) struct WrapperProcessContext<'a, P: ClapPlugin> {
    pub(super) plugin: &'a Wrapper<P>,
    pub(super) input_events_guard: RwLockWriteGuard<'a, VecDeque<NoteEvent>>,
}

impl<P: ClapPlugin> ProcessContext for WrapperProcessContext<'_, P> {
    fn set_latency_samples(&self, samples: u32) {
        // Only make a callback if it's actually needed
        // XXX: For CLAP we could move this handling to the Plugin struct, but it may be worthwhile
        //      to keep doing it this way to stay consistent with VST3.
        let old_latency = self.plugin.current_latency.swap(samples, Ordering::SeqCst);
        if old_latency != samples {
            let task_posted = self.plugin.do_maybe_async(Task::LatencyChanged);
            nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
        }
    }

    fn next_midi_event(&mut self) -> Option<NoteEvent> {
        self.input_events_guard.pop_front()
    }
}
