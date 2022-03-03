//! Different contexts the plugin can use to make callbacks to the host in different...contexts.

use crate::param::internals::ParamPtr;
use crate::param::Param;
use crate::plugin::NoteEvent;

// TODO: ProcessContext for parameter automation and sending events

/// General callbacks the plugin can make during its lifetime. This is passed to the plugin during
/// [`Plugin::initialize()`][crate::plugin::Plugin::initialize()] and as part of
/// [`Plugin::process()`][crate::plugin::Plugin::process()].
//
// # Safety
//
// The implementing wrapper needs to be able to handle concurrent requests, and it should perform
// the actual callback within [MainThreadQueue::do_maybe_async].
pub trait ProcessContext {
    /// Update the current latency of the plugin. If the plugin is currently processing audio, then
    /// this may cause audio playback to be restarted.
    fn set_latency_samples(&self, samples: u32);

    /// Return the next note event, if there is one. The event contains the timing
    ///
    /// TODO: Rethink this API, both in terms of ergonomics, and if we can do this in a way that
    ///       doesn't require locks (because of the thread safe-ness, which we don't really need
    ///       here)
    fn next_midi_event(&mut self) -> Option<NoteEvent>;

    // TODO: Add this, this works similar to [GuiContext::set_parameter] but it adds the parameter
    //       change to a queue (or directly to the VST3 plugin's parameter output queues) instead of
    //       using main thread host automation (and all the locks involved there).
    // fn set_parameter<P: Param>(&self, param: &P, value: P::Plain);
}

/// Callbacks the plugin can make when the user interacts with its GUI such as updating parameter
/// values. This is passed to the plugin during [`Editor::spawn()`][crate::Editor::spawn()]. All of
/// these functions assume they're being called from the main GUI thread.
//
// # Safety
//
// The implementing wrapper can assume that everything is being called from the main thread. Since
// NIH-plug doesn't own the GUI event loop, this invariant cannot be part of the interface.
pub trait GuiContext: Send + Sync + 'static {
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

    /// Retrieve the default value for a parameter, in case you forgot. This does not perform a
    /// callback Create a [`ParamSetter`] and use [`ParamSetter::default_param_value()`] instead for
    /// a safe, user friendly API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_default_normalized_param_value(&self, param: ParamPtr) -> f32;
}

/// A convenience helper for setting parameter values. Any changes made here will be broadcasted to
/// the host and reflected in the plugin's [crate::param::internals::Params] object. These functions
/// should only be called from the main thread.
pub struct ParamSetter<'a> {
    context: &'a dyn GuiContext,
}

impl<'a> ParamSetter<'a> {
    pub fn new(context: &'a dyn GuiContext) -> Self {
        Self { context }
    }

    /// Inform the host that you will start automating a parmater. This needs to be called before
    /// calling [`set_parameter()`][Self::set_parameter()] for the specified parameter.
    pub fn begin_set_parameter<P: Param>(&self, param: &P) {
        unsafe { self.context.raw_begin_set_parameter(param.as_ptr()) };
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
        unsafe { self.context.raw_set_parameter_normalized(ptr, normalized) };
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
        unsafe { self.context.raw_set_parameter_normalized(ptr, normalized) };
    }

    /// Inform the host that you are done automating a parameter. This needs to be called after one
    /// or more [`set_parameter()`][Self::set_parameter()] calls for a parameter so the host knows
    /// the automation gesture has finished.
    pub fn end_set_parameter<P: Param>(&self, param: &P) {
        unsafe { self.context.raw_end_set_parameter(param.as_ptr()) };
    }

    /// Retrieve the default value for a parameter, in case you forgot. The value is already
    /// normalized to `[0, 1]`. This is useful when implementing GUIs, and it does not perform a callback.
    pub fn default_normalized_param_value<P: Param>(&self, param: &P) -> f32 {
        unsafe {
            self.context
                .raw_default_normalized_param_value(param.as_ptr())
        }
    }

    /// The same as [`default_normalized_param_value()`][Self::default_normalized_param_value()],
    /// but without the normalization.
    pub fn default_param_value<P: Param>(&self, param: &P) -> P::Plain {
        param.preview_plain(self.default_normalized_param_value(param))
    }
}
