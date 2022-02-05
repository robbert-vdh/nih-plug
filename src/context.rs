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

#[cfg(all(target_family = "unix", not(target_os = "macos")))]
pub(crate) use linux::LinuxEventLoop as OsEventLoop;

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

/// Callbacks the plugin can make while handling its GUI, such as updating parameter values. This is
/// passed to the plugin during [crate::plugin::Plugin::create_editor()].
//
// # Safety
//
// The implementing wrapper needs to be able to handle concurrent requests, and it should perform
// the actual callback within [MainThreadQueue::do_maybe_async].
//
// TODO: Update documentation
// TODO: Add the safe generic setter API
pub trait GuiContext: Send + Sync {
    /// TODO: Docuemnt safe API
    fn setter(&self) -> ParamSetter
    where
        Self: Sized,
    {
        ParamSetter { context: self }
    }

    /// Inform the host that you will start automating a parmater. This needs to be called before
    /// calling [Self::set_parameter()] for the specified parameter.
    unsafe fn begin_set_parameter(&self, param: ParamPtr);

    /// Set a parameter to the specified parameter value. You will need to call
    /// [Self::begin_set_parameter()] before and [Self::end_set_parameter()] after calling this so
    /// the host can properly record automation for the parameter. This can be called multiple times
    /// in a row before calling [Self::end_set_parameter()], for instance when moving a slider
    /// around.
    ///
    /// This function assumes you're already calling this from a GUI thread. Calling any of these
    /// functions from any other thread may result in unexpected behavior.
    // TODO: Move into helper
    // fn set_parameter<P: Param>(&self, param: &P, value: P::Plain);
    unsafe fn set_parameter_normalized(&self, param: ParamPtr, normalized: f32);

    /// Inform the host that you are done automating a parameter. This needs to be called after one
    /// or more [Self::set_parameter()] calls for a parameter so the host knows the automation
    /// gesture has finished.
    unsafe fn end_set_parameter(&self, param: ParamPtr);
}

/// A convenience struct for setting parameter values.
// TODO: Document
pub struct ParamSetter<'a> {
    context: &'a dyn GuiContext,
}

impl ParamSetter<'_> {
    /// Inform the host that you will start automating a parmater. This needs to be called before
    /// calling [Self::set_parameter()] for the specified parameter.
    pub fn begin_set_parameter<P: Param>(&self, param: &P) {
        todo!()
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
        todo!()
    }

    /// Inform the host that you are done automating a parameter. This needs to be called after one
    /// or more [Self::set_parameter()] calls for a parameter so the host knows the automation
    /// gesture has finished.
    pub fn end_set_parameter<P: Param>(&self, param: &P) {
        todo!()
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
