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

use baseview::{Size, WindowHandle, WindowOpenOptions, WindowScalePolicy};
use egui::CtxRef;
use egui_baseview::{EguiWindow, RenderSettings, Settings};
use nih_plug::{Editor, EditorWindowHandle};
use std::sync::Arc;

/// Re-export for convenience.
pub use crossbeam::atomic::AtomicCell;
pub use egui;

/// Create an [Editor] instance using an [::egui] GUI. Using the state is optional, but it can be
/// useful for keeping track of some temporary GUI-only settings. See the `gui_gain` example for
/// more information on how to use this. The size passed to this function is the GUI's intiial size,
/// and this is kept in sync whenever the GUI gets resized. You should return the same size value in
/// your plugin' [nih_plug::Plugin::editor_size()] implementation.
//
// TODO: DPI scaling, this needs to be implemented on the framework level
// TODO: Opening the GUI causes an xrun, why?
// TODO: The current version of egui-baseview causes a SIGABRT you when you close the GUI
pub fn create_egui_editor<T, U>(
    parent: EditorWindowHandle,
    size: Arc<AtomicCell<(u32, u32)>>,
    initial_state: T,
    mut update: U,
) -> Option<Box<dyn Editor>>
where
    T: 'static + Send,
    U: FnMut(&CtxRef, &mut T) + 'static + Send,
{
    let (width, height) = size.load();
    let window = EguiWindow::open_parented(
        &parent,
        Settings {
            window: WindowOpenOptions {
                title: String::from("egui window"),
                size: Size::new(width as f64, height as f64),
                // TODO: What happens when we use the system scale factor here? I'd assume this
                //       would work everywhere, even if the window may be tiny in some cases.
                scale: WindowScalePolicy::ScaleFactor(1.0),
            },
            render_settings: RenderSettings {
                version: (3, 2),
                red_bits: 8,
                blue_bits: 8,
                green_bits: 8,
                // If the window was not created with the correct visual, then specifying 8 bits
                // here will cause creating the context to fail
                alpha_bits: 0,
                depth_bits: 24,
                stencil_bits: 8,
                samples: None,
                srgb: true,
                double_buffer: true,
                vsync: true,
                ..Default::default()
            },
        },
        initial_state,
        |_, _, _| {},
        move |egui_ctx, queue, state| {
            // For now, just always redraw. Most plugin GUIs have meters, and those almost always
            // need a redraw. Later we can try to be a bit more sophisticated about this. Without
            // this we would also have a blank GUI when it gets first opened because most DAWs open
            // their GUI while the window is still unmapped.
            // TODO: Are there other useful parts of this queue we could pass to thep lugin?
            queue.request_repaint();
            update(egui_ctx, state);
        },
    );

    // There's no error handling here, so let's just pray it worked
    if window.is_open() {
        Some(Box::new(EguiEditor {
            _window: window,
            size,
        }))
    } else {
        None
    }
}

/// An [Editor] implementation that calls an egui draw loop.
pub struct EguiEditor {
    _window: WindowHandle,
    size: Arc<AtomicCell<(u32, u32)>>,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl Send for EguiEditor {}
unsafe impl Sync for EguiEditor {}

impl Editor for EguiEditor {
    fn size(&self) -> (u32, u32) {
        self.size.load()
    }
}
