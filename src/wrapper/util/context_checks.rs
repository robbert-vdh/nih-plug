//! Common validation used for the [contexts][crate::context].

use std::collections::HashSet;

/// Ensures that parameter changes send from the GUI are wrapped in parameter gestures, and that the
/// gestures are handled consistently (no duplicate starts and ends, no end before start, etc.).
///
/// Should only be used in debug builds.
#[derive(Debug, Default)]
pub struct ParamGestureChecker {
    /// The parameters with an active gesture.
    active_params: HashSet<String>,
}

impl Drop for ParamGestureChecker {
    fn drop(&mut self) {
        nih_debug_assert!(
            self.active_params.is_empty(),
            "GuiContext::end_set_parameter() was never called for {} {} {:?}",
            self.active_params.len(),
            if self.active_params.len() == 1 {
                "parameter"
            } else {
                "parameters"
            },
            self.active_params
        );
    }
}

impl ParamGestureChecker {
    /// Called for
    /// [`GuiContext::begin_set_parameter()`][crate::prelude::GuiContext::begin_set_parameter()].
    /// Triggers a debug assertion failure if the state is inconsistent.
    pub fn begin_set_parameter(&mut self, param_id: &str) {
        nih_debug_assert!(
            !self.active_params.contains(param_id),
            "GuiContext::begin_set_parameter() was called twice for parameter '{}'",
            param_id
        );
        self.active_params.insert(param_id.to_owned());
    }

    /// Called for [`GuiContext::set_parameter()`][crate::prelude::GuiContext::set_parameter()].
    /// Triggers a debug assertion failure if the state is inconsistent.
    pub fn set_parameter(&self, param_id: &str) {
        nih_debug_assert!(
            self.active_params.contains(param_id),
            "GuiContext::set_parameter() was called for parameter '{}' without a preceding \
             begin_set_parameter() call",
            param_id
        );
    }

    /// Called for
    /// [`GuiContext::end_set_parameter()`][crate::prelude::GuiContext::end_set_parameter()].
    /// Triggers a debug assertion failure if the state is inconsistent.
    pub fn end_set_parameter(&mut self, param_id: &str) {
        nih_debug_assert!(
            self.active_params.contains(param_id),
            "GuiContext::end_set_parameter() was called for parameter '{}' without a preceding \
             begin_set_parameter() call",
            param_id
        );
        self.active_params.remove(param_id);
    }
}
