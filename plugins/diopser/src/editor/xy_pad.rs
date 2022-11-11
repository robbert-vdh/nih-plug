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

use nih_plug::prelude::Param;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::param_base::ParamWidgetBase;
use nih_plug_vizia::widgets::util::{self, ModifiersExt};

// TODO: Vizia doesn't let you do this -50% translation programmatically yet, so this is hardcoded
//       for now
const HANDLE_WIDTH_PX: f32 = 20.0;

/// An X-Y pad that controlers two parameters at the same time by binding them to one of the two
/// axes. This specific implementation has a tooltip for the X-axis parmaeter and allows
/// Alt+clicking to enter a specific value.
//
// TODO: Text entry for the x-parameter
// TODO: Tooltip
// TODO: Granular dragging
pub struct XyPad {
    x_param_base: ParamWidgetBase,
    y_param_base: ParamWidgetBase,

    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,
}

/// The [`XyPad`]'s handle. This is a separate eleemnt to allow easier positioning.
struct XyPadHandle;

impl XyPad {
    /// Creates a new [`XyPad`] for the given parameter. See
    /// [`ParamSlider`][nih_plug_vizia::widgets::ParamSlider] for more information on this
    /// function's arguments.
    pub fn new<L, Params, P1, P2, FMap1, FMap2>(
        cx: &mut Context,
        params: L,
        params_to_x_param: FMap1,
        params_to_y_param: FMap2,
    ) -> Handle<Self>
    where
        L: Lens<Target = Params> + Clone,
        Params: 'static,
        P1: Param + 'static,
        P2: Param + 'static,
        FMap1: Fn(&Params) -> &P1 + Copy + 'static,
        FMap2: Fn(&Params) -> &P2 + Copy + 'static,
    {
        Self {
            x_param_base: ParamWidgetBase::new(cx, params.clone(), params_to_x_param),
            y_param_base: ParamWidgetBase::new(cx, params.clone(), params_to_y_param),

            drag_active: false,
        }
        .build(
            cx,
            // We need to create lenses for both the x-parameter's values and the y-parameter's
            // values
            ParamWidgetBase::build_view(
                params.clone(),
                params_to_x_param,
                move |cx, x_param_data| {
                    ParamWidgetBase::view(
                        cx,
                        params,
                        params_to_y_param,
                        move |cx, y_param_data| {
                            let x_position_lens = x_param_data.make_lens(|param| {
                                Percentage(param.unmodulated_normalized_value() * 100.0)
                            });
                            let y_position_lens = y_param_data.make_lens(|param| {
                                // NOTE: The y-axis increments downards, and we want high values at
                                //       the top and low values at the bottom
                                Percentage((1.0 - param.unmodulated_normalized_value()) * 100.0)
                            });

                            XyPadHandle::new(cx)
                                .top(y_position_lens)
                                .left(x_position_lens)
                                // TODO: It would be much nicer if this could be set in the
                                //       stylesheet, but Vizia doesn't support that right now
                                .translate((-(HANDLE_WIDTH_PX / 2.0), -(HANDLE_WIDTH_PX / 2.0)))
                                .width(Pixels(HANDLE_WIDTH_PX))
                                .height(Pixels(HANDLE_WIDTH_PX));
                        },
                    );
                },
            ),
        )
    }

    /// Should be called at the start of a drag operation.
    fn begin_set_parameters(&self, cx: &mut EventContext) {
        self.x_param_base.begin_set_parameter(cx);
        self.y_param_base.begin_set_parameter(cx);
    }

    /// Resets both parameters. `begin_set_parameters()` needs to be called first.
    fn reset_parameters(&self, cx: &mut EventContext) {
        self.x_param_base
            .set_normalized_value(cx, self.x_param_base.default_normalized_value());
        self.y_param_base
            .set_normalized_value(cx, self.y_param_base.default_normalized_value());
    }

    /// Set a normalized value for both parameters. `begin_set_parameters()` needs to be called
    /// first.
    fn set_normalized_values(&self, cx: &mut EventContext, (x_value, y_value): (f32, f32)) {
        self.x_param_base.set_normalized_value(cx, x_value);
        self.y_param_base.set_normalized_value(cx, y_value);
    }

    /// Should be called at the end of a drag operation.
    fn end_set_parameters(&self, cx: &mut EventContext) {
        self.x_param_base.end_set_parameter(cx);
        self.y_param_base.end_set_parameter(cx);
    }
}

impl XyPadHandle {
    fn new(cx: &mut Context) -> Handle<Self> {
        // This doesn't have or need any special behavior, it's just a marker element used for
        // positioning he handle
        Self.build(cx, |_| ())
    }
}

impl View for XyPad {
    fn element(&self) -> Option<&'static str> {
        Some("xy-pad")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|window_event, meta| match window_event {
            WindowEvent::MouseDown(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                // TODO: Alt+click text entry
                if cx.modifiers.command() {
                    // Ctrl+Click and double click should reset the parameter instead of initiating
                    // a drag operation
                    self.begin_set_parameters(cx);
                    self.reset_parameters(cx);
                    self.end_set_parameters(cx);
                } else {
                    self.drag_active = true;
                    cx.capture();
                    // NOTE: Otherwise we don't get key up events
                    cx.focus();
                    cx.set_active(true);

                    // When holding down shift while clicking on a parameter we want to granuarly
                    // edit the parameter without jumping to a new value
                    self.begin_set_parameters(cx);
                    // TODO: Granular dragging
                    // if cx.modifiers.shift() {
                    //     self.granular_drag_start_x_value = Some((
                    //         cx.mouse.cursorx,
                    //         self.param_base.unmodulated_normalized_value(),
                    //     ));
                    // } else {
                    //     self.granular_drag_start_x_value = None;
                    //     self.set_normalized_value_drag(
                    //         cx,
                    //         util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
                    //     );
                    // }
                }

                meta.consume();
            }
            WindowEvent::MouseDoubleClick(MouseButton::Left) => {
                // Ctrl+Click and double click should reset the parameters instead of initiating a
                // drag operation
                self.begin_set_parameters(cx);
                self.reset_parameters(cx);
                self.end_set_parameters(cx);

                meta.consume();
            }
            WindowEvent::MouseUp(MouseButton::Left) => {
                if self.drag_active {
                    self.drag_active = false;
                    cx.release();
                    cx.set_active(false);

                    self.end_set_parameters(cx);

                    meta.consume();
                }
            }
            WindowEvent::MouseMove(x, y) => {
                if self.drag_active {
                    // If shift is being held then the drag should be more granular instead of
                    // absolute
                    // TODO: Granular dragging
                    // if cx.modifiers.shift() {
                    //     let (drag_start_x, drag_start_value) =
                    //         *self.granular_drag_start_x_value.get_or_insert_with(|| {
                    //             (
                    //                 cx.mouse.cursorx,
                    //                 self.param_base.unmodulated_normalized_value(),
                    //             )
                    //         });

                    //     self.set_normalized_value_drag(
                    //         cx,
                    //         util::remap_current_entity_x_coordinate(
                    //             cx,
                    //             // This can be optimized a bit
                    //             util::remap_current_entity_x_t(cx, drag_start_value)
                    //                 + (*x - drag_start_x) * GRANULAR_DRAG_MULTIPLIER,
                    //         ),
                    //     );
                    // } else {
                    //     self.granular_drag_start_x_value = None;

                    //     self.set_normalized_value_drag(
                    //         cx,
                    //         util::remap_current_entity_x_coordinate(cx, *x),
                    //     );
                    // }

                    self.set_normalized_values(
                        cx,
                        (
                            util::remap_current_entity_x_coordinate(cx, *x),
                            // We want the top of the widget to be 1.0 and the bottom to be 0.0,
                            // this is the opposite of how the y-coordinate works
                            1.0 - util::remap_current_entity_y_coordinate(cx, *y),
                        ),
                    );
                }
            }
            // TODO: Granular dragging
            // WindowEvent::KeyUp(_, Some(Key::Shift)) => {
            //     // If this happens while dragging, snap back to reality uh I mean the current screen
            //     // position
            //     if self.drag_active && self.granular_drag_start_x_value.is_some() {
            //         self.granular_drag_start_x_value = None;
            //         self.param_base.set_normalized_value(
            //             cx,
            //             util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
            //         );
            //     }
            // }
            // TODO: Scrolling, because why not. Could be useful on laptops/with touchpads.
            _ => {}
        });
    }
}

impl View for XyPadHandle {
    fn element(&self) -> Option<&'static str> {
        Some("xy-pad__handle")
    }
}
