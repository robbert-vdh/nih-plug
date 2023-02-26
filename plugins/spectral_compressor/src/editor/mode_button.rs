// Spectral Compressor: an FFT based compressor
// Copyright (C) 2021-2023 Robbert van der Helm
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

use std::sync::Arc;

use crossbeam::atomic::AtomicCell;
use nih_plug_vizia::vizia::prelude::*;

use super::EditorMode;

/// A custom toggleable button that allows changing between the collapsed and expanded editor modes.
pub struct EditorModeButton {
    mode: Arc<AtomicCell<EditorMode>>,
}

impl EditorModeButton {
    /// Creates a new button bound to the editor mode setting.
    pub fn new<L, T>(cx: &mut Context, lens: L, label: impl Res<T>) -> Handle<Self>
    where
        L: Lens<Target = Arc<AtomicCell<EditorMode>>>,
        T: ToString,
    {
        Self { mode: lens.get(cx) }
            .build(cx, move |cx| {
                Label::new(cx, label);
            })
            .checked(lens.map(|v| v.load() == EditorMode::VisualizerVisible))
            // We'll pretend this is a param-button, so this class is used for assigning a unique
            // color
            .class("editor-mode")
    }
}

impl View for EditorModeButton {
    fn element(&self) -> Option<&'static str> {
        // Reuse the styling from param-button
        Some("param-button")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|window_event, meta| match window_event {
            WindowEvent::MouseDown(MouseButton::Left)
            | WindowEvent::MouseDoubleClick(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                // TODO: Somehow change the way window sizes work in NIH-plug to be callback based.
                //       Now this can technically go out of sync if the GUI is closed between the
                //       mode changing and the resize actually getting processed
                let current_mode = self.mode.load();
                let new_mode = match current_mode {
                    EditorMode::Collapsed => EditorMode::VisualizerVisible,
                    EditorMode::VisualizerVisible => EditorMode::Collapsed,
                };

                match new_mode {
                    EditorMode::Collapsed => cx.set_window_size(WindowSize {
                        width: super::COLLAPSED_GUI_WIDTH,
                        height: super::GUI_HEIGHT,
                    }),
                    EditorMode::VisualizerVisible => cx.set_window_size(WindowSize {
                        width: super::EXPANDED_GUI_WIDTH,
                        height: super::GUI_HEIGHT,
                    }),
                }

                self.mode.store(new_mode);
                meta.consume();
            }
            // Mouse scrolling is intentionally not implemented here since it could be very easy to
            // do that by accident and that would cause the window to jump all over the place
            _ => {}
        });
    }
}
