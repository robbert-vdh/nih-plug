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

//! An event loop for windows, using an invisible window to hook into the host's message loop. This
//! has only been tested under Wine.

use crossbeam::queue::ArrayQueue;
use std::ffi::{c_void, CString};
use std::mem;
use std::ptr;
use std::sync::Arc;
use std::sync::Weak;
use std::thread::{self, ThreadId};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, PSTR, WPARAM};
use windows::Win32::System::{
    LibraryLoader::GetModuleHandleA, Performance::QueryPerformanceCounter,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CloseWindow, CreateWindowExA, DefWindowProcA, RegisterClassExA, UnregisterClassA, HMENU,
    WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSEXA,
};

use super::{EventLoop, MainThreadExecutor};
use crate::nih_log;

compile_error!("The Windows event loop has not yet been fully implemented");

/// See [super::EventLoop].
pub(crate) struct WindowsEventLoop<T, E> {
    /// The thing that ends up executing these tasks. The tasks are usually executed from the worker
    /// thread, but if the current thread is the main thread then the task cna also be executed
    /// directly.
    executor: Weak<E>,

    /// The ID of the main thread. In practice this is the ID of the thread that created this task
    /// queue.
    main_thread_id: ThreadId,

    /// An invisible window that we can post a message to when we need to do something on the main
    /// thread. The host's message loop will then cause our message to be proceded.
    message_window: HWND,
    /// The unique class for the message window, we'll clean this up together with the window.
    message_window_class_name: CString,
    /// A queue of tasks that still need to be performed. When something gets added to this queue
    /// we'll wake up the window, which then continues to pop tasks off this queue until it is
    /// empty.
    tasks: Arc<ArrayQueue<T>>,
}

impl<T, E> EventLoop<T, E> for WindowsEventLoop<T, E>
where
    T: Send + 'static,
    E: MainThreadExecutor<T> + 'static,
{
    fn new_and_spawn(executor: Weak<E>) -> Self {
        // We'll pass one copy of the this to the window, and we'll keep the other copy here
        let tasks = Arc::new(ArrayQueue::new(super::TASK_QUEUE_CAPACITY));

        // Window classes need to have unique names or else multiple plugins loaded into the same
        // process will end up calling the other plugin's callbacks
        let mut ticks = 0i64;
        assert!(unsafe { QueryPerformanceCounter(&mut ticks).as_bool() });
        let class_name = CString::new(format!("nih-event-loop-{ticks}"))
            .expect("Where did these null bytes come from?");
        let class_name_ptr = PSTR(class_name.as_bytes_with_nul().as_ptr());

        let class = WNDCLASSEXA {
            cbSize: mem::size_of::<WNDCLASSEXA>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: unsafe { GetModuleHandleA(PSTR(ptr::null())) },
            lpszClassName: class_name_ptr,
            ..Default::default()
        };
        assert_ne!(unsafe { RegisterClassExA(&class) }, 0);

        let window = unsafe {
            CreateWindowExA(
                WINDOW_EX_STYLE(0),
                class_name_ptr,
                PSTR(b"NIH-plug event loop\0".as_ptr()),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                HWND(0),
                HMENU(0),
                HINSTANCE(0),
                Arc::into_raw(tasks.clone()) as *const c_void,
            )
        };
        assert!(!window.is_invalid());

        Self {
            executor,
            main_thread_id: thread::current().id(),
            message_window: window,
            message_window_class_name: class_name,
            tasks,
        }
    }

    fn do_maybe_async(&self, task: T) -> bool {
        if self.is_main_thread() {
            match self.executor.upgrade() {
                Some(e) => {
                    unsafe { e.execute(task) };
                    true
                }
                None => {
                    nih_log!("The executor doesn't exist but somehow it's still submitting tasks, this shouldn't be possible!");
                    false
                }
            }
        } else {
            let success = self.tasks.push(task).is_ok();
            if success {
                todo!("Wake up the window");
            }

            success
        }
    }

    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread_id
    }
}

impl<T, E> Drop for WindowsEventLoop<T, E> {
    fn drop(&mut self) {
        unsafe { CloseWindow(self.message_window) };
        unsafe {
            UnregisterClassA(
                PSTR(self.message_window_class_name.as_bytes_with_nul().as_ptr()),
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
    eprintln!("Hello from the window proc!");

    todo!("Clean up the Arc (that got turned into a raw pointe) with the window");
    todo!("Handle messages");

    DefWindowProcA(handle, message, wparam, lparam)
}
