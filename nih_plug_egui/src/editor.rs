//! An [`Editor`] implementation for egui.

use baseview::gl::GlConfig;
use baseview::{Size, WindowHandle, WindowOpenOptions, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::Context;
use egui_baseview::EguiWindow;
use nih_plug::prelude::{Editor, GuiContext, ParamSetter, ParentWindowHandle};
use parking_lot::RwLock;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::EguiState;

/// An [`Editor`] implementation that calls an egui draw loop.
pub(crate) struct EguiEditor<T> {
    pub(crate) egui_state: Arc<EguiState>,
    /// The plugin's state. This is kept in between editor openenings.
    pub(crate) user_state: Arc<RwLock<T>>,

    /// The user's build function. Applied once at the start of the application.
    pub(crate) build: Arc<dyn Fn(&Context, &mut T) + 'static + Send + Sync>,
    /// The user's update function.
    pub(crate) update: Arc<dyn Fn(&Context, &ParamSetter, &mut T) + 'static + Send + Sync>,

    /// The scaling factor reported by the host, if any. On macOS this will never be set and we
    /// should use the system scaling factor instead.
    pub(crate) scaling_factor: AtomicCell<Option<f32>>,
}

/// This version of `baseview` uses a different version of `raw_window_handle than NIH-plug, so we
/// need to adapt it ourselves.
struct ParentWindowHandleAdapter(nih_plug::editor::ParentWindowHandle);

unsafe impl HasRawWindowHandle for ParentWindowHandleAdapter {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match self.0 {
            ParentWindowHandle::X11Window(window) => {
                let mut handle = raw_window_handle::XcbHandle::empty();
                handle.window = window;
                RawWindowHandle::Xcb(handle)
            }
            ParentWindowHandle::AppKitNsView(ns_view) => {
                let mut handle = raw_window_handle::AppKitHandle::empty();
                handle.ns_view = ns_view;
                RawWindowHandle::AppKit(handle)
            }
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                let mut handle = raw_window_handle::Win32Handle::empty();
                handle.hwnd = hwnd;
                RawWindowHandle::Win32(handle)
            }
        }
    }
}

impl<T> Editor for EguiEditor<T>
where
    T: 'static + Send + Sync,
{
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let build = self.build.clone();
        let update = self.update.clone();
        let state = self.user_state.clone();

        let (unscaled_width, unscaled_height) = self.egui_state.size();
        let scaling_factor = self.scaling_factor.load();
        let window = EguiWindow::open_parented(
            &ParentWindowHandleAdapter(parent),
            WindowOpenOptions {
                title: String::from("egui window"),
                // Baseview should be doing the DPI scaling for us
                size: Size::new(unscaled_width as f64, unscaled_height as f64),
                // NOTE: For some reason passing 1.0 here causes the UI to be scaled on macOS but
                //       not the mouse events.
                scale: scaling_factor
                    .map(|factor| WindowScalePolicy::ScaleFactor(factor as f64))
                    .unwrap_or(WindowScalePolicy::SystemScaleFactor),

                #[cfg(feature = "opengl")]
                gl_config: Some(GlConfig {
                    version: (3, 2),
                    red_bits: 8,
                    blue_bits: 8,
                    green_bits: 8,
                    alpha_bits: 8,
                    depth_bits: 24,
                    stencil_bits: 8,
                    samples: None,
                    srgb: true,
                    double_buffer: true,
                    vsync: true,
                    ..Default::default()
                }),
            },
            state,
            move |egui_ctx, _queue, state| build(egui_ctx, &mut state.write()),
            move |egui_ctx, _queue, state| {
                let setter = ParamSetter::new(context.as_ref());

                // For now, just always redraw. Most plugin GUIs have meters, and those almost always
                // need a redraw. Later we can try to be a bit more sophisticated about this. Without
                // this we would also have a blank GUI when it gets first opened because most DAWs open
                // their GUI while the window is still unmapped.
                egui_ctx.request_repaint();
                (update)(egui_ctx, &setter, &mut state.write());
            },
        );

        self.egui_state.open.store(true, Ordering::Release);
        Box::new(EguiEditorHandle {
            egui_state: self.egui_state.clone(),
            window,
        })
    }

    fn size(&self) -> (u32, u32) {
        self.egui_state.size()
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        // If the editor is currently open then the host must not change the current HiDPI scale as
        // we don't have a way to handle that. Ableton Live does this.
        if self.egui_state.is_open() {
            return false;
        }

        self.scaling_factor.store(Some(factor));
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        // As mentioned above, for now we'll always force a redraw to allow meter widgets to work
        // correctly. In the future we can use an `Arc<AtomicBool>` and only force a redraw when
        // that boolean is set.
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}

    fn param_values_changed(&self) {
        // Same
    }
}

/// The window handle used for [`EguiEditor`].
struct EguiEditorHandle {
    egui_state: Arc<EguiState>,
    window: WindowHandle,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl Send for EguiEditorHandle {}

impl Drop for EguiEditorHandle {
    fn drop(&mut self) {
        self.egui_state.open.store(false, Ordering::Release);
        // XXX: This should automatically happen when the handle gets dropped, but apparently not
        self.window.close();
    }
}
