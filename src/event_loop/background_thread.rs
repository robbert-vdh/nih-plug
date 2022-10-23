//! Used by the other [`EventLoop`][super::EventLoop] implementations to spawn threads for running
//! tasks in the background without blocking the GUI thread.
//!
//! This is essentially a slimmed down version of the `LinuxEventLoop`.

use crossbeam::channel;
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};

use super::MainThreadExecutor;

/// See the module's documentation. This is a slimmed down version of the `LinuxEventLoop` that can
/// be used with other OS and plugin format specific event loop implementations.
pub(crate) struct BackgroundThread<T> {
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

impl<T> BackgroundThread<T>
where
    T: Send + 'static,
{
    pub fn new_and_spawn<E>(executor: Arc<E>) -> Self
    where
        E: MainThreadExecutor<T> + 'static,
    {
        let (tasks_sender, tasks_receiver) = channel::bounded(super::TASK_QUEUE_CAPACITY);

        Self {
            // With our drop implementation we guarantee that this thread never outlives this struct
            worker_thread: Some(
                thread::Builder::new()
                    .name(String::from("bg-worker"))
                    .spawn(move || worker_thread(tasks_receiver, Arc::downgrade(&executor)))
                    .expect("Could not spawn background worker thread"),
            ),
            tasks_sender,
        }
    }

    pub fn schedule(&self, task: T) -> bool {
        self.tasks_sender.try_send(Message::Task(task)).is_ok()
    }
}

impl<T> Drop for BackgroundThread<T> {
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
