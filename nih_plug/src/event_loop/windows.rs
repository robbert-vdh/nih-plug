//! An event loop for windows, using an invisible window to hook into the host's message loop. This
//! has only been tested under Wine with [yabridge](https://github.com/robbert-vdh/yabridge).

use crossbeam::channel;
use std::ffi::{c_void, CString};
use std::mem;
use std::ptr;
use std::sync::Weak;
use std::thread::{self, ThreadId};
use windows::core::PCSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::{
    LibraryLoader::GetModuleHandleA, Performance::QueryPerformanceCounter,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExA, DefWindowProcA, DestroyWindow, GetWindowLongPtrA, PostMessageA,
    RegisterClassExA, SetWindowLongPtrA, UnregisterClassA, CREATESTRUCTA, GWLP_USERDATA, HMENU,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CREATE, WM_DESTROY, WM_USER, WNDCLASSEXA,
};

use super::{BackgroundThread, EventLoop, MainThreadExecutor};
use crate::util::permit_alloc;

/// The custom message ID for our notify event. If the hidden event loop window receives this, then
/// it knows it should start polling events.
const NOTIFY_MESSAGE_ID: u32 = WM_USER;

/// A type erased function passed to the window so it can poll for events. We can't pass the tasks
/// queue and executor to the window callback since the callback wouldn't know what types they are,
/// but we can wrap the polling loop in a closure and pass that instead.
///
/// This needs to be double boxed when passed to the function since fat pointers cannot be directly
/// casted from a regular pointer.
type PollCallback = Box<dyn Fn()>;

/// See [`EventLoop`][super::EventLoop].
pub(crate) struct WindowsEventLoop<T, E> {
    /// The thing that ends up executing these tasks. The tasks are usually executed from the worker
    /// thread, but if the current thread is the main thread then the task cna also be executed
    /// directly.
    executor: Weak<E>,

    /// The ID of the main thread. In practice this is the ID of the thread that created this task
    /// queue.
    main_thread_id: ThreadId,

    /// An invisible window that we can post a message to when we need to do something on the main
    /// thread. The host's message loop will then cause our message to be proceeded.
    message_window: HWND,
    /// The unique class for the message window, we'll clean this up together with the window.
    message_window_class_name: CString,
    /// A queue of tasks that still need to be performed. When something gets added to this queue
    /// we'll wake up the window, which then continues to pop tasks off this queue until it is
    /// empty.
    tasks_sender: channel::Sender<T>,

    /// A background thread for running tasks independently from the host's GUI thread. Useful for
    /// longer, blocking tasks.
    background_thread: BackgroundThread<T, E>,
}

impl<T, E> EventLoop<T, E> for WindowsEventLoop<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    fn new_and_spawn(executor: Weak<E>) -> Self {
        let (tasks_sender, tasks_receiver) = channel::bounded(super::TASK_QUEUE_CAPACITY);

        // Window classes need to have unique names or else multiple plugins loaded into the same
        // process will end up calling the other plugin's callbacks
        let mut ticks = 0i64;
        assert!(unsafe { QueryPerformanceCounter(&mut ticks).as_bool() });
        let class_name = CString::new(format!("nih-event-loop-{ticks}"))
            .expect("Where did these null bytes come from?");
        let class_name_ptr = PCSTR(class_name.as_bytes_with_nul().as_ptr());

        let class = WNDCLASSEXA {
            cbSize: mem::size_of::<WNDCLASSEXA>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: unsafe { GetModuleHandleA(PCSTR(ptr::null())) }
                .expect("Could not get the current module's handle"),
            lpszClassName: class_name_ptr,
            ..Default::default()
        };
        assert_ne!(unsafe { RegisterClassExA(&class) }, 0);

        // This will be called by the hidden event loop when it gets woken up to process events. We
        // can't pass the tasks queue and the executor to it directly, so this is a simple type
        // erased version of the polling loop.
        let callback: PollCallback = {
            let executor = executor.clone();
            Box::new(move || {
                let executor = match executor.upgrade() {
                    Some(e) => e,
                    None => {
                        nih_debug_assert_failure!("Executor died before the message loop exited");
                        return;
                    }
                };

                while let Ok(task) = tasks_receiver.try_recv() {
                    executor.execute(task, true);
                }
            })
        };

        let window = unsafe {
            CreateWindowExA(
                WINDOW_EX_STYLE(0),
                class_name_ptr,
                PCSTR(b"NIH-plug event loop\0".as_ptr()),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                HWND(0),
                HMENU(0),
                HINSTANCE(0),
                // NOTE: We're boxing a box here. As mentioned in [PollCallback], we can't directly
                //       pass around fat pointers, so we need a normal pointer to a fat pointer to
                //       be able to call this and deallocate it later
                Some(Box::into_raw(Box::new(callback)) as *const c_void),
            )
        };
        assert_ne!(!window.0, 0);

        Self {
            executor: executor.clone(),
            main_thread_id: thread::current().id(),
            message_window: window,
            message_window_class_name: class_name,
            tasks_sender,
            background_thread: BackgroundThread::get_or_create(executor),
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
            let success = self.tasks_sender.try_send(task).is_ok();
            if success {
                // Instead of polling on a timer, we can just wake up the window whenever there's a
                // new message.
                unsafe {
                    PostMessageA(self.message_window, NOTIFY_MESSAGE_ID, WPARAM(0), LPARAM(0))
                };
            }

            success
        }
    }

    fn schedule_background(&self, task: T) -> bool {
        self.background_thread.schedule(task)
    }

    fn is_main_thread(&self) -> bool {
        // FIXME: `thread::current()` may allocate the first time it's called, is there a safe
        //        non-allocating version of this without using huge OS-specific libraries?
        permit_alloc(|| thread::current().id() == self.main_thread_id)
    }
}

impl<T, E> Drop for WindowsEventLoop<T, E> {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.message_window) };
        unsafe {
            UnregisterClassA(
                PCSTR(self.message_window_class_name.as_bytes_with_nul().as_ptr()),
                HINSTANCE(0),
            )
        };
    }
}

unsafe extern "system" fn window_proc(
    handle: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CREATE => {
            let create_params = lparam.0 as *const CREATESTRUCTA;
            assert!(!create_params.is_null());

            let poll_callback = (*create_params).lpCreateParams as *mut PollCallback;
            assert!(!poll_callback.is_null());

            // Store this for later use
            SetWindowLongPtrA(handle, GWLP_USERDATA, poll_callback as isize);
        }
        NOTIFY_MESSAGE_ID => {
            let callback = GetWindowLongPtrA(handle, GWLP_USERDATA) as *mut PollCallback;
            if callback.is_null() {
                nih_debug_assert_failure!(
                    "The notify function got called before the window was created"
                );
                return LRESULT(0);
            }

            // This callback function just keeps popping off and handling tasks from the tasks queue
            // until there's nothing left
            (*callback)();
        }
        WM_DESTROY => {
            // Make sure to deallocate the polling callback we stored earlier
            let _the_bodies_hit_the_floor =
                Box::from_raw(GetWindowLongPtrA(handle, GWLP_USERDATA) as *mut PollCallback);
            SetWindowLongPtrA(handle, GWLP_USERDATA, 0);
        }
        _ => (),
    }

    DefWindowProcA(handle, message, wparam, lparam)
}
