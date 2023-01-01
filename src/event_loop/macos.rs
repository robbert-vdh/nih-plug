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
use std::sync::Arc;

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
    executor: Arc<E>,

    /// A background thread for running tasks independently from the host's GUI thread. Useful for
    /// longer, blocking tasks.
    background_thread: BackgroundThread<T>,

    /// The reference to the run-loop source so that it can be torn down when this struct is dropped.
    loop_source: LoopSourceWrapper,

    /// The sender for passing messages to the main thread.
    main_thread_sender: Sender<T>,

    /// Is it necessary to store the information that is sent to the callback function here?
    _info: Arc<(Arc<E>, Receiver<T>)>,
}

impl<T, E> EventLoop<T, E> for MacOSEventLoop<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    fn new_and_spawn(executor: Arc<E>) -> Self {
        // What's a good size for the queue?
        let (main_thread_sender, main_thread_receiver) = channel::bounded::<T>(32);

        let info = Arc::new((executor.clone(), main_thread_receiver));

        let loop_source;
        unsafe {
            let mut source_context = CFRunLoopSourceContext {
                info: Arc::as_ptr(&info) as *mut c_void,
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
                &mut source_context as *mut _ as *mut CFRunLoopSourceContext,
            );
            CFRunLoopAddSource(CFRunLoopGetMain(), loop_source, kCFRunLoopCommonModes);
        }

        Self {
            executor: executor.clone(),
            background_thread: BackgroundThread::new_and_spawn(executor),
            loop_source: LoopSourceWrapper(loop_source),
            main_thread_sender,
            _info: info,
        }
    }

    fn schedule_gui(&self, task: T) -> bool {
        if self.is_main_thread() {
            self.executor.execute(task, true);
            true
        } else {
            // Only signal the main thread callback to be called if the task was added to the queue.
            if self.main_thread_sender.try_send(task).is_ok() {
                unsafe {
                    CFRunLoopSourceSignal(self.loop_source.0);
                    CFRunLoopWakeUp(CFRunLoopGetMain());
                }
                true
            } else {
                false
            }
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
    unsafe {
        let (executor, receiver) = &*(info as *mut (Arc<E>, Receiver<T>));
        while let Ok(task) = receiver.try_recv() {
            executor.execute(task, true);
        }
    }
}
