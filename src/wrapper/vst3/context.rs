use atomic_refcell::AtomicRefMut;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use vst3_sys::vst::IComponentHandler;

use super::inner::{Task, WrapperInner};
use crate::context::{GuiContext, ProcessContext};
use crate::event_loop::EventLoop;
use crate::param::internals::ParamPtr;
use crate::plugin::{NoteEvent, Vst3Plugin};

/// A [`GuiContext`] implementation for the wrapper. This is passed to the plugin in
/// [`Editor::spawn()`][crate::Editor::spawn()] so it can interact with the rest of the plugin and
/// with the host for things like setting parameters.
pub(crate) struct WrapperGuiContext<P: Vst3Plugin> {
    pub(super) inner: Arc<WrapperInner<P>>,
}

/// A [`ProcessContext`] implementation for the wrapper. This is a separate object so it can hold on
/// to lock guards for event queues. Otherwise reading these events would require constant
/// unnecessary atomic operations to lock the uncontested locks.
pub(crate) struct WrapperProcessContext<'a, P: Vst3Plugin> {
    pub(super) inner: &'a WrapperInner<P>,
    pub(super) input_events_guard: AtomicRefMut<'a, VecDeque<NoteEvent>>,
}

impl<P: Vst3Plugin> GuiContext for WrapperGuiContext<P> {
    // All of these functions are supposed to be called from the main thread, so we'll put some
    // trust in the caller and assume that this is indeed the case
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        match &*self.inner.component_handler.borrow() {
            Some(handler) => match self.inner.param_ptr_to_hash.get(&param) {
                Some(hash) => {
                    handler.begin_edit(*hash);
                }
                None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
            },
            None => nih_debug_assert_failure!("Component handler not yet set"),
        }
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        match &*self.inner.component_handler.borrow() {
            Some(handler) => match self.inner.param_ptr_to_hash.get(&param) {
                Some(hash) => {
                    // Only update the parameters manually if the host is not processing audio. If
                    // the plugin is currently processing audio, the host will pass this change back
                    // to the plugin in the audio callback. This also prevents the values from
                    // changing in the middle of the process callback, which would be unsound.
                    if !self.inner.is_processing.load(Ordering::SeqCst) {
                        self.inner.set_normalized_value_by_hash(
                            *hash,
                            normalized,
                            self.inner
                                .current_buffer_config
                                .load()
                                .map(|c| c.sample_rate),
                        );
                    }

                    handler.perform_edit(*hash, normalized as f64);
                }
                None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
            },
            None => nih_debug_assert_failure!("Component handler not yet set"),
        }
    }

    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        match &*self.inner.component_handler.borrow() {
            Some(handler) => match self.inner.param_ptr_to_hash.get(&param) {
                Some(hash) => {
                    handler.end_edit(*hash);
                }
                None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
            },
            None => nih_debug_assert_failure!("Component handler not yet set"),
        }
    }

    unsafe fn raw_default_normalized_param_value(&self, param: ParamPtr) -> f32 {
        match self.inner.param_ptr_to_hash.get(&param) {
            Some(hash) => self.inner.param_defaults_normalized[hash],
            None => {
                nih_debug_assert_failure!("Unknown parameter: {:?}", param);
                0.5
            }
        }
    }
}

impl<P: Vst3Plugin> ProcessContext for WrapperProcessContext<'_, P> {
    fn set_latency_samples(&self, samples: u32) {
        // Only trigger a restart if it's actually needed
        let old_latency = self.inner.current_latency.swap(samples, Ordering::SeqCst);
        if old_latency != samples {
            let task_posted = unsafe { self.inner.event_loop.borrow().assume_init_ref() }
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
