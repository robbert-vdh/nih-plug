use std::sync::Arc;

use super::backend::Backend;
use super::wrapper::{Task, Wrapper};
use crate::prelude::{
    GuiContext, InitContext, ParamPtr, Plugin, PluginApi, PluginNoteEvent, ProcessContext,
    Transport,
};

/// An [`InitContext`] implementation for the standalone wrapper.
pub(crate) struct WrapperInitContext<'a, P: Plugin, B: Backend<P>> {
    pub(super) wrapper: &'a Wrapper<P, B>,
}

/// A [`ProcessContext`] implementation for the standalone wrapper. This is a separate object so it
/// can hold on to lock guards for event queues. Otherwise reading these events would require
/// constant unnecessary atomic operations to lock the uncontested `RwLock`s.
pub(crate) struct WrapperProcessContext<'a, P: Plugin, B: Backend<P>> {
    #[allow(dead_code)]
    pub(super) wrapper: &'a Wrapper<P, B>,
    pub(super) input_events: &'a [PluginNoteEvent<P>],
    // The current index in `input_events`, since we're not actually popping anything from a queue
    // here to keep the standalone backend implementation a bit more flexible
    pub(super) input_events_idx: usize,
    pub(super) output_events: &'a mut Vec<PluginNoteEvent<P>>,
    pub(super) transport: Transport,
}

/// A [`GuiContext`] implementation for the wrapper. This is passed to the plugin in
/// [`Editor::spawn()`][crate::prelude::Editor::spawn()] so it can interact with the rest of the plugin and
/// with the host for things like setting parameters.
pub(crate) struct WrapperGuiContext<P: Plugin, B: Backend<P>> {
    pub(super) wrapper: Arc<Wrapper<P, B>>,
    #[cfg(debug_assertions)]
    pub(super) param_gesture_checker:
        atomic_refcell::AtomicRefCell<crate::wrapper::util::context_checks::ParamGestureChecker>,
}

impl<P: Plugin, B: Backend<P>> InitContext<P> for WrapperInitContext<'_, P, B> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }

    fn execute(&self, task: P::BackgroundTask) {
        (self.wrapper.task_executor.lock())(task);
    }

    fn set_latency_samples(&self, samples: u32) {
        self.wrapper.set_latency_samples(samples)
    }

    fn set_current_voice_capacity(&self, _capacity: u32) {
        // This is only supported by CLAP
    }
}

impl<P: Plugin, B: Backend<P>> ProcessContext<P> for WrapperProcessContext<'_, P, B> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }

    fn execute_background(&self, task: P::BackgroundTask) {
        let task_posted = self.wrapper.schedule_background(Task::PluginTask(task));
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    fn execute_gui(&self, task: P::BackgroundTask) {
        let task_posted = self.wrapper.schedule_gui(Task::PluginTask(task));
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    #[inline]
    fn transport(&self) -> &Transport {
        &self.transport
    }

    fn next_event(&mut self) -> Option<PluginNoteEvent<P>> {
        // We'll pretend we're a queue, choo choo
        if self.input_events_idx < self.input_events.len() {
            let event = self.input_events[self.input_events_idx].clone();
            self.input_events_idx += 1;

            Some(event)
        } else {
            None
        }
    }

    fn send_event(&mut self, event: PluginNoteEvent<P>) {
        self.output_events.push(event);
    }

    fn set_latency_samples(&self, samples: u32) {
        self.wrapper.set_latency_samples(samples)
    }

    fn set_current_voice_capacity(&self, _capacity: u32) {
        // This is only supported by CLAP
    }
}

impl<P: Plugin, B: Backend<P>> GuiContext for WrapperGuiContext<P, B> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }

    fn request_resize(&self) -> bool {
        self.wrapper.request_resize();
        true
    }

    unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {
        // Since there's no automation being recorded here, gestures don't mean anything

        #[cfg(debug_assertions)]
        match self.wrapper.param_id_from_ptr(_param) {
            Some(param_id) => self
                .param_gesture_checker
                .borrow_mut()
                .begin_set_parameter(param_id),
            None => nih_debug_assert_failure!(
                "raw_begin_set_parameter() called with an unknown ParamPtr"
            ),
        }
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        self.wrapper.set_parameter(param, normalized);

        #[cfg(debug_assertions)]
        match self.wrapper.param_id_from_ptr(param) {
            Some(param_id) => self
                .param_gesture_checker
                .borrow_mut()
                .set_parameter(param_id),
            None => {
                nih_debug_assert_failure!("raw_set_parameter() called with an unknown ParamPtr")
            }
        }
    }

    unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {
        #[cfg(debug_assertions)]
        match self.wrapper.param_id_from_ptr(_param) {
            Some(param_id) => self
                .param_gesture_checker
                .borrow_mut()
                .end_set_parameter(param_id),
            None => {
                nih_debug_assert_failure!("raw_end_set_parameter() called with an unknown ParamPtr")
            }
        }
    }

    fn get_state(&self) -> crate::wrapper::state::PluginState {
        self.wrapper.get_state_object()
    }

    fn set_state(&self, state: crate::wrapper::state::PluginState) {
        self.wrapper.set_state_object_from_gui(state)
    }
}
