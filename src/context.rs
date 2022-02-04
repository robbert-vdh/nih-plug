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

use crate::plugin::NoteEvent;

pub(crate) const TASK_QUEUE_CAPACITY: usize = 512;

// TODO: ProcessContext for parameter automation and sending events
// TODO: GuiContext for GUI parameter automation and resizing

/// General callbacks the plugin can make during its lifetime. This is passed to the plugin during
/// [crate::plugin::Plugin::initialize].
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

    // // TODO: Add this next
    // fn set_parameter<P>(&self, param: &P, value: P::Plain)
    // where
    //     P: Param;
}

/// A trait describing the functionality of the platform-specific event loop that can execute tasks
/// of type `T` in executor `E`. Posting a task to the internal task queue should be realtime safe.
/// This event loop should be created during the wrapper's initial initialization on the main
/// thread.
///
/// This is never used generically, but having this as a trait will cause any missing functions on
/// an implementation to show up as compiler errors even when using a different platform.
///
/// TODO: At some point rethink the design to make it possible to have a singleton message queue for
///       all instances of a plugin.
pub(crate) trait EventLoop<T, E>
where
    T: Send,
    E: MainThreadExecutor<T>,
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
