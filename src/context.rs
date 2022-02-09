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

//! Different contexts the plugin can use to make callbacks to the host in different...contexts.

use std::sync::Weak;

#[cfg(all(target_family = "unix", not(target_os = "macos")))]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(target_family = "unix", not(target_os = "macos")))]
pub(crate) use self::linux::LinuxEventLoop as OsEventLoop;
#[cfg(target_os = "windows")]
pub(crate) use self::windows::WindowsEventLoop as OsEventLoop;
#[cfg(target_os = "macos")]
compile_error!("The macOS event loop has not yet been implemented");

use crate::param::internals::ParamPtr;
use crate::param::Param;
use crate::plugin::NoteEvent;

pub(crate) const TASK_QUEUE_CAPACITY: usize = 512;

// TODO: ProcessContext for parameter automation and sending events

/// General callbacks the plugin can make during its lifetime. This is passed to the plugin during
/// [crate::plugin::Plugin::initialize()] and as part of [crate::plugin::Plugin::process()].
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
/// values. This is passed to the plugin during [crate::plugin::Plugin::create_editor()]. All of
/// these functions assume they're being called from the main GUI thread.
//
// # Safety
//
// The implementing wrapper can assume that everything is being called from the main thread. Since
// NIH-plug doesn't own the GUI event loop, this invariant cannot be part of the interface.
pub trait GuiContext: Send + Sync + 'static {
    /// Inform the host a parameter will be automated. Create a [ParamSetter] and use
    /// [ParamSetter::begin_set_parameter] instead for a safe, user friendly API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr);

    /// Inform the host a parameter is being automated with an already normalized value. Create a
    /// [ParamSetter] and use [ParamSetter::set_parameter] instead for a safe, user friendly API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32);

    /// Inform the host a parameter has been automated. Create a [ParamSetter] and use
    /// [ParamSetter::end_set_parameter] instead for a safe, user friendly API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_end_set_parameter(&self, param: ParamPtr);

    /// Retrieve the default value for a parameter, in case you forgot. This does not perform a
    /// callback Create a [ParamSetter] and use [ParamSetter::default_param_value] instead for a
    /// safe, user friendly API.
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
    /// calling [Self::set_parameter()] for the specified parameter.
    pub fn begin_set_parameter<P: Param>(&self, param: &P) {
        unsafe { self.context.raw_begin_set_parameter(param.as_ptr()) };
    }

    /// Set a parameter to the specified parameter value. You will need to call
    /// [Self::begin_set_parameter()] before and [Self::end_set_parameter()] after calling this so
    /// the host can properly record automation for the parameter. This can be called multiple times
    /// in a row before calling [Self::end_set_parameter()], for instance when moving a slider
    /// around.
    ///
    /// This function assumes you're already calling this from a GUI thread. Calling any of these
    /// functions from any other thread may result in unexpected behavior.
    pub fn set_parameter<P: Param>(&self, param: &P, value: P::Plain) {
        let ptr = param.as_ptr();
        let normalized = param.preview_normalized(value);
        unsafe { self.context.raw_set_parameter_normalized(ptr, normalized) };
    }

    /// Set a parameter to an already normalized value. Works exactly the same as
    /// [Self::set_parameter] and needs to follow the same rules, but this may be useful when
    /// implementigna a GUI.
    pub fn set_parameter_normalized<P: Param>(&self, param: &P, normalized: f32) {
        let ptr = param.as_ptr();
        unsafe { self.context.raw_set_parameter_normalized(ptr, normalized) };
    }

    /// Inform the host that you are done automating a parameter. This needs to be called after one
    /// or more [Self::set_parameter()] calls for a parameter so the host knows the automation
    /// gesture has finished.
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

    /// The same as [Self::default_normalized_param_value], but without the normalization.
    pub fn default_param_value<P: Param>(&self, param: &P) -> P::Plain {
        param.preview_plain(self.default_normalized_param_value(param))
    }
}

/// A trait describing the functionality of the platform-specific event loop that can execute tasks
/// of type `T` in executor `E`. Posting a task to the internal task queue should be realtime safe.
/// This event loop should be created during the wrapper's initial initialization on the main
/// thread.
///
/// This is never used generically, but having this as a trait will cause any missing functions on
/// an implementation to show up as compiler errors even when using a different platform. And since
/// the tasks and executor will be sent to a thread, they need to have static lifetimes.
///
/// TODO: At some point rethink the design to make it possible to have a singleton message queue for
///       all instances of a plugin.
pub(crate) trait EventLoop<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    /// Create and start a new event loop. The thread this is called on will be designated as the
    /// main thread, so this should be called when constructing the wrapper.
    fn new_and_spawn(executor: Weak<E>) -> Self;

    /// Either post the function to the task queue so it can be delegated to the main thread, or
    /// execute the task directly if this is the main thread. This function needs to be callable at
    /// any time without blocking.
    ///
    /// If the task queue is full, then this will return false.
    #[must_use]
    fn do_maybe_async(&self, task: T) -> bool;

    /// Whether the calling thread is the event loop's main thread. This is usually the thread the
    /// event loop instance was initialized on.
    fn is_main_thread(&self) -> bool;
}

/// Something that can execute tasks of type `T`.
pub(crate) trait MainThreadExecutor<T>: Send + Sync {
    /// Execute a task on the current thread. This shoudl only be called from the main thread.
    unsafe fn execute(&self, task: T);
}
