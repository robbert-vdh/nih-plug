use atomic_float::AtomicF32;
use parking_lot::{Mutex, RwLock};
use std::any::Any;
use std::ffi::{c_void, CStr};
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use vst3::Steinberg::{
    char16, int16, kInvalidArgument, kNotImplemented, kResultFalse, kResultOk, tresult, FIDString,
    TBool,
};
use vst3::Steinberg::{
    IPlugFrame, IPlugFrameTrait, IPlugView, IPlugViewContentScaleSupport,
    IPlugViewContentScaleSupportTrait, IPlugViewContentScaleSupport_::ScaleFactor, IPlugViewTrait,
    ViewRect,
};
use vst3::{Class, ComPtr, ComRef, ComWrapper};

use super::inner::{Task, WrapperInner};
use crate::plugin::vst3::Vst3Plugin;
use crate::prelude::{Editor, ParentWindowHandle};

// Thanks for putting this behind a platform-specific ifdef...
// NOTE: This should also be used on the BSDs, but vst3-sys exposes these interfaces only for Linux
#[cfg(target_os = "linux")]
use {
    crate::event_loop::{EventLoop, MainThreadExecutor, TASK_QUEUE_CAPACITY},
    crossbeam::queue::ArrayQueue,
    libc,
    vst3::Steinberg::Linux::{
        FileDescriptor, IEventHandler, IEventHandlerTrait, IRunLoop, IRunLoopTrait,
    },
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

/// FIXME: vst3-sys does not allow you to conditionally define fields with #[cfg()], so this is a
///        workaround to define the field outside of the struct
#[cfg(target_os = "linux")]
struct RunLoopEventHandlerWrapper<P: Vst3Plugin>(RwLock<Option<Box<RunLoopEventHandler<P>>>>);
#[cfg(not(target_os = "linux"))]
struct RunLoopEventHandlerWrapper<P: Vst3Plugin>(std::marker::PhantomData<P>);

/// The plugin's [`IPlugView`] instance created in [`IEditController::create_view()`] if `P` has an
/// editor. This is managed separately so the lifetime bounds match up.
pub(crate) struct WrapperView<P: Vst3Plugin> {
    inner: Arc<WrapperInner<P>>,
    editor: Arc<Mutex<Box<dyn Editor>>>,
    editor_handle: RwLock<Option<Box<dyn Any>>>,

    /// The `IPlugFrame` instance passed by the host during [IPlugView::set_frame()].
    plug_frame: RwLock<Option<ComPtr<IPlugFrame>>>,
    /// Allows handling events events on the host's GUI thread when using Linux. Needed because
    /// otherwise REAPER doesn't like us very much. The event handler could be implemented directly
    /// on this object but vst3-sys does not let us conditionally implement interfaces.
    run_loop_event_handler: RunLoopEventHandlerWrapper<P>,

    /// The DPI scaling factor as passed to the [IPlugViewContentScaleSupport::set_scale_factor()]
    /// function. Defaults to 1.0, and will be kept there on macOS. When reporting and handling size
    /// the sizes communicated to and from the DAW should be scaled by this factor since NIH-plug's
    /// APIs only deal in logical pixels.
    scaling_factor: AtomicF32,
}

impl<P: Vst3Plugin> Class for WrapperView<P> {
    type Interfaces = (IPlugView, IPlugViewContentScaleSupport);
}

/// Allow handling tasks on the host's GUI thread on Linux. This doesn't need to be a separate
/// struct, but vst3-sys does not let us implement interfaces conditionally and the interface is
/// only exposed when compiling on Linux. The struct will register itself when calling
/// [`RunLoopEventHandler::new()`] and it will unregister itself when it gets dropped.
#[cfg(target_os = "linux")]
struct RunLoopEventHandler<P: Vst3Plugin> {
    /// We need access to the inner wrapper so we that we can post any outstanding tasks there when
    /// this object gets dropped so no work is lost.
    inner: Arc<WrapperInner<P>>,

    /// The host's run loop interface. This lets us run tasks on the same thread as the host's UI.
    run_loop: ComPtr<IRunLoop>,

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
    tasks: ArrayQueue<Task<P>>,
}

impl<P: Vst3Plugin> WrapperView<P> {
    pub fn new(inner: Arc<WrapperInner<P>>, editor: Arc<Mutex<Box<dyn Editor>>>) -> Self {
        Self {
            inner,
            editor,
            editor_handle: RwLock::new(None),
            plug_frame: RwLock::new(None),
            #[cfg(target_os = "linux")]
            run_loop_event_handler: RunLoopEventHandlerWrapper(RwLock::new(None)),
            #[cfg(not(target_os = "linux"))]
            run_loop_event_handler: RunLoopEventHandlerWrapper(Default::default()),
            scaling_factor: AtomicF32::new(1.0),
        }
    }

    /// Ask the host to resize the view to the size specified by [`Editor::size()`]. Will return false
    /// if the host doesn't like you. This **needs** to be run from the GUI thread.
    ///
    /// # Safety
    ///
    /// May cause memory corruption in Linux REAPER when called from outside of the `IRunLoop`.
    #[must_use]
    pub unsafe fn request_resize(this: &ComWrapper<Self>) -> bool {
        // Don't do anything if the editor is not open, because that would be strange
        if this
            .editor_handle
            .try_read()
            .map(|e| e.is_none())
            .unwrap_or(true)
        {
            return false;
        }

        match &*this.plug_frame.read() {
            Some(plug_frame) => {
                let (unscaled_width, unscaled_height) = this.editor.lock().size();
                let scaling_factor = this.scaling_factor.load(Ordering::Relaxed);
                let mut size = ViewRect {
                    left: 0,
                    top: 0,
                    right: (unscaled_width as f32 * scaling_factor).round() as i32,
                    bottom: (unscaled_height as f32 * scaling_factor).round() as i32,
                };

                let plug_view = this.as_com_ref::<IPlugView>().unwrap();
                let result = plug_frame.resizeView(plug_view.as_ptr(), &mut size);

                debug_assert_eq!(
                    result, kResultOk,
                    "The host denied the resize, we currently don't handle this for VST3 plugins"
                );

                result == kResultOk
            }
            None => false,
        }
    }

    /// If the host supports `IRunLoop`, then this will post the task to a task queue that will be
    /// run on the host's UI thread. If not, then this will return an `Err` value containing the
    /// task so it can be run elsewhere.
    #[cfg(target_os = "linux")]
    pub fn do_maybe_in_run_loop(&self, task: Task<P>) -> Result<(), Task<P>> {
        match &*self.run_loop_event_handler.0.read() {
            Some(run_loop) => run_loop.post_task(task),
            None => Err(task),
        }
    }

    /// If the host supports `IRunLoop`, then this will post the task to a task queue that will be
    /// run on the host's UI thread. If not, then this will return an `Err` value containing the
    /// task so it can be run elsewhere.
    #[cfg(not(target_os = "linux"))]
    pub fn do_maybe_in_run_loop(&self, task: Task<P>) -> Result<(), Task<P>> {
        Err(task)
    }
}

#[cfg(target_os = "linux")]
impl<P: Vst3Plugin> RunLoopEventHandler<P> {
    pub fn new(inner: Arc<WrapperInner<P>>, run_loop: ComPtr<IRunLoop>) -> Box<Self> {
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

        let handler = Box::new(RunLoopEventHandler {
            inner,
            run_loop,
            socket_read_fd,
            socket_write_fd,
            tasks: ArrayQueue::new(TASK_QUEUE_CAPACITY),
        });

        let handler_ptr: *mut IEventHandler = &handler as *const _ as *mut _;

        assert_eq!(
            unsafe {
                handler
                    .run_loop
                    .registerEventHandler(handler_ptr, handler.socket_read_fd)
            },
            kResultOk
        );

        handler
    }

    /// Post a task to the tasks queue so it will be run on the host's GUI thread later. Returns the
    /// task if the queue is full and the task could not be posted.
    pub fn post_task(&self, task: Task<P>) -> Result<(), Task<P>> {
        self.tasks.push(task)?;

        // We need to use a Unix domain socket to let the host know to call our event handler. In
        // theory eventfd would be more suitable here, but Ardour does not support that. This is
        // read again in `Self::on_fd_is_set()`.
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

impl<P: Vst3Plugin> IPlugViewTrait for WrapperView<P> {
    #[cfg(all(target_family = "unix", not(target_os = "macos")))]
    unsafe fn isPlatformTypeSupported(&self, type_: FIDString) -> tresult {
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
    unsafe fn isPlatformTypeSupported(&self, type_: FIDString) -> tresult {
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
    unsafe fn isPlatformTypeSupported(&self, type_: FIDString) -> tresult {
        let type_ = CStr::from_ptr(type_);
        match type_.to_str() {
            Ok(type_) if type_ == VST3_PLATFORM_HWND => kResultOk,
            _ => {
                nih_debug_assert_failure!("Invalid window handle type: {:?}", type_);
                kResultFalse
            }
        }
    }

    unsafe fn attached(&self, parent: *mut c_void, type_: FIDString) -> tresult {
        let mut editor_handle = self.editor_handle.write();
        if editor_handle.is_none() {
            let type_ = CStr::from_ptr(type_);
            let parent_handle = match type_.to_str() {
                Ok(type_) if type_ == VST3_PLATFORM_X11_WINDOW => {
                    ParentWindowHandle::X11Window(parent as usize as u32)
                }
                Ok(type_) if type_ == VST3_PLATFORM_NSVIEW => {
                    ParentWindowHandle::AppKitNsView(parent)
                }
                Ok(type_) if type_ == VST3_PLATFORM_HWND => ParentWindowHandle::Win32Hwnd(parent),
                _ => {
                    nih_debug_assert_failure!("Unknown window handle type: {:?}", type_);
                    return kInvalidArgument;
                }
            };

            *editor_handle = Some(
                self.editor
                    .lock()
                    .spawn(parent_handle, self.inner.clone().make_gui_context()),
            );
            // *self.inner.plug_view.write() = Some(ObjectPtr::from(self));

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

    unsafe fn onWheel(&self, _distance: f32) -> tresult {
        // We'll let the plugin use the OS' input mechanisms because not all DAWs (or very few
        // actually) implement these functions
        kNotImplemented
    }

    unsafe fn onKeyDown(&self, _key: char16, _key_code: int16, _modifiers: int16) -> tresult {
        kNotImplemented
    }

    unsafe fn onKeyUp(&self, _key: char16, _key_code: int16, _modifiers: int16) -> tresult {
        kNotImplemented
    }

    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        check_null_ptr!(size);

        *size = mem::zeroed();

        // TODO: This is technically incorrect during resizing, this should still report the old
        //       size until `.on_size()` has been called. We should probably only bother fixing this
        //       if it turns out to be an issue.
        let (unscaled_width, unscaled_height) = self.editor.lock().size();
        let scaling_factor = self.scaling_factor.load(Ordering::Relaxed);
        let size = &mut *size;
        size.left = 0;
        size.right = (unscaled_width as f32 * scaling_factor).round() as i32;
        size.top = 0;
        size.bottom = (unscaled_height as f32 * scaling_factor).round() as i32;

        kResultOk
    }

    unsafe fn onSize(&self, new_size: *mut ViewRect) -> tresult {
        check_null_ptr!(new_size);

        // TODO: Implement Host->Plugin resizing
        let (unscaled_width, unscaled_height) = self.editor.lock().size();
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

    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        kNotImplemented
    }

    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        match ComRef::from_raw(frame) {
            Some(frame) => {
                // On Linux the host will expose another interface that lets us run code on the
                // host's GUI thread. REAPER will segfault when we don't do this for resizes.
                #[cfg(target_os = "linux")]
                {
                    *self.run_loop_event_handler.0.write() = frame
                        .cast()
                        .map(|run_loop| RunLoopEventHandler::new(self.inner.clone(), run_loop));
                }
                // *self.plug_frame.write() = Some(ComPtr::from(frame));
            }
            None => {
                #[cfg(target_os = "linux")]
                {
                    *self.run_loop_event_handler.0.write() = None;
                }
                *self.plug_frame.write() = None;
            }
        }

        kResultOk
    }

    unsafe fn canResize(&self) -> tresult {
        // TODO: Implement Host->Plugin resizing
        kResultFalse
    }

    unsafe fn checkSizeConstraint(&self, rect: *mut ViewRect) -> tresult {
        check_null_ptr!(rect);

        // TODO: Implement Host->Plugin resizing
        if (*rect).right - (*rect).left > 0 && (*rect).bottom - (*rect).top > 0 {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

impl<P: Vst3Plugin> IPlugViewContentScaleSupportTrait for WrapperView<P> {
    unsafe fn setContentScaleFactor(&self, factor: ScaleFactor) -> tresult {
        // TODO: So apparently Ableton Live doesn't call this function. Right now we'll hardcode the
        //       default scale to 1.0 on Linux and Windows since we can't easily get the scale from
        //       baseview. A better alternative would be to do the fallback DPI scale detection
        //       within NIH-plug itself. Then we can still only use baseview's system scale policy
        //       on macOS and both the editor implementation and the wrappers would know about the
        //       correct scale.

        // On macOS scaling is done by the OS, and all window sizes are in logical pixels
        if cfg!(target_os = "macos") {
            nih_debug_assert_failure!("Ignoring host request to set explicit DPI scaling factor");
            return kResultFalse;
        }

        if self.editor.lock().set_scale_factor(factor) {
            self.scaling_factor.store(factor, Ordering::Relaxed);
            kResultOk
        } else {
            kResultFalse
        }
    }
}

#[cfg(target_os = "linux")]
impl<P: Vst3Plugin> IEventHandlerTrait for RunLoopEventHandler<P> {
    unsafe fn onFDIsSet(&self, _fd: FileDescriptor) {
        // There should be a one-to-one correlation to bytes being written to `self.socket_read_fd`
        // and events being pushed to `self.tasks`, but because the process of pushing a task and
        // notifying this thread through the socket is not atomic we can't reliably just read a byte
        // from this socket for every task we process. For instance, if `Self::post_task()` gets
        // called while this loop is already running, it could happen that we pop and execute the
        // task before the byte gets written to the socket. To avoid this, we'll clear the socket
        // upfront, and then execute the tasks afterwards. If this situation does happen, then the
        // worst thing that can happen is that this function is called a second time while
        // `self.tasks()` is already empty.
        let mut notify_value = [0; 32];
        loop {
            let read_result = libc::read(
                self.socket_read_fd,
                &mut notify_value as *mut _ as *mut c_void,
                std::mem::size_of_val(&notify_value),
            );

            // If after the first loop the socket contains no more data, then the `read()` call will
            // return -1 and `errno` will have been set to `EAGAIN`
            if read_result <= 0 {
                break;
            }
        }

        // This gets called from the host's UI thread because we wrote some bytes to the Unix domain
        // socket. We'll read that data from the socket again just to make REAPER happy.
        while let Some(task) = self.tasks.pop() {
            self.inner.execute(task, true);
        }
    }
}

#[cfg(target_os = "linux")]
impl<P: Vst3Plugin> Drop for RunLoopEventHandler<P> {
    fn drop(&mut self) {
        // When this object gets dropped and there are still unprocessed tasks left, then we'll
        // handle those in the regular event loop so no work gets lost
        let mut posting_failed = false;
        while let Some(task) = self.tasks.pop() {
            posting_failed |= !self
                .inner
                .event_loop
                .borrow()
                .as_ref()
                .unwrap()
                .schedule_gui(task);
        }

        if posting_failed {
            nih_debug_assert_failure!(
                "Outstanding tasks have been dropped when closing the editor as the task queue \
                 was full"
            );
        }

        unsafe {
            libc::close(self.socket_read_fd);
            libc::close(self.socket_write_fd);
        }

        unsafe {
            self.run_loop
                .unregisterEventHandler(&self as *const _ as *mut _);
        }
    }
}
