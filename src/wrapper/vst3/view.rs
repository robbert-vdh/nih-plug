use atomic_float::AtomicF32;
use parking_lot::RwLock;
use raw_window_handle::RawWindowHandle;
use std::any::Any;
use std::ffi::{c_void, CStr};
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use vst3_sys::base::{kInvalidArgument, kResultFalse, kResultOk, tresult, TBool};
use vst3_sys::gui::{IPlugFrame, IPlugView, IPlugViewContentScaleSupport, ViewRect};
use vst3_sys::utils::SharedVstPtr;
use vst3_sys::VST3;

use super::inner::WrapperInner;
use super::util::{ObjectPtr, VstPtr};
use crate::plugin::{Editor, ParentWindowHandle, Vst3Plugin};

// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;

// Thanks for putting this behind a platform-specific ifdef...
// NOTE: This should also be used on the BSDs, but vst3-sys exposes these interfaces only for Linux
#[cfg(target_os = "linux")]
use {
    super::inner::Task,
    crate::event_loop::{EventLoop, MainThreadExecutor, TASK_QUEUE_CAPACITY},
    crossbeam::queue::ArrayQueue,
    libc,
    vst3_sys::gui::linux::{FileDescriptor, IEventHandler, IRunLoop},
};

// Window handle type constants missing from vst3-sys
#[allow(unused)]
const VST3_PLATFORM_HWND: &str = "HWND";
#[allow(unused)]
const VST3_PLATFORM_HIVIEW: &str = "HIView";
#[allow(unused)]
const VST3_PLATFORM_NSVIEW: &str = "NSView";
#[allow(unused)]
const VST3_PLATFORM_UIVIEW: &str = "UIView";
#[allow(unused)]
const VST3_PLATFORM_X11_WINDOW: &str = "X11EmbedWindowID";

/// The plugin's [`IPlugView`] instance created in [`IEditController::create_view()`] if `P` has an
/// editor. This is managed separately so the lifetime bounds match up.
#[VST3(implements(IPlugView, IPlugViewContentScaleSupport))]
pub(crate) struct WrapperView<P: Vst3Plugin> {
    inner: Arc<WrapperInner<P>>,
    editor: Arc<dyn Editor>,
    editor_handle: RwLock<Option<Box<dyn Any>>>,

    /// The `IPlugFrame` instance passed by the host during [IPlugView::set_frame()].
    plug_frame: RwLock<Option<VstPtr<dyn IPlugFrame>>>,
    /// Allows handling events events on the host's GUI thread when using Linux. Needed because
    /// otherwise REAPER doesn't like us very much. The event handler could be implemented directly
    /// on this object but vst3-sys does not let us conditionally implement interfaces.
    #[cfg(target_os = "linux")]
    run_loop_event_handler: RwLock<Option<Box<RunLoopEventHandler<P>>>>,

    /// The DPI scaling factor as passed to the [IPlugViewContentScaleSupport::set_scale_factor()]
    /// function. Defaults to 1.0, and will be kept there on macOS. When reporting and handling size
    /// the sizes communicated to and from the DAW should be scaled by this factor since NIH-plug's
    /// APIs only deal in logical pixels.
    scaling_factor: AtomicF32,
}

/// Allow handling tasks on the host's GUI thread on Linux. This doesn't need to be a separate
/// struct, but vst3-sys does not let us implement interfaces conditionally and the interface is
/// only exposed when compiling on Linux. The struct will register itself when calling
/// [`RunLoopEventHandler::new()`] and it will unregister itself when it gets dropped.
#[cfg(target_os = "linux")]
#[VST3(implements(IEventHandler))]
struct RunLoopEventHandler<P: Vst3Plugin> {
    /// We need access to the inner wrapper so we that we can post any outstanding tasks there when
    /// this object gets dropped so no work is lost.
    inner: Arc<WrapperInner<P>>,

    /// The host's run lopp interface. This lets us run tasks on the same thread as the host's UI.
    run_loop: VstPtr<dyn IRunLoop>,

    /// We need a Unix domain socket the host can poll to know that we have an event to handle. In
    /// theory eventfd would be much better suited for this, but Ardour doesn't respond to fds that
    /// aren't sockets. So instead, we will write a single byte here for every message we should
    /// handle.
    socket_read_fd: i32,
    socket_write_fd: i32,

    /// A queue of tasks that still need to be performed. Because CLAP lets the plugin request a
    /// host callback directly, we don't need to use the OsEventLoop we use in our other plugin
    /// implementations. Instead, we'll post tasks to this queue, ask the host to call
    /// [`on_main_thread()`][Self::on_main_thread()] on the main thread, and then continue to pop
    /// tasks off this queue there until it is empty.
    tasks: ArrayQueue<Task>,
}

impl<P: Vst3Plugin> WrapperView<P> {
    pub fn new(inner: Arc<WrapperInner<P>>, editor: Arc<dyn Editor>) -> Box<Self> {
        Self::allocate(
            inner,
            editor,
            RwLock::new(None),
            RwLock::new(None),
            #[cfg(target_os = "linux")]
            RwLock::new(None),
            AtomicF32::new(1.0),
        )
    }

    /// Ask the host to resize the view to the size specified by [Editor::size()]. Will return false
    /// if the host doesn't like you.
    pub fn request_resize(&self) -> bool {
        // Don't do anything if the editor is not open, because that would be strange
        if self
            .editor_handle
            .try_read()
            .map(|e| e.is_none())
            .unwrap_or(true)
        {
            return false;
        }

        match &*self.plug_frame.read() {
            Some(plug_frame) => {
                let (unscaled_width, unscaled_height) = self.editor.size();
                let scaling_factor = self.scaling_factor.load(Ordering::Relaxed);
                let mut size = ViewRect {
                    right: (unscaled_width as f32 * scaling_factor).round() as i32,
                    bottom: (unscaled_height as f32 * scaling_factor).round() as i32,
                    ..Default::default()
                };

                // The argument types are a bit wonky here because you can't construct a
                // `SharedVstPtr`. This _should_ work however.
                // FIXME: Run this in the `IRonLoop` on Linux. Otherwise REAPER will be very cross
                //        with us.
                let plug_view: SharedVstPtr<dyn IPlugView> =
                    unsafe { mem::transmute(self.__iplugviewvptr) };
                let result = unsafe { plug_frame.resize_view(plug_view, &mut size) };

                result == kResultOk
            }
            None => false,
        }
    }

    /// If the host supports `IRunLoop`, then this will post the task to a task queue that will be
    /// run on the host's UI thread. If not, then this will return an `Err` value containing the
    /// task so it can be run elsewhere.
    #[cfg(target_os = "linux")]
    pub fn do_maybe_in_run_loop(&self, task: Task) -> Result<(), Task> {
        match &*self.run_loop_event_handler.read() {
            Some(run_loop) => run_loop.post_task(task),
            None => Err(task),
        }
    }

    /// If the host supports `IRunLoop`, then this will post the task to a task queue that will be
    /// run on the host's UI thread. If not, then this will return an `Err` value containing the
    /// task so it can be run elsewhere.
    #[cfg(not(target_os = "linux"))]
    #[must_use]
    pub fn do_maybe_in_run_loop(&self, task: Task) -> Result<(), Task> {
        Err(task)
    }
}

#[cfg(target_os = "linux")]
impl<P: Vst3Plugin> RunLoopEventHandler<P> {
    pub fn new(inner: Arc<WrapperInner<P>>, run_loop: VstPtr<dyn IRunLoop>) -> Box<Self> {
        let mut sockets = [0i32; 2];
        assert_eq!(
            unsafe {
                libc::socketpair(
                    libc::AF_UNIX,
                    libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                    0,
                    sockets.as_mut_ptr(),
                )
            },
            0
        );
        let [socket_read_fd, socket_write_fd] = sockets;

        // vst3-sys provides no way to convert to a SharedVstPtr, so, uh, yeah
        let handler = RunLoopEventHandler::allocate(
            inner,
            run_loop,
            socket_read_fd,
            socket_write_fd,
            ArrayQueue::new(TASK_QUEUE_CAPACITY),
        );
        let event_handler: SharedVstPtr<dyn IEventHandler> =
            unsafe { mem::transmute(handler.__ieventhandlervptr) };
        assert_eq!(
            unsafe {
                handler
                    .run_loop
                    .register_event_handler(event_handler, handler.socket_read_fd)
            },
            kResultOk
        );

        handler
    }

    /// Post a task to the tasks queue so it will be run on the host's GUI thread later. Returns the
    /// task if the queue is full and the task could not be posted.
    pub fn post_task(&self, task: Task) -> Result<(), Task> {
        let () = self.tasks.push(task)?;

        // We need to use a Unix domain socket to let the host know to call our event handler. In
        // theory eventfd would be more suitable here, but Ardour does not support that.
        // XXX: This can technically lead to a race condition if the host is currently calling
        //      `on_fd_is_set()` on another thread and the task has already been popped and executed
        //      and this value has not yet been written to the socket. Doing it the other way around
        //      gets you the other situation where the event handler could be run without the task
        //      being posted yet. In practice this won't cause any issues however.
        let notify_value = 1i8;
        const NOTIFY_VALUE_SIZE: usize = std::mem::size_of::<i8>();
        assert_eq!(
            unsafe {
                libc::write(
                    self.socket_write_fd,
                    &notify_value as *const _ as *const c_void,
                    NOTIFY_VALUE_SIZE,
                )
            },
            NOTIFY_VALUE_SIZE as isize
        );

        Ok(())
    }
}

impl<P: Vst3Plugin> IPlugView for WrapperView<P> {
    #[cfg(all(target_family = "unix", not(target_os = "macos")))]
    unsafe fn is_platform_type_supported(&self, type_: vst3_sys::base::FIDString) -> tresult {
        let type_ = CStr::from_ptr(type_);
        match type_.to_str() {
            Ok(type_) if type_ == VST3_PLATFORM_X11_WINDOW => kResultOk,
            _ => {
                nih_debug_assert_failure!("Invalid window handle type: {:?}", type_);
                kResultFalse
            }
        }
    }

    #[cfg(target_os = "macos")]
    unsafe fn is_platform_type_supported(&self, type_: vst3_sys::base::FIDString) -> tresult {
        let type_ = CStr::from_ptr(type_);
        match type_.to_str() {
            Ok(type_) if type_ == VST3_PLATFORM_NSVIEW => kResultOk,
            _ => {
                nih_debug_assert_failure!("Invalid window handle type: {:?}", type_);
                kResultFalse
            }
        }
    }

    #[cfg(target_os = "windows")]
    unsafe fn is_platform_type_supported(&self, type_: vst3_sys::base::FIDString) -> tresult {
        let type_ = CStr::from_ptr(type_);
        match type_.to_str() {
            Ok(type_) if type_ == VST3_PLATFORM_HWND => kResultOk,
            _ => {
                nih_debug_assert_failure!("Invalid window handle type: {:?}", type_);
                kResultFalse
            }
        }
    }

    unsafe fn attached(&self, parent: *mut c_void, type_: vst3_sys::base::FIDString) -> tresult {
        let mut editor_handle = self.editor_handle.write();
        if editor_handle.is_none() {
            let type_ = CStr::from_ptr(type_);
            let handle = match type_.to_str() {
                #[cfg(all(target_family = "unix", not(target_os = "macos")))]
                Ok(type_) if type_ == VST3_PLATFORM_X11_WINDOW => {
                    let mut handle = raw_window_handle::XcbHandle::empty();
                    handle.window = parent as usize as u32;
                    RawWindowHandle::Xcb(handle)
                }
                #[cfg(target_os = "macos")]
                Ok(type_) if type_ == VST3_PLATFORM_NSVIEW => {
                    let mut handle = raw_window_handle::AppKitHandle::empty();
                    handle.ns_view = parent;
                    RawWindowHandle::AppKit(handle)
                }
                #[cfg(target_os = "windows")]
                Ok(type_) if type_ == VST3_PLATFORM_HWND => {
                    let mut handle = raw_window_handle::Win32Handle::empty();
                    handle.hwnd = parent;
                    RawWindowHandle::Win32(handle)
                }
                _ => {
                    nih_debug_assert_failure!("Unknown window handle type: {:?}", type_);
                    return kInvalidArgument;
                }
            };

            *editor_handle = Some(self.editor.spawn(
                ParentWindowHandle { handle },
                self.inner.clone().make_gui_context(),
            ));
            *self.inner.plug_view.write() = Some(ObjectPtr::from(self));

            kResultOk
        } else {
            nih_debug_assert_failure!(
                "Host tried to attach editor while the editor is already attached"
            );

            kResultFalse
        }
    }

    unsafe fn removed(&self) -> tresult {
        let mut editor_handle = self.editor_handle.write();
        if editor_handle.is_some() {
            *self.inner.plug_view.write() = None;
            *editor_handle = None;

            kResultOk
        } else {
            nih_debug_assert_failure!("Host tried to remove the editor without an active editor");

            kResultFalse
        }
    }

    unsafe fn on_wheel(&self, _distance: f32) -> tresult {
        // We'll let the plugin use the OS' input mechamisms because not all DAWs (or very few
        // actually) implement these functions
        kResultOk
    }

    unsafe fn on_key_down(
        &self,
        _key: vst3_sys::base::char16,
        _key_code: i16,
        _modifiers: i16,
    ) -> tresult {
        kResultOk
    }

    unsafe fn on_key_up(
        &self,
        _key: vst3_sys::base::char16,
        _key_code: i16,
        _modifiers: i16,
    ) -> tresult {
        kResultOk
    }

    unsafe fn get_size(&self, size: *mut ViewRect) -> tresult {
        check_null_ptr!(size);

        *size = mem::zeroed();

        // TODO: This is technically incorrect during resizing, this should still report the old
        //       size until `.on_size()` has been called. We should probably only bother fixing this
        //       if it turns out to be an issue.
        let (unscaled_width, unscaled_height) = self.editor.size();
        let scaling_factor = self.scaling_factor.load(Ordering::Relaxed);
        let size = &mut *size;
        size.left = 0;
        size.right = (unscaled_width as f32 * scaling_factor).round() as i32;
        size.top = 0;
        size.bottom = (unscaled_height as f32 * scaling_factor).round() as i32;

        kResultOk
    }

    unsafe fn on_size(&self, new_size: *mut ViewRect) -> tresult {
        check_null_ptr!(new_size);

        // TODO: Implement Host->Plugin resizing
        let (unscaled_width, unscaled_height) = self.editor.size();
        let scaling_factor = self.scaling_factor.load(Ordering::Relaxed);
        let (editor_width, editor_height) = (
            (unscaled_width as f32 * scaling_factor).round() as i32,
            (unscaled_height as f32 * scaling_factor).round() as i32,
        );

        let width = (*new_size).right - (*new_size).left;
        let height = (*new_size).bottom - (*new_size).top;
        if width == editor_width && height == editor_height {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn on_focus(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn set_frame(&self, frame: *mut c_void) -> tresult {
        // The correct argument type is missing from the bindings
        let frame: SharedVstPtr<dyn IPlugFrame> = mem::transmute(frame);
        match frame.upgrade() {
            Some(frame) => {
                // On Linux the host will expose another interface that lets us run code on the
                // host's GUI thread. REAPER will segfault when we don't do this for resizes.
                #[cfg(target_os = "linux")]
                {
                    *self.run_loop_event_handler.write() = frame.cast().map(|run_loop| {
                        RunLoopEventHandler::new(self.inner.clone(), VstPtr::from(run_loop))
                    });
                }
                *self.plug_frame.write() = Some(VstPtr::from(frame));
            }
            None => {
                #[cfg(target_os = "linux")]
                {
                    *self.run_loop_event_handler.write() = None;
                }
                *self.plug_frame.write() = None;
            }
        }

        kResultOk
    }

    unsafe fn can_resize(&self) -> tresult {
        // TODO: Implement Host->Plugin resizing
        kResultFalse
    }

    unsafe fn check_size_constraint(&self, rect: *mut ViewRect) -> tresult {
        check_null_ptr!(rect);

        // TODO: Implement Host->Plugin resizing
        if (*rect).right - (*rect).left > 0 && (*rect).bottom - (*rect).top > 0 {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

impl<P: Vst3Plugin> IPlugViewContentScaleSupport for WrapperView<P> {
    unsafe fn set_scale_factor(&self, factor: f32) -> tresult {
        // On macOS scaling is done by the OS, and all window sizes are in logical pixels
        if cfg!(target_os = "macos") {
            nih_debug_assert_failure!("Ignoring host request to set explicit DPI scaling factor");
            return kResultFalse;
        }

        if self.editor.set_scale_factor(factor) {
            self.scaling_factor.store(factor, Ordering::Relaxed);
            kResultOk
        } else {
            kResultFalse
        }
    }
}

#[cfg(target_os = "linux")]
impl<P: Vst3Plugin> IEventHandler for RunLoopEventHandler<P> {
    unsafe fn on_fd_is_set(&self, _fd: FileDescriptor) {
        // This gets called from the host's UI thread because we wrote some bytes to the Unix domain
        // socket. We'll read that data from the socket again just to make REAPER happy.
        while let Some(task) = self.tasks.pop() {
            self.inner.execute(task);

            let mut notify_value = 1i8;
            const NOTIFY_VALUE_SIZE: usize = std::mem::size_of::<i8>();
            assert_eq!(
                libc::read(
                    self.socket_read_fd,
                    &mut notify_value as *mut _ as *mut c_void,
                    NOTIFY_VALUE_SIZE
                ),
                NOTIFY_VALUE_SIZE as isize
            );
        }
    }
}

#[cfg(target_os = "linux")]
impl<P: Vst3Plugin> Drop for RunLoopEventHandler<P> {
    fn drop(&mut self) {
        // When this object gets dropped and there are still unprocssed tasks left, then we'll
        // handle those in the regular event loop so no work gets lost
        let mut posting_failed = false;
        while let Some(task) = self.tasks.pop() {
            posting_failed |= !unsafe {
                self.inner
                    .event_loop
                    .borrow()
                    .assume_init_ref()
                    .do_maybe_async(task)
            };
        }

        if posting_failed {
            nih_debug_assert_failure!(
                "Outstanding tasks have been dropped when clsoing \
                 the editor as the task queue was full"
            );
        }

        unsafe {
            libc::close(self.socket_read_fd);
            libc::close(self.socket_write_fd);
        }

        let event_handler: SharedVstPtr<dyn IEventHandler> =
            unsafe { mem::transmute(self.__ieventhandlervptr) };
        unsafe { self.run_loop.unregister_event_handler(event_handler) };
    }
}
