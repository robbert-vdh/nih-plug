//! An event loop implementation for Linux. APIs on Linux are generally thread safe, so the context
//! of a main thread does not exist there. Because of that, this mostly just serves as a way to
//! delegate expensive processing to another thread.

use std::sync::Weak;
use std::thread::{self, ThreadId};

use super::{BackgroundThread, EventLoop, MainThreadExecutor};
use crate::util::permit_alloc;

/// See [`EventLoop`][super::EventLoop].
pub(crate) struct LinuxEventLoop<T, E> {
    /// The thing that ends up executing these tasks. The tasks are usually executed from the worker
    /// thread, but if the current thread is the main thread then the task cna also be executed
    /// directly.
    executor: Weak<E>,

    /// The actual background thread. The implementation is shared with the background thread used
    /// in other backends.
    background_thread: BackgroundThread<T, E>,

    /// The ID of the main thread. In practice this is the ID of the thread that created this task
    /// queue.
    main_thread_id: ThreadId,
}

impl<T, E> EventLoop<T, E> for LinuxEventLoop<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    fn new_and_spawn(executor: Weak<E>) -> Self {
        Self {
            executor: executor.clone(),
            background_thread: BackgroundThread::get_or_create(executor),
            main_thread_id: thread::current().id(),
        }
    }

    fn schedule_gui(&self, task: T) -> bool {
        if self.is_main_thread() {
            match self.executor.upgrade() {
                Some(executor) => executor.execute(task, true),
                None => {
                    nih_debug_assert_failure!("GUI task was posted after the executor was dropped")
                }
            }

            true
        } else {
            self.background_thread.schedule(task)
        }
    }

    fn schedule_background(&self, task: T) -> bool {
        // This event loop implementation already uses a thread that's completely decoupled from the
        // operating system's or the host's main thread, so we don't need _another_ thread here
        self.background_thread.schedule(task)
    }

    fn is_main_thread(&self) -> bool {
        // FIXME: `thread::current()` may allocate the first time it's called, is there a safe
        //        non-allocating version of this without using huge OS-specific libraries?
        permit_alloc(|| thread::current().id() == self.main_thread_id)
    }
}
