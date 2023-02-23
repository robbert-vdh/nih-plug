//! Used by the other [`EventLoop`][super::EventLoop] implementations to spawn threads for running
//! tasks in the background without blocking the GUI thread.
//!
//! This is essentially a slimmed down version of the `LinuxEventLoop`.

use crossbeam::channel;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};

use super::MainThreadExecutor;
use crate::util::permit_alloc;

/// See the module's documentation. This is a background thread that can be used to run tasks on.
/// The implementation shares a single thread between all of a plugin's instances hosted in the same
/// process.
pub(crate) struct BackgroundThread<T, E> {
    /// The object that actually executes the task `T`. We'll send a weak reference to this to the
    /// worker thread whenever a task needs to be executed. This allows multiple plugin instances to
    /// share the same worker thread.
    executor: Arc<E>,
    /// A thread that act as our worker thread. When [`schedule()`][Self::schedule()] is called,
    /// this thread will be woken up to execute the task on the executor. When the last worker
    /// thread handle gets dropped the thread is shut down.
    worker_thread: WorkerThreadHandle<T, E>,
}

/// A handle for the singleton worker thread. This lets multiple instances of the same plugin share
/// a worker thread, and when the last instance gets dropped the worker thread gets terminated.
struct WorkerThreadHandle<T, E> {
    pub(self) tasks_sender: channel::Sender<Message<T, E>>,
    /// The thread's reference count. Shared between all handles to the same thread. This is
    /// decrased by one when the struct is dropped.
    reference_count: Arc<AtomicIsize>,
    /// The thread's join handle. Joined when the reference count reaches 0.
    join_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

/// A message for communicating with the worker thread.
enum Message<T, E> {
    /// A new task for the event loop to execute along with the executor that should execute the
    /// task. A reference to the executor is sent alongside because multiple plugin instances may
    /// share the same background thread.
    Task((T, Weak<E>)),
    /// Shut down the worker thread. Send when the last reference to the thread is dropped.
    Shutdown,
}

impl<T, E> BackgroundThread<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    pub fn get_or_create(executor: Arc<E>) -> Self {
        Self {
            executor,
            // The same worker thread can be shared by multiple instances. Lifecycle management
            // happens through reference counting.
            worker_thread: get_or_create_worker_thread(),
        }
    }

    pub fn schedule(&self, task: T) -> bool {
        // NOTE: This may check the current thread ID, which involves an allocation whenever this
        //       first happens on a new thread because of the way thread local storage works
        permit_alloc(|| {
            self.worker_thread
                .tasks_sender
                .try_send(Message::Task((task, Arc::downgrade(&self.executor))))
                .is_ok()
        })
    }
}

// Rust does not allow us to use the `T` and `E` type variable in statics, so this is a
// workaround to have a singleton that also works if for whatever reason there arem ultiple `T`
// and `E`s in a single process (won't happen with normal plugin usage, but sho knwos).
lazy_static::lazy_static! {
    static ref HANDLE_MAP: Mutex<anymap::Map<dyn anymap::any::Any + Send + 'static>> =
        Mutex::new(anymap::Map::new());
}

impl<T, E> Clone for WorkerThreadHandle<T, E> {
    fn clone(&self) -> Self {
        self.reference_count.fetch_add(1, Ordering::SeqCst);

        Self {
            tasks_sender: self.tasks_sender.clone(),
            reference_count: self.reference_count.clone(),
            join_handle: self.join_handle.clone(),
        }
    }
}

impl<T, E> Drop for WorkerThreadHandle<T, E> {
    fn drop(&mut self) {
        // If the host for whatever reason instantiates and destroys a plugin at the same time from
        // different threads, we need to make sure this doesn't do anything weird.
        let _handle_map = HANDLE_MAP.lock();

        // The thread is shut down and joined when the last handle is dropped
        if self.reference_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.tasks_sender
                .send(Message::Shutdown)
                .expect("Failed while sending worker thread shutdown request");
            let join_handle = self
                .join_handle
                .lock()
                .take()
                .expect("The thread has already been joined");
            join_handle.join().expect("Worker thread panicked");
        }
    }
}

/// Either acquire a handle for an existing worker thread or create one if it does not yet exists.
/// This allows multiple plugin instances to share a worker thread. Reference counting happens
/// automatically as part of this function and `WorkerThreadHandle`'s lifecycle.
fn get_or_create_worker_thread<T, E>() -> WorkerThreadHandle<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    // The map entry contains both the thread's reference count
    // NOTE: This uses `AtomicIsize` for a reason. The `HANDLE_MAP` also holds a reference to this
    //       thread handle, and its `Drop` implementation will also fire if the
    //       `Option<WorkerThreadHandle<T, E>>` is ever overwritten. This will cause the reference
    //       count to become -1 which is fine.
    let mut handle_map = HANDLE_MAP.lock();
    let (reference_count, worker_thread_handle) = handle_map
        .entry::<(Arc<AtomicIsize>, Option<WorkerThreadHandle<T, E>>)>()
        .or_insert_with(|| (Arc::new(AtomicIsize::new(0)), None));

    // When this is the first reference to the worker thread, the thread is (re)initialized
    if reference_count.fetch_add(1, Ordering::SeqCst) <= 0 {
        let (tasks_sender, tasks_receiver) = channel::bounded(super::TASK_QUEUE_CAPACITY);
        let join_handle = thread::Builder::new()
            .name(String::from("bg-worker"))
            .spawn(move || worker_thread(tasks_receiver))
            .expect("Could not spawn background worker thread");

        // This needs special handling if `worker_thread_handle` was already a `Some` value because
        // the `Drop` will decrease the reference count when it gets overwritten. There may be a
        // better alternative to this.
        if worker_thread_handle.is_some() {
            reference_count.fetch_add(1, Ordering::SeqCst);
        }

        *worker_thread_handle = Some(WorkerThreadHandle {
            tasks_sender,
            reference_count: reference_count.clone(),
            join_handle: Arc::new(Mutex::new(Some(join_handle))),
        });
    }

    worker_thread_handle.clone().unwrap()
}

/// The worker thread used in [`EventLoop`] that executes incoming tasks on the event loop's
/// executor.
fn worker_thread<T, E>(tasks_receiver: channel::Receiver<Message<T, E>>)
where
    T: Send,
    E: MainThreadExecutor<T>,
{
    loop {
        match tasks_receiver.recv() {
            Ok(Message::Task((task, executor))) => match executor.upgrade() {
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
