//! Traits for working with plugin editors.

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use std::any::Any;
use std::ffi::c_void;
use std::sync::Arc;

use crate::prelude::GuiContext;

/// An editor for a [`Plugin`][crate::prelude::Plugin].
pub trait Editor: Send {
    /// Create an instance of the plugin's editor and embed it in the parent window. As explained in
    /// [`Plugin::editor()`][crate::prelude::Plugin::editor()], you can then read the parameter
    /// values directly from your [`Params`][crate::prelude::Params] object, and modifying the
    /// values can be done using the functions on the [`ParamSetter`][crate::prelude::ParamSetter].
    /// When you change a parameter value that way it will be broadcasted to the host and also
    /// updated in your [`Params`][crate::prelude::Params] struct.
    ///
    /// This function should return a handle to the editor, which will be dropped when the editor
    /// gets closed. Implement the [`Drop`] trait on the returned handle if you need to explicitly
    /// handle the editor's closing behavior.
    ///
    /// If [`set_scale_factor()`][Self::set_scale_factor()] has been called, then any created
    /// windows should have their sizes multiplied by that factor.
    ///
    /// The wrapper guarantees that a previous handle has been dropped before this function is
    /// called again.
    //
    // TODO: Think of how this would work with the event loop. On Linux the wrapper must provide a
    //       timer using VST3's `IRunLoop` interface, but on Window and macOS the window would
    //       normally register its own timer. Right now we just ignore this because it would
    //       otherwise be basically impossible to have this still be GUI-framework agnostic. Any
    //       callback that deos involve actual GUI operations will still be spooled to the IRunLoop
    //       instance.
    // TODO: This function should return an `Option` instead. Right now window opening failures are
    //       always fatal. This would need to be fixed in baseview first.
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send>;

    /// Returns the (current) size of the editor in pixels as a `(width, height)` pair. This size
    /// must be reported in _logical pixels_, i.e. the size before being multiplied by the DPI
    /// scaling factor to get the actual physical screen pixels.
    fn size(&self) -> (u32, u32);

    /// Set the DPI scaling factor, if supported. The plugin APIs don't make any guarantees on when
    /// this is called, but for now just assume it will be the first function that gets called
    /// before creating the editor. If this is set, then any windows created by this editor should
    /// have their sizes multiplied by this scaling factor on Windows and Linux.
    ///
    /// Right now this is never called on macOS since DPI scaling is built into the operating system
    /// there.
    fn set_scale_factor(&self, factor: f32) -> bool;

    /// Called whenever a specific parameter's value has changed while the editor is open. You don't
    /// need to do anything with this, but this can be used to force a redraw when the host sends a
    /// new value for a parameter or when a parameter change sent to the host gets processed.
    fn param_value_changed(&self, id: &str, normalized_value: f32);

    /// Called whenever a specific parameter's monophonic modulation value has changed while the
    /// editor is open.
    fn param_modulation_changed(&self, id: &str, modulation_offset: f32);

    /// Called whenever one or more parameter values or modulations have changed while the editor is
    /// open. This may be called in place of [`param_value_changed()`][Self::param_value_changed()]
    /// when multiple parameter values hcange at the same time. For example, when a preset is
    /// loaded.
    fn param_values_changed(&self);

    // TODO: Reconsider adding a tick function here for the Linux `IRunLoop`. To keep this platform
    //       and API agnostic, add a way to ask the GuiContext if the wrapper already provides a
    //       tick function. If it does not, then the Editor implementation must handle this by
    //       itself. This would also need an associated `PREFERRED_FRAME_RATE` constant.
    // TODO: Host->Plugin resizing
}

/// A raw window handle for platform and GUI framework agnostic editors. This implements
/// [`HasRawWindowHandle`] so it can be used directly with GUI libraries that use the same
/// [`raw_window_handle`] version. If the library links against a different version of
/// `raw_window_handle`, then you'll need to wrap around this type and implement the trait yourself.
#[derive(Debug, Clone, Copy)]
pub enum ParentWindowHandle {
    /// The ID of the host's parent window. Used with X11.
    X11Window(u32),
    /// A handle to the host's parent window. Used only on macOS.
    AppKitNsView(*mut c_void),
    /// A handle to the host's parent window. Used only on Windows.
    Win32Hwnd(*mut c_void),
}

unsafe impl HasRawWindowHandle for ParentWindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match *self {
            ParentWindowHandle::X11Window(window) => {
                let mut handle = raw_window_handle::XcbWindowHandle::empty();
                handle.window = window;
                RawWindowHandle::Xcb(handle)
            }
            ParentWindowHandle::AppKitNsView(ns_view) => {
                let mut handle = raw_window_handle::AppKitWindowHandle::empty();
                handle.ns_view = ns_view;
                RawWindowHandle::AppKit(handle)
            }
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                let mut handle = raw_window_handle::Win32WindowHandle::empty();
                handle.hwnd = hwnd;
                RawWindowHandle::Win32(handle)
            }
        }
    }
}
