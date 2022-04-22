use parking_lot::Mutex;
use std::sync::Arc;

use super::wrapper::Wrapper;
use crate::context::{GuiContext, PluginApi, ProcessContext, Transport};
use crate::midi::NoteEvent;
use crate::param::internals::ParamPtr;
use crate::plugin::Plugin;

/// A [`GuiContext`] implementation for the wrapper. This is passed to the plugin in
/// [`Editor::spawn()`][crate::prelude::Editor::spawn()] so it can interact with the rest of the plugin and
/// with the host for things like setting parameters.
pub(crate) struct WrapperGuiContext<P: Plugin> {
    pub(super) wrapper: Arc<Wrapper<P>>,

    /// If the widnow should be resized, then we will write the size here. This will be set on the
    /// window at the start of the next frame.
    pub(super) new_window_size: Arc<Mutex<Option<(u32, u32)>>>,
}

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

impl<P: Plugin> GuiContext for WrapperGuiContext<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }

    fn request_resize(&self) -> bool {
        let new_size = self.wrapper.editor.as_ref().unwrap().size();

        // This will cause the editor to be resized at the start of the next frame. If we need to do
        // more of these things, then we should consider using a channel instead.
        *self.new_window_size.lock() = Some(new_size);

        true
    }

    // All of these functions are supposed to be called from the main thread, so we'll put some
    // trust in the caller and assume that this is indeed the case
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        nih_debug_assert_failure!("TODO: WrapperGuiContext::raw_begin_set_parameter()");
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        nih_debug_assert_failure!("TODO: WrapperGuiContext::raw_set_parameter_normalized()");
    }

    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        nih_debug_assert_failure!("TODO: WrapperGuiContext::raw_end_set_parameter()");
    }

    fn get_state(&self) -> crate::wrapper::state::PluginState {
        todo!("WrapperGuiContext::get_state()");
    }

    fn set_state(&self, state: crate::wrapper::state::PluginState) {
        nih_debug_assert_failure!("TODO: WrapperGuiContext::set_state()");
    }
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
