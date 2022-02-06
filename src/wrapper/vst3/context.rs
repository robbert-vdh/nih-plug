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
