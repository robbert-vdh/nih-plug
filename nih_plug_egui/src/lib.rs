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
use egui::CtxRef;
use egui_baseview::EguiWindow;
use nih_plug::{Editor, ParamSetter, ParentWindowHandle};
use parking_lot::RwLock;
use std::sync::Arc;

/// Re-export for convenience.
pub use crossbeam::atomic::AtomicCell;
pub use egui;

/// Create an [Editor] instance using an [::egui] GUI. Using the state is optional, but it can be
/// useful for keeping track of some temporary GUI-only settings. See the `gui_gain` example for
/// more information on how to use this. The size passed to this function is the GUI's intitial
/// size, and this is kept in sync whenever the GUI gets resized. If you want this size to be
/// persisted when restoring a plugin instance, then you can store it in a `#[persist]` field on
/// your parameters struct.
//
// TODO: DPI scaling, this needs to be implemented on the framework level
// TODO: Add some way for the plugin to check whether the GUI is open
pub fn create_egui_editor<T, U>(
    size: Arc<AtomicCell<(u32, u32)>>,
    initial_state: T,
    update: U,
) -> Option<Box<dyn Editor>>
where
    T: 'static + Send + Sync,
    U: Fn(&CtxRef, &ParamSetter, &mut T) + 'static + Send + Sync,
{
    Some(Box::new(EguiEditor {
        size,
        state: Arc::new(RwLock::new(initial_state)),
        update: Arc::new(update),
    }))
}

/// An [Editor] implementation that calls an egui draw loop.
struct EguiEditor<T> {
    size: Arc<AtomicCell<(u32, u32)>>,
    /// The plugin's state. This is kept in between editor openenings.
    state: Arc<RwLock<T>>,
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
        let state = self.state.clone();

        let (width, height) = self.size.load();
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

        Box::new(EguiEditorHandle { window })
    }

    fn size(&self) -> (u32, u32) {
        self.size.load()
    }
}

/// The window handle used for [EguiEditor].
struct EguiEditorHandle {
    window: WindowHandle,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl Send for EguiEditorHandle {}
unsafe impl Sync for EguiEditorHandle {}

impl Drop for EguiEditorHandle {
    fn drop(&mut self) {
        // XXX: This should automatically happen when the handle gets dropped, but apparently not
        self.window.close();
    }
}
