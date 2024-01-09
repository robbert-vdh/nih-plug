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

use nih_plug::prelude::{FloatRange, Param};
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::param_base::ParamWidgetBase;
use nih_plug_vizia::widgets::util::{self, ModifiersExt};

use crate::params;

/// When shift+dragging the X-Y pad, one pixel dragged corresponds to this much change in the
/// normalized parameter.
const GRANULAR_DRAG_MULTIPLIER: f32 = 0.1;

/// An X-Y pad that controlers two parameters at the same time by binding them to one of the two
/// axes. This specific implementation has a tooltip for the X-axis parameter and allows
/// Alt+clicking to enter a specific value.
///
/// The x-parameter's range is restricted when safe mode is enabled. See `RestrictedParamSlider` for
/// more details.
#[derive(Lens)]
pub struct XyPad {
    x_param_base: ParamWidgetBase,
    y_param_base: ParamWidgetBase,

    /// The same range as that used by the filter frequency parameter. This is used to snap to
    /// frequencies when holding Alt while dragging.
    /// NOTE: This is hardcoded to work with the filter frequency parameter.
    frequency_range: FloatRange,
    /// Renormalizes the x-parameter's normalized value to a `[0, 1]` value that is used to display
    /// the parameter. This range may end up zooming in on a part of the parameter's original range
    /// when safe mode is enabled.
    x_renormalize_display: Box<dyn Fn(f32) -> f32>,
    /// The inverse of `renormalize_display`. This is used to map a normalized `[0, 1]` screen
    /// coordinate back to a `[0, 1]` normalized parameter value. These values may be different when
    /// safe mode is enabled.
    x_renormalize_event: Box<dyn Fn(f32) -> f32>,

    /// Will be set to `true` when the X-Y pad gets Alt+Click'ed. This will replace the handle with
    /// a text input box.
    text_input_active: bool,
    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,
    /// This keeps track of whether the user has pressed shift and a granular drag is active. This
    /// works exactly the same as in `ParamSlider`.
    granular_drag_status: Option<GranularDragStatus>,
    /// The number of (fractional) scrolled lines that have not yet been turned into parameter
    /// change events. This is needed to support trackpads with smooth scrolling. This probably
    /// doesn't make much sense for devices without horizontal scrolling, but we'll support it
    /// anyways because why not.
    scrolled_lines: (f32, f32),

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

enum XyPadEvent {
    /// The tooltip's size has changed. This causes us to recompute the tooltip position.
    TooltipWidthChanged,
    /// Text input has been cancelled without submitting a new value.
    CancelTextInput,
    /// A new value has been sent by the text input dialog after pressing Enter.
    TextInput(String),
}

impl XyPad {
    /// Creates a new [`XyPad`] for the given parameter. See
    /// [`ParamSlider`][nih_plug_vizia::widgets::ParamSlider] for more information on this
    /// function's arguments.
    ///
    /// The x-parameter's range is restricted when safe mode is enabled. See `RestrictedParamSlider`
    /// for more details.
    pub fn new<L, Params, P1, P2, FMap1, FMap2>(
        cx: &mut Context,
        params: L,
        params_to_x_param: FMap1,
        params_to_y_param: FMap2,
        x_renormalize_display: impl Fn(f32) -> f32 + Clone + 'static,
        x_renormalize_event: impl Fn(f32) -> f32 + 'static,
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
            x_param_base: ParamWidgetBase::new(cx, params, params_to_x_param),
            y_param_base: ParamWidgetBase::new(cx, params, params_to_y_param),

            frequency_range: params::filter_frequency_range(),
            x_renormalize_display: Box::new(x_renormalize_display.clone()),
            x_renormalize_event: Box::new(x_renormalize_event),

            text_input_active: false,
            drag_active: false,
            granular_drag_status: None,
            scrolled_lines: (0.0, 0.0),

            tooltip_pos_x: Pixels(0.0),
            tooltip_pos_y: Pixels(0.0),
        }
        .build(
            cx,
            // We need to create lenses for both the x-parameter's values and the y-parameter's
            // values
            ParamWidgetBase::build_view(params, params_to_x_param, move |cx, x_param_data| {
                ParamWidgetBase::view(cx, params, params_to_y_param, move |cx, y_param_data| {
                    // The x-parameter's range is clamped when safe mode is enabled
                    let x_position_lens = {
                        let x_renormalize_display = x_renormalize_display.clone();
                        x_param_data.make_lens(move |param| {
                            Percentage(
                                x_renormalize_display(param.unmodulated_normalized_value()) * 100.0,
                            )
                        })
                    };
                    let y_position_lens = y_param_data.make_lens(|param| {
                        // NOTE: The y-axis increments downards, and we want high values at
                        //       the top and low values at the bottom
                        Percentage((1.0 - param.unmodulated_normalized_value()) * 100.0)
                    });

                    // Another handle is drawn below the regular handle to show the
                    // modualted value
                    let modulated_x_position_lens = x_param_data.make_lens(move |param| {
                        Percentage(
                            x_renormalize_display(param.modulated_normalized_value()) * 100.0,
                        )
                    });
                    let modulated_y_position_lens = y_param_data.make_lens(|param| {
                        Percentage((1.0 - param.modulated_normalized_value()) * 100.0)
                    });

                    // Can't use `.to_string()` here as that would include the modulation.
                    let x_display_value_lens = x_param_data.make_lens(|param| {
                        param.normalized_value_to_string(param.unmodulated_normalized_value(), true)
                    });
                    let y_display_value_lens = y_param_data.make_lens(|param| {
                        param.normalized_value_to_string(param.unmodulated_normalized_value(), true)
                    });

                    // When the X-Y pad gets Alt+clicked, we'll replace it with a text input
                    // box for the frequency parameter
                    Binding::new(
                        cx,
                        XyPad::text_input_active,
                        move |cx, text_input_active| {
                            if text_input_active.get(cx) {
                                Self::text_input_view(cx, x_display_value_lens);
                            } else {
                                Self::xy_pad_modulation_handle_view(
                                    cx,
                                    modulated_x_position_lens,
                                    modulated_y_position_lens,
                                );
                                Self::xy_pad_handle_view(
                                    cx,
                                    x_position_lens,
                                    y_position_lens,
                                    x_display_value_lens,
                                    y_display_value_lens,
                                );
                            }
                        },
                    );
                });
            }),
        )
    }

    /// Create a text input that's shown in place of the X-Y pad's handle.
    fn text_input_view(cx: &mut Context, x_display_value_lens: impl Lens<Target = String>) {
        Textbox::new(cx, x_display_value_lens)
            .class("xy-pad__value-entry")
            .on_submit(|cx, string, success| {
                if success {
                    cx.emit(XyPadEvent::TextInput(string))
                } else {
                    cx.emit(XyPadEvent::CancelTextInput);
                }
            })
            .on_build(|cx| {
                cx.emit(TextEvent::StartEdit);
                cx.emit(TextEvent::SelectAll);
            })
            .class("align_center")
            .space(Stretch(1.0));
    }

    /// Draws the X-Y pad's handle and the tooltip.
    fn xy_pad_handle_view(
        cx: &mut Context,
        x_position_lens: impl Lens<Target = Units>,
        y_position_lens: impl Lens<Target = Units>,
        x_display_value_lens: impl Lens<Target = String>,
        y_display_value_lens: impl Lens<Target = String>,
    ) {
        XyPadHandle::new(cx)
            .position_type(PositionType::SelfDirected)
            .top(y_position_lens)
            .left(x_position_lens)
            .hoverable(false);

        // The stylesheet makes the tooltip visible when hovering over the X-Y
        // pad. Its position is set to the mouse coordinate in the event
        // handler. If there's enough space, the tooltip is drawn at the top
        // right of the mouse cursor.
        VStack::new(cx, |cx| {
            // The X-parameter is the 'important' one, so we'll display that at
            // the bottom since it's closer to the mouse cursor. We'll also
            // hardcode the `Q: ` prefix for now to make it a bit clearer and to
            // reduce the length difference between the lines a bit.
            Label::new(cx, y_display_value_lens.map(|value| format!("Q: {value}")));
            Label::new(cx, x_display_value_lens);
        })
        .class("xy-pad__tooltip")
        .left(XyPad::tooltip_pos_x)
        .top(XyPad::tooltip_pos_y)
        .position_type(PositionType::SelfDirected)
        .on_geo_changed(|cx, change_flags| {
            // When a new parameter value causes the width of the tooltip to
            // change, we must recompute its position so it stays anchored to
            // the mouse cursor
            if change_flags.intersects(GeoChanged::WIDTH_CHANGED) {
                cx.emit(XyPadEvent::TooltipWidthChanged);
            }
        })
        .hoverable(false);
    }

    /// The secondary handle that shows the modulated value if the plugin is being monophonically
    /// modualted.
    fn xy_pad_modulation_handle_view(
        cx: &mut Context,
        modulated_x_position_lens: impl Lens<Target = Units>,
        modulated_y_position_lens: impl Lens<Target = Units>,
    ) {
        XyPadHandle::new(cx)
            .class("xy-pad__handle--modulated")
            .position_type(PositionType::SelfDirected)
            .top(modulated_y_position_lens)
            .left(modulated_x_position_lens)
            .hoverable(false);
    }

    /// Should be called at the start of a drag operation.
    fn begin_set_parameters(&self, cx: &mut EventContext) {
        // NOTE: Since the X-parameter is the main parameter, we'll always modify this parameter
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
        snap_to_whole_notes: bool,
    ) {
        // When snapping to whole notes, we'll transform the normalized value back to unnormalized
        // (this is hardcoded for the filter frequency parameter). These coordinate mappings also
        // need to respect the restricted ranges from the safe mode button.
        let mut x_value =
            (self.x_renormalize_event)(util::remap_current_entity_x_coordinate(cx, x_pos));
        if snap_to_whole_notes {
            let x_freq = self.frequency_range.unnormalize(x_value);

            let fractional_note = nih_plug::util::freq_to_midi_note(x_freq);
            let note = fractional_note.round();
            let note_freq = nih_plug::util::f32_midi_note_to_freq(note);

            x_value = self.frequency_range.normalize(note_freq);
        }

        // We want the top of the widget to be 1.0 and the bottom to be 0.0, this is the opposite of
        // how the y-coordinate works
        let y_value = 1.0 - util::remap_current_entity_y_coordinate(cx, y_pos);

        self.set_normalized_values(cx, (x_value, y_value));
    }

    /// Should be called at the end of a drag operation.
    fn end_set_parameters(&self, cx: &mut EventContext) {
        self.x_param_base.end_set_parameter(cx);
        self.y_param_base.end_set_parameter(cx);
    }

    /// Used to position the tooltip to the top right of the mouse cursor. If there's not enough
    /// space there, the tooltip will be pushed to the left or the right of the cursor.
    fn update_tooltip_pos(&mut self, cx: &mut EventContext) {
        let bounds = cx.cache.get_bounds(cx.current());
        let relative_x = cx.mouse().cursorx - bounds.x;
        let relative_y = cx.mouse().cursory - bounds.y;

        // These positions need to take DPI scaling into account
        let dpi_scale = cx.scale_factor();
        let padding = 2.0 * dpi_scale;

        // If there's not enough space at the top right, we'll move the tooltip to the
        // bottom and/or the left
        // NOTE: This is hardcoded to find the tooltip. The Binding also counts as a child.
        let binding_entity = cx.last_child().expect("Missing child view in X-Y pad");
        let tooltip_entity = cx
            .with_current(binding_entity, |cx| cx.last_child())
            .expect("Missing child view in X-Y pad binding");
        let tooltip_bounds = cx.cache.get_bounds(tooltip_entity);
        // NOTE: The width can vary drastically depending on the frequency value, so we'll
        //       hardcode a minimum width in this comparison to avoid this from jumping
        //       around. The parameter value updates are a bit delayed when dragging the
        //       parameter, so we may be using an old width here.
        let tooltip_pos_x =
            if (relative_x + tooltip_bounds.w.max(150.0) + (padding * 2.0)) >= bounds.w {
                relative_x - padding - tooltip_bounds.w
            } else {
                relative_x + padding
            };
        let tooltip_pos_y = if (relative_y - tooltip_bounds.h - (padding * 2.0)) <= 0.0 {
            relative_y + padding
        } else {
            relative_y - padding - tooltip_bounds.h
        };

        self.tooltip_pos_x = Pixels(tooltip_pos_x / dpi_scale);
        self.tooltip_pos_y = Pixels(tooltip_pos_y / dpi_scale);
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
        event.map(|window_event, meta| {
            match window_event {
                XyPadEvent::TooltipWidthChanged => {
                    // The tooltip tracks the mouse position, but it also needs to be recomputed when
                    // the parameter changes while the tooltip is still visible. Without this the
                    // position maya be off when the parameter is automated, or because of the samll
                    // delay between interacting with a parameter and the parameter changing.
                    if cx.hovered() == cx.current() {
                        self.update_tooltip_pos(cx);
                    }
                }
                XyPadEvent::CancelTextInput => {
                    self.text_input_active = false;
                    cx.set_active(false);

                    meta.consume();
                }
                XyPadEvent::TextInput(string) => {
                    // This controls the X-parameter directly
                    if let Some(normalized_value) =
                        self.x_param_base.string_to_normalized_value(string)
                    {
                        self.x_param_base.begin_set_parameter(cx);
                        self.x_param_base.set_normalized_value(cx, normalized_value);
                        self.x_param_base.end_set_parameter(cx);
                    }

                    self.text_input_active = false;

                    meta.consume();
                }
            }

            meta.consume();
        });

        event.map(|window_event, meta| match window_event {
            WindowEvent::MouseDown(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                if cx.modifiers().alt() {
                    // ALt+Click brings up a text entry dialog
                    self.text_input_active = true;
                    cx.set_active(true);
                } else if cx.modifiers().command() {
                    // Ctrl+Click, double click, and right clicks should reset the parameter instead
                    // of initiating a drag operation
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
                    if cx.modifiers().shift() {
                        self.granular_drag_status = Some(GranularDragStatus {
                            starting_x_coordinate: cx.mouse().cursorx,
                            x_starting_value: self.x_param_base.unmodulated_normalized_value(),
                            starting_y_coordinate: cx.mouse().cursory,
                            y_starting_value: self.y_param_base.unmodulated_normalized_value(),
                        });
                    } else {
                        self.granular_drag_status = None;
                        self.set_normalized_values_for_mouse_pos(
                            cx,
                            (cx.mouse().cursorx, cx.mouse().cursory),
                            false,
                        );
                    }
                }

                meta.consume();
            }
            WindowEvent::MouseDoubleClick(MouseButton::Left)
            | WindowEvent::MouseDown(MouseButton::Right)
            | WindowEvent::MouseDoubleClick(MouseButton::Right)
            | WindowEvent::MouseTripleClick(MouseButton::Right) => {
                // Ctrl+Click, double click, and right clicks should reset the parameter instead of
                // initiating a drag operation
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
                // The tooltip should track the mouse position. This is also recomputed whenever
                // parameter values change (and thus the tooltip's width changes) so it stays in the
                // correct position.
                self.update_tooltip_pos(cx);

                if self.drag_active {
                    let dpi_scale = cx.scale_factor();

                    if cx.modifiers().shift() {
                        // If shift is being held then the drag should be more granular instead of
                        // absolute
                        // TODO: Mouse warping is really needed here, but it's not exposed right now
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

                        // These positions should be compensated for the DPI scale so it remains
                        // consistent. When the range is restricted the `delta_x` should also change
                        // accordingly.
                        let start_x = util::remap_current_entity_x_t(
                            cx,
                            (self.x_renormalize_display)(granular_drag_status.x_starting_value),
                        );
                        let delta_x = (*x - granular_drag_status.starting_x_coordinate)
                            * GRANULAR_DRAG_MULTIPLIER
                            * dpi_scale;

                        let start_y = util::remap_current_entity_y_t(
                            cx,
                            // NOTE: Just like above, the corodinates go from top to bottom
                            //       while we want the X-Y pad to go from bottom to top
                            1.0 - granular_drag_status.y_starting_value,
                        );
                        let delta_y = ((*y - granular_drag_status.starting_y_coordinate)
                            * GRANULAR_DRAG_MULTIPLIER)
                            * dpi_scale;

                        // This also takes the Alt+drag note snapping into account
                        self.set_normalized_values_for_mouse_pos(
                            cx,
                            (start_x + delta_x, start_y + delta_y),
                            cx.modifiers().alt(),
                        );
                    } else {
                        // When alt is pressed _while_ dragging, the frequency parameter on the
                        // X-axis snaps to whole notes
                        self.granular_drag_status = None;
                        self.set_normalized_values_for_mouse_pos(
                            cx,
                            (*x, *y),
                            cx.modifiers().alt(),
                        );
                    }
                }
            }
            WindowEvent::KeyUp(_, Some(Key::Shift)) => {
                // If this happens while dragging, snap back to reality uh I mean the current screen
                // position
                if self.drag_active && self.granular_drag_status.is_some() {
                    // When alt is pressed _while_ dragging, the frequency parameter on the X-axis
                    // snaps to whole notes
                    self.granular_drag_status = None;
                    self.set_normalized_values_for_mouse_pos(
                        cx,
                        (cx.mouse().cursorx, cx.mouse().cursory),
                        cx.modifiers().alt(),
                    );
                }
            }
            WindowEvent::MouseScroll(scroll_x, scroll_y) => {
                // With a regular scroll wheel `scroll_*` will only ever be -1 or 1, but with smooth
                // scrolling trackpads being a these `scroll_*` can be anything.
                let (remaining_scroll_x, remaining_scroll_y) = &mut self.scrolled_lines;
                *remaining_scroll_x += scroll_x;
                *remaining_scroll_y += scroll_y;

                // This is a pretty crude way to avoid scrolling outside of the safe mode range
                let clamp_x_value =
                    |value| (self.x_renormalize_event)((self.x_renormalize_display)(value));

                if remaining_scroll_x.abs() >= 1.0 {
                    let use_finer_steps = cx.modifiers().shift();

                    // Scrolling while dragging needs to be taken into account here
                    if !self.drag_active {
                        self.x_param_base.begin_set_parameter(cx);
                    }

                    let mut current_value = self.x_param_base.unmodulated_normalized_value();

                    while *remaining_scroll_x >= 1.0 {
                        current_value = self
                            .x_param_base
                            .next_normalized_step(current_value, use_finer_steps);
                        self.x_param_base
                            .set_normalized_value(cx, clamp_x_value(current_value));
                        *remaining_scroll_x -= 1.0;
                    }

                    while *remaining_scroll_x <= -1.0 {
                        current_value = self
                            .x_param_base
                            .previous_normalized_step(current_value, use_finer_steps);
                        self.x_param_base
                            .set_normalized_value(cx, clamp_x_value(current_value));
                        *remaining_scroll_x += 1.0;
                    }

                    if !self.drag_active {
                        self.x_param_base.end_set_parameter(cx);
                    }
                }

                if remaining_scroll_y.abs() >= 1.0 {
                    let use_finer_steps = cx.modifiers().shift();

                    // Scrolling while dragging needs to be taken into account here
                    if !self.drag_active {
                        self.y_param_base.begin_set_parameter(cx);
                    }

                    let mut current_value = self.y_param_base.unmodulated_normalized_value();

                    while *remaining_scroll_y >= 1.0 {
                        current_value = self
                            .y_param_base
                            .next_normalized_step(current_value, use_finer_steps);
                        self.y_param_base.set_normalized_value(cx, current_value);
                        *remaining_scroll_y -= 1.0;
                    }

                    while *remaining_scroll_y <= -1.0 {
                        current_value = self
                            .y_param_base
                            .previous_normalized_step(current_value, use_finer_steps);
                        self.y_param_base.set_normalized_value(cx, current_value);
                        *remaining_scroll_y += 1.0;
                    }

                    if !self.drag_active {
                        self.y_param_base.end_set_parameter(cx);
                    }
                }

                meta.consume();
            }

            _ => {}
        });
    }
}

impl View for XyPadHandle {
    fn element(&self) -> Option<&'static str> {
        Some("xy-pad__handle")
    }
}
