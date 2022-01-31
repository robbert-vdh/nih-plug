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

use std::sync::Arc;

#[cfg(all(target_family = "unix", not(target_os = "macos")))]
mod linux;

#[cfg(all(target_family = "unix", not(target_os = "macos")))]
pub(crate) use linux::LinuxEventLoop as OsEventLoop;

// TODO: ProcessContext for parameter automation and sending events
// TODO: GuiContext for GUI parameter automation and resizing

pub(crate) const TASK_QUEUE_CAPACITY: usize = 512;

/// General callbacks the plugin can make during its lifetime. This is passed to the plugin during
/// [Plugin::initialize].
//
// # Safety
//
// The implementing wrapper needs to be able to handle concurrent requests, and it should perform
// the actual callback within [MainThreadQueue::do_maybe_async].
pub trait ProcessContext {
    /// Update the current latency of the plugin. If the plugin is currently processing audio, then
    /// this may cause audio playback to be restarted.
    fn set_latency_samples(&self, samples: u32);
}

/// A trait describing the functionality of the platform-specific event loop that can execute tasks
/// of type `T` in executor `E`. Posting a task to the queue should be realtime safe. This thread
/// queue should be created during the wrapper's initial initialization on the main thread.
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
    /// Create a main thread tasks queue for the given executor. The thread this is called on will
    /// be designated as the main thread, so this should be called when constructing the wrapper.
    ///
    /// TODO: Spawn, and update docs
    fn new_and_spawn(executor: Arc<E>) -> Self;

    /// Either post the function to a queue so it can be run later from the main thread using a
    /// timer, or run the function directly if this is the main thread. This needs to be callable at
    /// any time withotu blocking.
    ///
    /// If the task queue was full, then this will return false.
    #[must_use]
    fn do_maybe_async(&self, task: T) -> bool;

    /// Whether the calling thread is the even loop's main thread. This is usually the thread the
    /// event loop instance wel initialized on.
    fn is_main_thread(&self) -> bool;
}

/// Something that can execute tasks of type `T`.
pub(crate) trait MainThreadExecutor<T>: Send + Sync {
    /// Execute a task on the current thread.
    fn execute(&self, task: T);
}
