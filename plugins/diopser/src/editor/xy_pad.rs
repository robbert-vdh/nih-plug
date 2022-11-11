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

/// When shift+dragging the X-Y pad, one pixel dragged corresponds to this much change in the
/// normalized parameter.
const GRANULAR_DRAG_MULTIPLIER: f32 = 0.1;

// TODO: Vizia doesn't let you do this -50% translation programmatically yet, so this is hardcoded
//       for now
const HANDLE_WIDTH_PX: f32 = 20.0;

/// An X-Y pad that controlers two parameters at the same time by binding them to one of the two
/// axes. This specific implementation has a tooltip for the X-axis parmaeter and allows
/// Alt+clicking to enter a specific value.
//
// TODO: Text entry for the x-parameter
// TODO: Tooltip
#[derive(Lens)]
pub struct XyPad {
    x_param_base: ParamWidgetBase,
    y_param_base: ParamWidgetBase,

    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,
    /// This keeps track of whether the user has pressed shift and a granular drag is active. This
    /// works exactly the same as in `ParamSlider`.
    granular_drag_status: Option<GranularDragStatus>,

    // Used to position the tooltip so it's anchored to the mouse cursor, set in the `MouseMove`
    // event handler so the tooltip stays at the top right of the mouse cursor.
    tooltip_pos_x: Units,
    tooltip_pos_y: Units,
}

/// The [`XyPad`]'s handle. This is a separate eleemnt to allow easier positioning.
struct XyPadHandle;

// TODO: Vizia's derive macro requires this to be pub
#[derive(Debug, Clone, Copy)]
pub struct GranularDragStatus {
    /// The mouse's X-coordinate when the granular drag was started.
    pub starting_x_coordinate: f32,
    /// The normalized value when the granular drag was started for the X-parameter.
    pub x_starting_value: f32,
    /// The mouse's Y-coordinate when the granular drag was started.
    pub starting_y_coordinate: f32,
    /// The normalized value when the granular drag was started for the Y-parameter.
    pub y_starting_value: f32,
}

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
            granular_drag_status: None,

            tooltip_pos_x: Pixels(0.0),
            tooltip_pos_y: Pixels(0.0),
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

                            // Can't use `.to_string()` here as that would include the modulation.
                            let x_display_value_lens = x_param_data.make_lens(|param| {
                                param.normalized_value_to_string(
                                    param.unmodulated_normalized_value(),
                                    true,
                                )
                            });
                            let y_display_value_lens = y_param_data.make_lens(|param| {
                                param.normalized_value_to_string(
                                    param.unmodulated_normalized_value(),
                                    true,
                                )
                            });

                            XyPadHandle::new(cx)
                                .position_type(PositionType::SelfDirected)
                                .top(y_position_lens)
                                .left(x_position_lens)
                                // TODO: It would be much nicer if this could be set in the
                                //       stylesheet, but Vizia doesn't support that right now
                                .translate((-(HANDLE_WIDTH_PX / 2.0), -(HANDLE_WIDTH_PX / 2.0)))
                                .width(Pixels(HANDLE_WIDTH_PX))
                                .height(Pixels(HANDLE_WIDTH_PX))
                                .hoverable(false);

                            // The stylesheet makes the tooltip visible when hovering over the X-Y
                            // pad. Its position is set to the mouse coordinate in the event
                            // handler. If there's enough space, the tooltip is drawn at the top
                            // right of the mouse cursor.
                            VStack::new(cx, move |cx| {
                                // The X-parameter is the 'important' one, so we'll display that at
                                // the bottom since it's closer to the mouse cursor
                                Label::new(cx, y_display_value_lens);
                                Label::new(cx, x_display_value_lens);
                            })
                            .class("xy-pad__tooltip")
                            .left(XyPad::tooltip_pos_x)
                            .top(XyPad::tooltip_pos_y)
                            .position_type(PositionType::SelfDirected)
                            .hoverable(false);
                        },
                    );
                },
            ),
        )
    }

    /// Should be called at the start of a drag operation.
    fn begin_set_parameters(&self, cx: &mut EventContext) {
        // NOTE: Since the X-parameter is the main parmaeter, we'll always modify this parameter
        //       last so the host will keep this parameter highlighted
        self.y_param_base.begin_set_parameter(cx);
        self.x_param_base.begin_set_parameter(cx);
    }

    /// Resets both parameters. `begin_set_parameters()` needs to be called first.
    fn reset_parameters(&self, cx: &mut EventContext) {
        self.y_param_base
            .set_normalized_value(cx, self.y_param_base.default_normalized_value());
        self.x_param_base
            .set_normalized_value(cx, self.x_param_base.default_normalized_value());
    }

    /// Set a normalized value for both parameters. `begin_set_parameters()` needs to be called
    /// first.
    fn set_normalized_values(&self, cx: &mut EventContext, (x_value, y_value): (f32, f32)) {
        self.y_param_base.set_normalized_value(cx, y_value);
        self.x_param_base.set_normalized_value(cx, x_value);
    }

    /// Set a normalized value for both parameters based on mouse coordinates.
    /// `begin_set_parameters()` needs to be called first.
    fn set_normalized_values_for_mouse_pos(
        &self,
        cx: &mut EventContext,
        (x_pos, y_pos): (f32, f32),
    ) {
        self.set_normalized_values(
            cx,
            (
                util::remap_current_entity_x_coordinate(cx, x_pos),
                // We want the top of the widget to be 1.0 and the bottom to be 0.0,
                // this is the opposite of how the y-coordinate works
                1.0 - util::remap_current_entity_y_coordinate(cx, y_pos),
            ),
        );
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

                    // When holding down shift while clicking on the X-Y pad we want to granuarly
                    // edit the parameter without jumping to a new value
                    self.begin_set_parameters(cx);
                    if cx.modifiers.shift() {
                        self.granular_drag_status = Some(GranularDragStatus {
                            starting_x_coordinate: cx.mouse.cursorx,
                            x_starting_value: self.x_param_base.unmodulated_normalized_value(),
                            starting_y_coordinate: cx.mouse.cursory,
                            y_starting_value: self.y_param_base.unmodulated_normalized_value(),
                        });
                    } else {
                        self.granular_drag_status = None;
                        self.set_normalized_values_for_mouse_pos(
                            cx,
                            (cx.mouse.cursorx, cx.mouse.cursory),
                        );
                    }
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
                // This is used to position the tooltip to the top right of the mouse cursor
                let bounds = cx.cache.get_bounds(cx.current());
                let relative_x = x - bounds.x;
                let relative_y = y - bounds.y;

                // If there's not enough space at the top right, we'll move the tooltip to the
                // bottom and/or the left
                let tooltip_entity = cx
                    .tree
                    .get_last_child(cx.current())
                    .expect("Missing child view in X-Y pad");
                let tooltip_bounds = cx.cache.get_bounds(tooltip_entity);
                self.tooltip_pos_x = if (relative_x + tooltip_bounds.w + 4.0) >= bounds.w {
                    Pixels(relative_x - 2.0 - tooltip_bounds.w)
                } else {
                    Pixels(relative_x + 2.0)
                };
                self.tooltip_pos_y = if (relative_y - tooltip_bounds.h - 4.0) <= 0.0 {
                    Pixels(relative_y + 2.0)
                } else {
                    Pixels(relative_y - 2.0 - tooltip_bounds.h)
                };

                if self.drag_active {
                    // If shift is being held then the drag should be more granular instead of
                    // absolute
                    // TODO: Mouse warping is really needed here, but it's not exposed right now
                    if cx.modifiers.shift() {
                        let granular_drag_status =
                            *self
                                .granular_drag_status
                                .get_or_insert_with(|| GranularDragStatus {
                                    starting_x_coordinate: *x,
                                    x_starting_value: self
                                        .x_param_base
                                        .unmodulated_normalized_value(),
                                    starting_y_coordinate: *y,
                                    y_starting_value: self
                                        .y_param_base
                                        .unmodulated_normalized_value(),
                                });

                        self.set_normalized_values_for_mouse_pos(
                            cx,
                            (
                                // This can be optimized a bit
                                util::remap_current_entity_x_t(
                                    cx,
                                    granular_drag_status.x_starting_value,
                                ) + ((*x - granular_drag_status.starting_x_coordinate)
                                    * GRANULAR_DRAG_MULTIPLIER),
                                (util::remap_current_entity_y_t(
                                    cx,
                                    // NOTE: Just like above, the corodinates go from top to bottom
                                    //       while we want the X-Y pad to go from bottom to top
                                    1.0 - granular_drag_status.y_starting_value,
                                ) + ((*y - granular_drag_status.starting_y_coordinate)
                                    * GRANULAR_DRAG_MULTIPLIER)),
                            ),
                        );
                    } else {
                        self.granular_drag_status = None;
                        self.set_normalized_values_for_mouse_pos(cx, (*x, *y));
                    }
                }
            }
            WindowEvent::KeyUp(_, Some(Key::Shift)) => {
                // If this happens while dragging, snap back to reality uh I mean the current screen
                // position
                if self.drag_active && self.granular_drag_status.is_some() {
                    self.granular_drag_status = None;
                    self.set_normalized_values_for_mouse_pos(
                        cx,
                        (cx.mouse.cursorx, cx.mouse.cursory),
                    );
                }
            }
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
