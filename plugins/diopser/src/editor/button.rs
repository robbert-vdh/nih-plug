// Diopser: a phase rotation plugin
// Copyright (C) 2021-2024 Robbert van der Helm
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

use super::SafeModeClamper;

/// A custom toggleable button that toggles safe mode whenever it is Alt+clicked. Otherwise this is
/// very similar to the param button.
#[derive(Lens)]
pub struct SafeModeButton<L: Lens<Target = SafeModeClamper>> {
    lens: L,

    /// The number of (fractional) scrolled lines that have not yet been turned into parameter
    /// change events. This is needed to support trackpads with smooth scrolling.
    scrolled_lines: f32,
}

impl<L: Lens<Target = SafeModeClamper>> SafeModeButton<L> {
    /// Creates a new button bound to the [`SafeModeClamper`].
    pub fn new<T>(cx: &mut Context, lens: L, label: impl Res<T> + Clone) -> Handle<Self>
    where
        T: ToString,
    {
        Self {
            lens,
            scrolled_lines: 0.0,
        }
        .build(cx, |cx| {
            Label::new(cx, label).hoverable(false);
        })
        .checked(lens.map(|v| v.status()))
        // We'll pretend this is a param-button, so this class is used for assigning a unique color
        .class("safe-mode")
    }
}

impl<L: Lens<Target = SafeModeClamper>> View for SafeModeButton<L> {
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
                // We can just unconditionally toggle the boolean here. When safe mode is enabled
                // this immediately clamps the affected parameters to their new range.
                let safe_mode_clamper = self.lens.get(cx);
                safe_mode_clamper.toggle(cx);

                meta.consume();
            }
            WindowEvent::MouseScroll(_scroll_x, scroll_y) => {
                self.scrolled_lines += scroll_y;

                if self.scrolled_lines.abs() >= 1.0 {
                    let safe_mode_clamper = self.lens.get(cx);

                    if self.scrolled_lines >= 1.0 {
                        safe_mode_clamper.enable(cx);
                        self.scrolled_lines -= 1.0;
                    } else {
                        safe_mode_clamper.disable();
                        self.scrolled_lines += 1.0;
                    }
                }

                meta.consume();
            }
            _ => {}
        });
    }
}
