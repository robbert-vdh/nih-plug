//! An event loop implementation for macOS.

use core_foundation::base::kCFAllocatorDefault;
use core_foundation::runloop::{
    kCFRunLoopCommonModes, CFRunLoopAddSource, CFRunLoopGetMain, CFRunLoopRemoveSource,
    CFRunLoopSourceContext, CFRunLoopSourceCreate, CFRunLoopSourceInvalidate, CFRunLoopSourceRef,
    CFRunLoopSourceSignal, CFRunLoopWakeUp,
};
use crossbeam::channel::{self, Receiver, Sender};
use objc::{class, msg_send, sel, sel_impl};
use std::os::raw::c_void;
use std::sync::Weak;

use super::{BackgroundThread, EventLoop, MainThreadExecutor};

/// Wrapping the `CFRunLoopSourceRef` type is required to be able to annotate it as thread-safe.
struct LoopSourceWrapper(CFRunLoopSourceRef);

unsafe impl Send for LoopSourceWrapper {}
unsafe impl Sync for LoopSourceWrapper {}

/// See [`EventLoop`][super::EventLoop].
pub(crate) struct MacOSEventLoop<T, E> {
    /// The thing that ends up executing these tasks. The tasks are usually executed from the worker
    /// thread, but if the current thread is the main thread then the task cna also be executed
    /// directly.
    executor: Weak<E>,

    /// A background thread for running tasks independently from the host's GUI thread. Useful for
    /// longer, blocking tasks.
    background_thread: BackgroundThread<T, E>,

    /// The reference to the run-loop source so that it can be torn down when this struct is
    /// dropped.
    loop_source: LoopSourceWrapper,

    /// The sender for passing messages to the main thread.
    main_thread_sender: Sender<T>,

    /// The data that is passed to the external run loop source callback function via a raw pointer.
    /// The data is not accessed from the Rust side after creating it but it's kept here so as not
    /// to get dropped.
    _callback_data: Box<(Weak<E>, Receiver<T>)>,
}

impl<T, E> EventLoop<T, E> for MacOSEventLoop<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    fn new_and_spawn(executor: Weak<E>) -> Self {
        let (main_thread_sender, main_thread_receiver) =
            channel::bounded::<T>(super::TASK_QUEUE_CAPACITY);

        let callback_data = Box::new((executor.clone(), main_thread_receiver));

        let loop_source;
        unsafe {
            let source_context = CFRunLoopSourceContext {
                info: &*callback_data as *const _ as *mut c_void,
                cancel: None,
                copyDescription: None,
                equal: None,
                hash: None,
                perform: loop_source_callback::<T, E>,
                release: None,
                retain: None,
                schedule: None,
                version: 0,
            };

            loop_source = CFRunLoopSourceCreate(
                kCFAllocatorDefault,
                1,
                &source_context as *const _ as *mut CFRunLoopSourceContext,
            );
            CFRunLoopAddSource(CFRunLoopGetMain(), loop_source, kCFRunLoopCommonModes);
        }

        Self {
            executor: executor.clone(),
            background_thread: BackgroundThread::get_or_create(executor),
            loop_source: LoopSourceWrapper(loop_source),
            main_thread_sender,
            _callback_data: callback_data,
        }
    }

    fn schedule_gui(&self, task: T) -> bool {
        if self.is_main_thread() {
            match self.executor.upgrade() {
                Some(executor) => executor.execute(task, true),
                None => nih_debug_assert_failure!("GUI task posted after the executor was dropped"),
            }

            true
        } else {
            // Only signal the main thread callback to be called if the task was added to the queue.
            let success = self.main_thread_sender.try_send(task).is_ok();
            if success {
                unsafe {
                    CFRunLoopSourceSignal(self.loop_source.0);
                    CFRunLoopWakeUp(CFRunLoopGetMain());
                }
            }

            success
        }
    }

    fn schedule_background(&self, task: T) -> bool {
        self.background_thread.schedule(task)
    }

    fn is_main_thread(&self) -> bool {
        unsafe { msg_send![class!(NSThread), isMainThread] }
    }
}

impl<T, E> Drop for MacOSEventLoop<T, E> {
    fn drop(&mut self) {
        unsafe {
            CFRunLoopRemoveSource(
                CFRunLoopGetMain(),
                self.loop_source.0,
                kCFRunLoopCommonModes,
            );
            CFRunLoopSourceInvalidate(self.loop_source.0);
        }
    }
}

extern "C" fn loop_source_callback<T, E>(info: *const c_void)
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    let (executor, receiver) = unsafe { &*(info as *mut (Weak<E>, Receiver<T>)) };
    let executor = match executor.upgrade() {
        Some(executor) => executor,
        None => {
            nih_debug_assert_failure!("GUI task was posted after the executor was dropped");
            return;
        }
    };

    while let Ok(task) = receiver.try_recv() {
        executor.execute(task, true);
    }
}
