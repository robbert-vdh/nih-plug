// Diopser: a phase rotation plugin
// Copyright (C) 2021-2022 Robbert van der Helm
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

use nih_plug_vizia::vizia::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A custom toggleable button coupled to an `Arc<AtomicBool`. Otherwise this is very similar to the
/// param button.
#[derive(Lens)]
pub struct SafeModeButton<L: Lens<Target = Arc<AtomicBool>>> {
    lens: L,

    /// The number of (fractional) scrolled lines that have not yet been turned into parameter
    /// change events. This is needed to support trackpads with smooth scrolling.
    scrolled_lines: f32,
}

impl<L: Lens<Target = Arc<AtomicBool>>> SafeModeButton<L> {
    /// Creates a new button bound to the `Arc<AtomicBool>`.
    pub fn new<T>(cx: &mut Context, lens: L, label: impl Res<T>) -> Handle<Self>
    where
        T: ToString,
    {
        Self {
            lens: lens.clone(),
            scrolled_lines: 0.0,
        }
        .build(cx, move |cx| {
            Label::new(cx, label);
        })
        .checked(lens.map(|v| v.load(Ordering::Relaxed)))
        // We'll pretend this is a param-button, so this class is used for assigning a unique color
        .class("safe-mode")
    }
}

impl<L: Lens<Target = Arc<AtomicBool>>> View for SafeModeButton<L> {
    fn element(&self) -> Option<&'static str> {
        // Reuse the styling from param-button
        Some("param-button")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|window_event, meta| match window_event {
            // We don't need special double and triple click handling
            WindowEvent::MouseDown(MouseButton::Left)
            | WindowEvent::MouseDoubleClick(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                // We can just unconditionally toggle the boolean here
                let atomic = self.lens.get(cx);
                atomic.fetch_xor(true, Ordering::AcqRel);

                meta.consume();
            }
            WindowEvent::MouseScroll(_scroll_x, scroll_y) => {
                self.scrolled_lines += scroll_y;

                if self.scrolled_lines.abs() >= 1.0 {
                    let atomic = self.lens.get(cx);

                    if self.scrolled_lines >= 1.0 {
                        atomic.store(true, Ordering::SeqCst);
                        self.scrolled_lines -= 1.0;
                    } else {
                        atomic.store(false, Ordering::SeqCst);
                        self.scrolled_lines += 1.0;
                    }
                }

                meta.consume();
            }
            _ => {}
        });
    }
}
