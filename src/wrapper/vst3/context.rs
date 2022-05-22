use atomic_refcell::AtomicRefMut;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use vst3_sys::vst::{IComponentHandler, RestartFlags};

use super::inner::{Task, WrapperInner};
use crate::context::{GuiContext, PluginApi, ProcessContext, Transport};
use crate::midi::NoteEvent;
use crate::param::internals::ParamPtr;
use crate::plugin::Vst3Plugin;
use crate::wrapper::state::PluginState;

/// A [`GuiContext`] implementation for the wrapper. This is passed to the plugin in
/// [`Editor::spawn()`][crate::prelude::Editor::spawn()] so it can interact with the rest of the plugin and
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
    pub(super) output_events_guard: AtomicRefMut<'a, VecDeque<NoteEvent>>,
    pub(super) transport: Transport,
}

impl<P: Vst3Plugin> GuiContext for WrapperGuiContext<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Vst3
    }

    fn request_resize(&self) -> bool {
        let task_posted = self.inner.do_maybe_async(Task::RequestResize);
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");

        // TODO: We don't handle resize request failures right now. In practice this should however
        //       not happen.
        true
    }

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
                    // FIXME: So this doesn't work for REAPER, because they just silently stop
                    //        processing audio when you bypass the plugin. Great. We can add a time
                    //        based heuristic to work aorund this in the meantime.
                    if !self.inner.is_processing.load(Ordering::SeqCst) {
                        self.inner.set_normalized_value_by_hash(
                            *hash,
                            normalized,
                            self.inner
                                .current_buffer_config
                                .load()
                                .map(|c| c.sample_rate),
                        );
                        self.inner.notify_param_values_changed();
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

    fn get_state(&self) -> PluginState {
        self.inner.get_state_object()
    }

    fn set_state(&self, state: PluginState) {
        self.inner.set_state_object(state)
    }
}

impl<P: Vst3Plugin> ProcessContext for WrapperProcessContext<'_, P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Vst3
    }

    fn transport(&self) -> &Transport {
        &self.transport
    }

    fn next_event(&mut self) -> Option<NoteEvent> {
        self.input_events_guard.pop_front()
    }

    fn send_event(&mut self, event: NoteEvent) {
        self.output_events_guard.push_back(event);
    }

    fn set_latency_samples(&self, samples: u32) {
        // Only trigger a restart if it's actually needed
        let old_latency = self.inner.current_latency.swap(samples, Ordering::SeqCst);
        if old_latency != samples {
            let task_posted = self
                .inner
                .do_maybe_async(Task::TriggerRestart(RestartFlags::kLatencyChanged as i32));
            nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
        }
    }
}
