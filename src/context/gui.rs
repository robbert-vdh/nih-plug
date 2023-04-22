//! A context passed to a plugin's editor.

use std::sync::Arc;

use super::PluginApi;
use crate::prelude::{Param, ParamPtr, Plugin, PluginState};

/// Callbacks the plugin can make when the user interacts with its GUI such as updating parameter
/// values. This is passed to the plugin during [`Editor::spawn()`][crate::prelude::Editor::spawn()]. All of
/// these functions assume they're being called from the main GUI thread.
//
// # Safety
//
// The implementing wrapper can assume that everything is being called from the main thread. Since
// NIH-plug doesn't own the GUI event loop, this invariant cannot be part of the interface.
pub trait GuiContext: Send + Sync + 'static {
    /// Get the current plugin API. This may be useful to display in the plugin's GUI as part of an
    /// about screen.
    fn plugin_api(&self) -> PluginApi;

    /// Ask the host to resize the editor window to the size specified by
    /// [`Editor::size()`][crate::prelude::Editor::size()]. This will return false if the host
    /// somehow didn't like this and rejected the resize, in which case the window should revert to
    /// its old size. You should only actually resize your embedded window once this returns `true`.
    ///
    /// TODO: Host->Plugin resizing has not been implemented yet
    fn request_resize(&self) -> bool;

    /// Inform the host a parameter will be automated. Create a [`ParamSetter`] and use
    /// [`ParamSetter::begin_set_parameter()`] instead for a safe, user friendly API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr);

    /// Inform the host a parameter is being automated with an already normalized value. Create a
    /// [`ParamSetter`] and use [`ParamSetter::set_parameter()`] instead for a safe, user friendly
    /// API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32);

    /// Inform the host a parameter has been automated. Create a [`ParamSetter`] and use
    /// [`ParamSetter::end_set_parameter()`] instead for a safe, user friendly API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_end_set_parameter(&self, param: ParamPtr);

    /// Serialize the plugin's current state to a serde-serializable object. Useful for implementing
    /// preset handling within a plugin's GUI.
    fn get_state(&self) -> PluginState;

    /// Restore the state from a previously serialized state object. This will block the GUI thread
    /// until the state has been restored and a parameter value rescan has been requested from the
    /// host. If the plugin is currently processing audio, then the parameter values will be
    /// restored at the end of the current processing cycle.
    fn set_state(&self, state: PluginState);
}

/// An way to run background tasks from the plugin's GUI, equivalent to the
/// [`ProcessContext::execute_background()`][crate::prelude::ProcessContext::execute_background()]
/// and [`ProcessContext::execute_gui()`][crate::prelude::ProcessContext::execute_gui()] functions.
/// This is passed directly to [`Plugin::editor()`] so the plugin can move it into its editor and
/// use it later.
///
/// # Note
///
/// This is only intended to be used from the GUI. Use the methods on
/// [`InitContext`][crate::prelude::InitContext] and
/// [`ProcessContext`][crate::prelude::ProcessContext] to run tasks during the `initialize()` and
/// `process()` functions.
//
// NOTE: This is separate from `GuiContext` because adding a type parameter there would clutter up a
//       lot of structs, and may even be incompatible with the way certain GUI libraries work.
pub struct AsyncExecutor<P: Plugin> {
    pub(crate) execute_background: Arc<dyn Fn(P::BackgroundTask) + Send + Sync>,
    pub(crate) execute_gui: Arc<dyn Fn(P::BackgroundTask) + Send + Sync>,
}

// Can't derive this since Rust then requires `P` to also be `Clone`able
impl<P: Plugin> Clone for AsyncExecutor<P> {
    fn clone(&self) -> Self {
        Self {
            execute_background: self.execute_background.clone(),
            execute_gui: self.execute_gui.clone(),
        }
    }
}

/// A convenience helper for setting parameter values. Any changes made here will be broadcasted to
/// the host and reflected in the plugin's [`Params`][crate::params::Params] object. These
/// functions should only be called from the main thread.
pub struct ParamSetter<'a> {
    pub raw_context: &'a dyn GuiContext,
}

impl<P: Plugin> AsyncExecutor<P> {
    /// Execute a task on a background thread using `[Plugin::task_executor]`. This allows you to
    /// defer expensive tasks for later without blocking either the process function or the GUI
    /// thread. As long as creating the `task` is realtime-safe, this operation is too.
    ///
    /// # Note
    ///
    /// Scheduling the same task multiple times will cause those duplicate tasks to pile up. Try to
    /// either prevent this from happening, or check whether the task still needs to be completed in
    /// your task executor.
    pub fn execute_background(&self, task: P::BackgroundTask) {
        (self.execute_background)(task);
    }

    /// Execute a task on a background thread using `[Plugin::task_executor]`.
    ///
    /// # Note
    ///
    /// Scheduling the same task multiple times will cause those duplicate tasks to pile up. Try to
    /// either prevent this from happening, or check whether the task still needs to be completed in
    /// your task executor.
    pub fn execute_gui(&self, task: P::BackgroundTask) {
        (self.execute_gui)(task);
    }
}

impl<'a> ParamSetter<'a> {
    pub fn new(context: &'a dyn GuiContext) -> Self {
        Self {
            raw_context: context,
        }
    }

    /// Inform the host that you will start automating a parameter. This needs to be called before
    /// calling [`set_parameter()`][Self::set_parameter()] for the specified parameter.
    pub fn begin_set_parameter<P: Param>(&self, param: &P) {
        unsafe { self.raw_context.raw_begin_set_parameter(param.as_ptr()) };
    }

    /// Set a parameter to the specified parameter value. You will need to call
    /// [`begin_set_parameter()`][Self::begin_set_parameter()] before and
    /// [`end_set_parameter()`][Self::end_set_parameter()] after calling this so the host can
    /// properly record automation for the parameter. This can be called multiple times in a row
    /// before calling [`end_set_parameter()`][Self::end_set_parameter()], for instance when moving
    /// a slider around.
    ///
    /// This function assumes you're already calling this from a GUI thread. Calling any of these
    /// functions from any other thread may result in unexpected behavior.
    pub fn set_parameter<P: Param>(&self, param: &P, value: P::Plain) {
        let ptr = param.as_ptr();
        let normalized = param.preview_normalized(value);
        unsafe {
            self.raw_context
                .raw_set_parameter_normalized(ptr, normalized)
        };
    }

    /// Set a parameter to an already normalized value. Works exactly the same as
    /// [`set_parameter()`][Self::set_parameter()] and needs to follow the same rules, but this may
    /// be useful when implementing a GUI.
    ///
    /// This does not perform any snapping. Consider converting the normalized value to a plain
    /// value and setting that with [`set_parameter()`][Self::set_parameter()] instead so the
    /// normalized value known to the host matches `param.normalized_value()`.
    pub fn set_parameter_normalized<P: Param>(&self, param: &P, normalized: f32) {
        let ptr = param.as_ptr();
        unsafe {
            self.raw_context
                .raw_set_parameter_normalized(ptr, normalized)
        };
    }

    /// Inform the host that you are done automating a parameter. This needs to be called after one
    /// or more [`set_parameter()`][Self::set_parameter()] calls for a parameter so the host knows
    /// the automation gesture has finished.
    pub fn end_set_parameter<P: Param>(&self, param: &P) {
        unsafe { self.raw_context.raw_end_set_parameter(param.as_ptr()) };
    }
}
