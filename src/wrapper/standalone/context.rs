use crossbeam::channel;
use std::sync::Arc;

use super::backend::Backend;
use super::wrapper::{GuiTask, Task, Wrapper};
use crate::context::gui::GuiContext;
use crate::context::init::InitContext;
use crate::context::process::{ProcessContext, Transport};
use crate::context::PluginApi;
use crate::midi::NoteEvent;
use crate::params::internals::ParamPtr;
use crate::plugin::Plugin;

/// An [`InitContext`] implementation for the standalone wrapper.
pub(crate) struct WrapperInitContext<'a, P: Plugin, B: Backend> {
    pub(super) wrapper: &'a Wrapper<P, B>,
}

/// A [`ProcessContext`] implementation for the standalone wrapper. This is a separate object so it
/// can hold on to lock guards for event queues. Otherwise reading these events would require
/// constant unnecessary atomic operations to lock the uncontested RwLocks.
pub(crate) struct WrapperProcessContext<'a, P: Plugin, B: Backend> {
    #[allow(dead_code)]
    pub(super) wrapper: &'a Wrapper<P, B>,
    pub(super) input_events: &'a [NoteEvent],
    // The current index in `input_events`, since we're not actually popping anything from a queue
    // here to keep the standalone backend implementation a bit more flexible
    pub(super) input_events_idx: usize,
    pub(super) output_events: &'a mut Vec<NoteEvent>,
    pub(super) transport: Transport,
}

/// A [`GuiContext`] implementation for the wrapper. This is passed to the plugin in
/// [`Editor::spawn()`][crate::prelude::Editor::spawn()] so it can interact with the rest of the plugin and
/// with the host for things like setting parameters.
pub(crate) struct WrapperGuiContext<P: Plugin, B: Backend> {
    pub(super) wrapper: Arc<Wrapper<P, B>>,

    /// This allows us to send tasks to the parent view that will be handled at the start of its
    /// next frame.
    pub(super) gui_task_sender: channel::Sender<GuiTask>,
}

impl<P: Plugin, B: Backend> InitContext<P> for WrapperInitContext<'_, P, B> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }

    fn execute(&self, task: P::BackgroundTask) {
        (self.wrapper.task_executor.lock())(task);
    }

    fn set_latency_samples(&self, _samples: u32) {
        nih_debug_assert_failure!("TODO: WrapperInitContext::set_latency_samples()");
    }

    fn set_current_voice_capacity(&self, _capacity: u32) {
        // This is only supported by CLAP
    }
}

impl<P: Plugin, B: Backend> ProcessContext<P> for WrapperProcessContext<'_, P, B> {
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

    fn next_event(&mut self) -> Option<NoteEvent> {
        // We'll pretend we're a queue, choo choo
        if self.input_events_idx < self.input_events.len() {
            let event = self.input_events[self.input_events_idx];
            self.input_events_idx += 1;

            Some(event)
        } else {
            None
        }
    }

    fn send_event(&mut self, event: NoteEvent) {
        self.output_events.push(event);
    }

    fn set_latency_samples(&self, _samples: u32) {
        nih_debug_assert_failure!("TODO: WrapperProcessContext::set_latency_samples()");
    }

    fn set_current_voice_capacity(&self, _capacity: u32) {
        // This is only supported by CLAP
    }
}

impl<P: Plugin, B: Backend> GuiContext for WrapperGuiContext<P, B> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }

    fn request_resize(&self) -> bool {
        let (unscaled_width, unscaled_height) =
            self.wrapper.editor.borrow().as_ref().unwrap().lock().size();

        // This will cause the editor to be resized at the start of the next frame
        let push_successful = self
            .gui_task_sender
            .send(GuiTask::Resize(unscaled_width, unscaled_height))
            .is_ok();
        nih_debug_assert!(push_successful, "Could not queue window resize");

        true
    }

    unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {
        // Since there's no automation being recorded here, gestures don't mean anything
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        self.wrapper.set_parameter(param, normalized);
    }

    unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}

    fn get_state(&self) -> crate::wrapper::state::PluginState {
        self.wrapper.get_state_object()
    }

    fn set_state(&self, state: crate::wrapper::state::PluginState) {
        self.wrapper.set_state_object(state)
    }
}
