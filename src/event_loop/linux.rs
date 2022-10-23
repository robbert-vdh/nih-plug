//! An event loop implementation for Linux. APIs on Linux are generally thread safe, so the context
//! of a main thread does not exist there. Because of that, this mostly just serves as a way to
//! delegate expensive processing to another thread.

use crossbeam::channel;
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle, ThreadId};

use super::{EventLoop, MainThreadExecutor};
use crate::util::permit_alloc;

/// See [`EventLoop`][super::EventLoop].
#[cfg_attr(
    target_os = "macos",
    deprecated = "macOS needs to have its own event loop implementation, this implementation may \
                  not work correctly"
)]
pub(crate) struct LinuxEventLoop<T, E> {
    /// The thing that ends up executing these tasks. The tasks are usually executed from the worker
    /// thread, but if the current thread is the main thread then the task cna also be executed
    /// directly.
    executor: Arc<E>,

    /// The ID of the main thread. In practice this is the ID of the thread that created this task
    /// queue.
    main_thread_id: ThreadId,

    /// A thread that act as our worker thread. When [`schedule_gui()`][Self::schedule_gui()] is
    /// called, this thread will be woken up to execute the task on the executor. This is wrapped in
    /// an `Option` so the thread can be taken out of it and joined when this struct gets dropped.
    worker_thread: Option<JoinHandle<()>>,
    /// A channel for waking up the worker thread and having it perform one of the tasks from
    /// [`Message`].
    tasks_sender: channel::Sender<Message<T>>,
}

/// A message for communicating with the worker thread.
enum Message<T> {
    /// A new task for the event loop to execute.
    Task(T),
    /// Shut down the worker thread.
    Shutdown,
}

impl<T, E> EventLoop<T, E> for LinuxEventLoop<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    fn new_and_spawn(executor: Arc<E>) -> Self {
        let (tasks_sender, tasks_receiver) = channel::bounded(super::TASK_QUEUE_CAPACITY);

        Self {
            executor: executor.clone(),
            main_thread_id: thread::current().id(),
            // With our drop implementation we guarantee that this thread never outlives this struct
            worker_thread: Some(
                thread::Builder::new()
                    .name(String::from("worker"))
                    .spawn(move || worker_thread(tasks_receiver, Arc::downgrade(&executor)))
                    .expect("Could not spawn worker thread"),
            ),
            tasks_sender,
        }
    }

    fn schedule_gui(&self, task: T) -> bool {
        if self.is_main_thread() {
            self.executor.execute(task, true);
            true
        } else {
            self.tasks_sender.try_send(Message::Task(task)).is_ok()
        }
    }

    fn is_main_thread(&self) -> bool {
        // FIXME: `thread::current()` may allocate the first time it's called, is there a safe
        //        non-allocating version of this without using huge OS-specific libraries?
        permit_alloc(|| thread::current().id() == self.main_thread_id)
    }
}

impl<T, E> Drop for LinuxEventLoop<T, E> {
    fn drop(&mut self) {
        self.tasks_sender
            .send(Message::Shutdown)
            .expect("Failed while sending worker thread shutdown request");
        if let Some(join_handle) = self.worker_thread.take() {
            join_handle.join().expect("Worker thread panicked");
        }
    }
}

/// The worker thread used in [`EventLoop`] that executes incoming tasks on the event loop's
/// executor.
fn worker_thread<T, E>(tasks_receiver: channel::Receiver<Message<T>>, executor: Weak<E>)
where
    T: Send,
    E: MainThreadExecutor<T>,
{
    loop {
        match tasks_receiver.recv() {
            Ok(Message::Task(task)) => match executor.upgrade() {
                Some(e) => e.execute(task, true),
                None => {
                    nih_trace!(
                        "Received a new task but the executor is no longer alive, shutting down \
                         worker"
                    );
                    return;
                }
            },
            Ok(Message::Shutdown) => return,
            Err(err) => {
                nih_trace!(
                    "Worker thread got disconnected unexpectedly, shutting down: {}",
                    err
                );
                return;
            }
        }
    }
}
