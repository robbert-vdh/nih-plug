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

//! [egui](https://github.com/emilk/egui) editor support for NIH plug.
//!
//! TODO: Proper usage example

use baseview::gl::GlConfig;
use baseview::{Size, WindowHandle, WindowOpenOptions, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui::CtxRef;
use egui_baseview::EguiWindow;
use nih_plug::{Editor, ParamSetter, ParentWindowHandle};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Re-export for convenience.
pub use egui;

/// Create an [Editor] instance using an [::egui] GUI. Using the user state parameter is optional,
/// but it can be useful for keeping track of some temporary GUI-only settings. See the `gui_gain`
/// example for more information on how to use this. The [EguiState] passed to this function
/// contains the GUI's intitial size, and this is kept in sync whenever the GUI gets resized. You
/// can also use this to know if the GUI is open, so you can avoid performing potentially expensive
/// calculations while the GUI is not open. If you want this size to be persisted when restoring a
/// plugin instance, then you can store it in a `#[persist]` field on your parameters struct.
///
/// See [EguiState::from_size()].
//
// TODO: DPI scaling, this needs to be implemented on the framework level
pub fn create_egui_editor<T, U>(
    egui_state: Arc<EguiState>,
    user_state: T,
    update: U,
) -> Option<Box<dyn Editor>>
where
    T: 'static + Send + Sync,
    U: Fn(&CtxRef, &ParamSetter, &mut T) + 'static + Send + Sync,
{
    Some(Box::new(EguiEditor {
        egui_state,
        user_state: Arc::new(RwLock::new(user_state)),
        update: Arc::new(update),
    }))
}

// TODO: Once we add resizing, we may want to be able to remember the GUI size. In that case we need
//       to make this serializable (only restoring the size of course) so it can be persisted.
pub struct EguiState {
    size: AtomicCell<(u32, u32)>,
    open: AtomicBool,
}

impl EguiState {
    /// Initialize the GUI's state. This is passed to [create_egui_editor()].
    pub fn from_size(width: u32, height: u32) -> Arc<EguiState> {
        Arc::new(EguiState {
            size: AtomicCell::new((width, height)),
            open: AtomicBool::new(false),
        })
    }

    /// Return a `(width, height)` pair for the current size of the GUI.
    pub fn size(&self) -> (u32, u32) {
        self.size.load()
    }

    /// Whether the GUI is currently visible.
    // Called `is_open()` instead of `open()` to avoid the ambiguity.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

/// An [Editor] implementation that calls an egui draw loop.
struct EguiEditor<T> {
    egui_state: Arc<EguiState>,
    /// The plugin's state. This is kept in between editor openenings.
    user_state: Arc<RwLock<T>>,
    update: Arc<dyn Fn(&CtxRef, &ParamSetter, &mut T) + 'static + Send + Sync>,
}

impl<T> Editor for EguiEditor<T>
where
    T: 'static + Send + Sync,
{
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn nih_plug::GuiContext>,
    ) -> Box<dyn std::any::Any> {
        let update = self.update.clone();
        let state = self.user_state.clone();

        let (width, height) = self.egui_state.size();
        let window = EguiWindow::open_parented(
            &parent,
            WindowOpenOptions {
                title: String::from("egui window"),
                size: Size::new(width as f64, height as f64),
                // TODO: What happens when we use the system scale factor here? I'd assume this
                //       would work everywhere, even if the window may be tiny in some cases.
                scale: WindowScalePolicy::ScaleFactor(1.0),
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
            |_, _, _| {},
            move |egui_ctx, queue, state| {
                let setter = ParamSetter::new(context.as_ref());

                // For now, just always redraw. Most plugin GUIs have meters, and those almost always
                // need a redraw. Later we can try to be a bit more sophisticated about this. Without
                // this we would also have a blank GUI when it gets first opened because most DAWs open
                // their GUI while the window is still unmapped.
                // TODO: Are there other useful parts of this queue we could pass to thep lugin?
                queue.request_repaint();
                (update)(egui_ctx, &setter, &mut state.write());
            },
        )
        .expect("We provided an OpenGL config, did we not?");

        self.egui_state.open.store(true, Ordering::Release);
        Box::new(EguiEditorHandle {
            egui_state: self.egui_state.clone(),
            window,
        })
    }

    fn size(&self) -> (u32, u32) {
        self.egui_state.size()
    }
}

/// The window handle used for [EguiEditor].
struct EguiEditorHandle {
    egui_state: Arc<EguiState>,
    window: WindowHandle,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl Send for EguiEditorHandle {}
unsafe impl Sync for EguiEditorHandle {}

impl Drop for EguiEditorHandle {
    fn drop(&mut self) {
        self.egui_state.open.store(false, Ordering::Release);
        // XXX: This should automatically happen when the handle gets dropped, but apparently not
        self.window.close();
    }
}
