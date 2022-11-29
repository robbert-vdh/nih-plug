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

//! A modified version of the regular `ParamSlider` that works with Diopser's safe mode.

use nih_plug::prelude::Param;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::param_base::ParamWidgetBase;
use nih_plug_vizia::widgets::util::{self, ModifiersExt};

use super::SafeModeClamper;

/// When shift+dragging a parameter, one pixel dragged corresponds to this much change in the
/// normalized parameter.
const GRANULAR_DRAG_MULTIPLIER: f32 = 0.1;

/// A simplified version of `ParamSlider` that works with Diopser's safe mode. The slider's range is
/// restricted when safe mode is enabled.
#[derive(Lens)]
pub struct RestrictedParamSlider<LSafeMode: Lens<Target = SafeModeClamper>> {
    safe_mode: LSafeMode,
    param_base: ParamWidgetBase,

    /// Will be set to `true` when the field gets Alt+Click'ed which will replace the label with a
    /// text box.
    text_input_active: bool,
    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,
    /// We keep track of the start coordinate and normalized value when holding down Shift while
    /// dragging for higher precision dragging. This is a `None` value when granular dragging is not
    /// active.
    granular_drag_status: Option<GranularDragStatus>,

    // These fields are set through modifiers:
    /// Whether or not to listen to scroll events for changing the parameter's value in steps.
    use_scroll_wheel: bool,
    /// The number of (fractional) scrolled lines that have not yet been turned into parameter
    /// change events. This is needed to support trackpads with smooth scrolling.
    scrolled_lines: f32,
}

enum ParamSliderEvent {
    /// Text input has been cancelled without submitting a new value.
    CancelTextInput,
    /// A new value has been sent by the text input dialog after pressing Enter.
    TextInput(String),
}

#[derive(Debug, Clone, Copy)]
pub struct GranularDragStatus {
    /// The mouse's X-coordinate when the granular drag was started.
    pub starting_x_coordinate: f32,
    /// The normalized value when the granular drag was started.
    pub starting_value: f32,
}

impl<LSafeMode: Lens<Target = SafeModeClamper>> RestrictedParamSlider<LSafeMode> {
    /// See the original `ParamSlider`.
    pub fn new<LParams, Params, P, FMap>(
        cx: &mut Context,
        params: LParams,
        params_to_param: FMap,
        safe_mode: LSafeMode,
    ) -> Handle<Self>
    where
        LParams: Lens<Target = Params> + Clone,
        Params: 'static,
        P: Param + 'static,
        FMap: Fn(&Params) -> &P + Copy + 'static,
    {
        // We'll visualize the difference between the current value and the default value if the
        // default value lies somewhere in the middle and the parameter is continuous. Otherwise
        // this approach looks a bit jarring.
        Self {
            safe_mode,
            param_base: ParamWidgetBase::new(cx, params.clone(), params_to_param),

            text_input_active: false,
            drag_active: false,
            granular_drag_status: None,

            use_scroll_wheel: true,
            scrolled_lines: 0.0,
        }
        .build(
            cx,
            ParamWidgetBase::build_view(params, params_to_param, move |cx, param_data| {
                // Can't use `.to_string()` here as that would include the modulation.
                let unmodulated_normalized_value_lens =
                    param_data.make_lens(|param| param.unmodulated_normalized_value());
                let display_value_lens = param_data.make_lens(|param| {
                    param.normalized_value_to_string(param.unmodulated_normalized_value(), true)
                });

                // The resulting tuple `(start_t, delta)` corresponds to the start and the
                // signed width of the bar. `start_t` is in `[0, 1]`, and `delta` is in
                // `[-1, 1]`.
                let fill_start_delta_lens = {
                    let param_data = param_data.clone();
                    unmodulated_normalized_value_lens.map(move |current_value| {
                        Self::compute_fill_start_delta(param_data.param(), *current_value)
                    })
                };

                // If the parameter is being modulated by the host (this only works for CLAP
                // plugins with hosts that support this), then this is the difference
                // between the 'true' value and the current value after modulation has been
                // applied. This follows the same format as `fill_start_delta_lens`.
                let modulation_start_delta_lens = param_data
                    .make_lens(move |param| Self::compute_modulation_fill_start_delta(param));

                // Only draw the text input widget when it gets focussed. Otherwise, overlay the
                // label with the slider. Creating the textbox based on
                // `ParamSliderInternal::text_input_active` lets us focus the textbox when it gets
                // created.
                Binding::new(
                    cx,
                    RestrictedParamSlider::<LSafeMode>::text_input_active,
                    move |cx, text_input_active| {
                        if text_input_active.get(cx) {
                            Self::text_input_view(cx, display_value_lens.clone());
                        } else {
                            // All of this data needs to be moved into the `ZStack` closure, and
                            // the `Map` lens combinator isn't `Copy`
                            let fill_start_delta_lens = fill_start_delta_lens.clone();
                            let modulation_start_delta_lens = modulation_start_delta_lens.clone();
                            let display_value_lens = display_value_lens.clone();

                            ZStack::new(cx, move |cx| {
                                Self::slider_fill_view(
                                    cx,
                                    fill_start_delta_lens,
                                    modulation_start_delta_lens,
                                );
                                Self::slider_label_view(cx, display_value_lens);
                            })
                            .hoverable(false);
                        }
                    },
                );
            }),
        )
    }

    /// Create a text input that's shown in place of the slider.
    fn text_input_view(cx: &mut Context, display_value_lens: impl Lens<Target = String>) {
        Textbox::new(cx, display_value_lens)
            .class("value-entry")
            .on_submit(|cx, string, success| {
                if success {
                    cx.emit(ParamSliderEvent::TextInput(string))
                } else {
                    cx.emit(ParamSliderEvent::CancelTextInput);
                }
            })
            .on_build(|cx| {
                cx.emit(TextEvent::StartEdit);
                cx.emit(TextEvent::SelectAll);
            })
            // `.child_space(Stretch(1.0))` no longer works
            .class("align_center")
            .child_top(Stretch(1.0))
            .child_bottom(Stretch(1.0))
            .height(Stretch(1.0))
            .width(Stretch(1.0));
    }

    /// Create the fill part of the slider.
    fn slider_fill_view(
        cx: &mut Context,
        fill_start_delta_lens: impl Lens<Target = (f32, f32)>,
        modulation_start_delta_lens: impl Lens<Target = (f32, f32)>,
    ) {
        // The filled bar portion. This can be visualized in a couple different ways depending on
        // the current style property. See [`ParamSliderStyle`].
        Element::new(cx)
            .class("fill")
            .height(Stretch(1.0))
            .left(
                fill_start_delta_lens
                    .clone()
                    .map(|(start_t, _)| Percentage(start_t * 100.0)),
            )
            .width(fill_start_delta_lens.map(|(_, delta)| Percentage(delta * 100.0)))
            // Hovering is handled on the param slider as a whole, this
            // should not affect that
            .hoverable(false);

        // If the parameter is being modulated, then we'll display another
        // filled bar showing the current modulation delta
        // VIZIA's bindings make this a bit, uh, difficult to read
        Element::new(cx)
            .class("fill")
            .class("fill--modulation")
            .height(Stretch(1.0))
            .visibility(
                modulation_start_delta_lens
                    .clone()
                    .map(|(_, delta)| *delta != 0.0),
            )
            // Widths cannot be negative, so we need to compensate the start
            // position if the width does happen to be negative
            .width(
                modulation_start_delta_lens
                    .clone()
                    .map(|(_, delta)| Percentage(delta.abs() * 100.0)),
            )
            .left(modulation_start_delta_lens.map(|(start_t, delta)| {
                if *delta < 0.0 {
                    Percentage((start_t + delta) * 100.0)
                } else {
                    Percentage(start_t * 100.0)
                }
            }))
            .hoverable(false);
    }

    /// Create the text part of the slider. Shown on top of the fill using a `ZStack`.
    fn slider_label_view(cx: &mut Context, display_value_lens: impl Lens<Target = String>) {
        Label::new(cx, display_value_lens)
            .class("value")
            .class("value--single")
            .child_space(Stretch(1.0))
            .height(Stretch(1.0))
            .width(Stretch(1.0))
            .hoverable(false);
    }

    /// Calculate the start position and width of the slider's fill region based on the selected
    /// style, the parameter's current value, and the parameter's step sizes. The resulting tuple
    /// `(start_t, delta)` corresponds to the start and the signed width of the bar. `start_t` is in
    /// `[0, 1]`, and `delta` is in `[-1, 1]`.
    fn compute_fill_start_delta<P: Param>(param: &P, current_value: f32) -> (f32, f32) {
        let default_value = param.default_normalized_value();
        let step_count = param.step_count();
        let draw_fill_from_default = step_count.is_none() && (0.45..=0.55).contains(&default_value);

        if draw_fill_from_default {
            let delta = (default_value - current_value).abs();

            // Don't draw the filled portion at all if it could have been a
            // rounding error since those slivers just look weird
            (
                default_value.min(current_value),
                if delta >= 1e-3 { delta } else { 0.0 },
            )
        } else {
            (0.0, current_value)
        }
    }

    /// The same as `compute_fill_start_delta`, but just showing the modulation offset.
    fn compute_modulation_fill_start_delta<P: Param>(param: &P) -> (f32, f32) {
        let modulation_start = param.unmodulated_normalized_value();

        (
            modulation_start,
            param.modulated_normalized_value() - modulation_start,
        )
    }

    /// `self.param_base.set_normalized_value()`, but resulting from a mouse drag. When using the
    /// 'even' stepped slider styles from [`ParamSliderStyle`] this will remap the normalized range
    /// to match up with the fill value display. This still needs to be wrapped in a parameter
    /// automation gesture.
    fn set_normalized_value_drag(&self, cx: &mut EventContext, normalized_value: f32) {
        self.param_base.set_normalized_value(cx, normalized_value);
    }
}

impl<LSafeMode: Lens<Target = SafeModeClamper>> View for RestrictedParamSlider<LSafeMode> {
    fn element(&self) -> Option<&'static str> {
        // We'll reuse the original ParamSlider's styling
        Some("param-slider")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|param_slider_event, meta| match param_slider_event {
            ParamSliderEvent::CancelTextInput => {
                self.text_input_active = false;
                cx.set_active(false);

                meta.consume();
            }
            ParamSliderEvent::TextInput(string) => {
                if let Some(normalized_value) = self.param_base.string_to_normalized_value(string) {
                    self.param_base.begin_set_parameter(cx);
                    self.param_base.set_normalized_value(cx, normalized_value);
                    self.param_base.end_set_parameter(cx);
                }

                self.text_input_active = false;

                meta.consume();
            }
        });

        event.map(|window_event, meta| match window_event {
            // Vizia always captures the third mouse click as a triple click. Treating that triple
            // click as a regular mouse button makes double click followed by another drag work as
            // expected, instead of requiring a delay or an additional click. Double double click
            // still won't work.
            WindowEvent::MouseDown(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                if cx.modifiers.alt() {
                    // ALt+Click brings up a text entry dialog
                    self.text_input_active = true;
                    cx.set_active(true);
                } else if cx.modifiers.command() {
                    // Ctrl+Click, double click, and right clicks should reset the parameter instead
                    // of initiating a drag operation
                    self.param_base.begin_set_parameter(cx);
                    self.param_base
                        .set_normalized_value(cx, self.param_base.default_normalized_value());
                    self.param_base.end_set_parameter(cx);
                } else {
                    self.drag_active = true;
                    cx.capture();
                    // NOTE: Otherwise we don't get key up events
                    cx.focus();
                    cx.set_active(true);

                    // When holding down shift while clicking on a parameter we want to granuarly
                    // edit the parameter without jumping to a new value
                    self.param_base.begin_set_parameter(cx);
                    if cx.modifiers.shift() {
                        self.granular_drag_status = Some(GranularDragStatus {
                            starting_x_coordinate: cx.mouse.cursorx,
                            starting_value: self.param_base.unmodulated_normalized_value(),
                        });
                    } else {
                        self.granular_drag_status = None;
                        self.set_normalized_value_drag(
                            cx,
                            util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
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
                self.param_base.begin_set_parameter(cx);
                self.param_base
                    .set_normalized_value(cx, self.param_base.default_normalized_value());
                self.param_base.end_set_parameter(cx);

                meta.consume();
            }
            WindowEvent::MouseUp(MouseButton::Left) => {
                if self.drag_active {
                    self.drag_active = false;
                    cx.release();
                    cx.set_active(false);

                    self.param_base.end_set_parameter(cx);

                    meta.consume();
                }
            }
            WindowEvent::MouseMove(x, _y) => {
                if self.drag_active {
                    // If shift is being held then the drag should be more granular instead of
                    // absolute
                    if cx.modifiers.shift() {
                        let granular_drag_status =
                            *self
                                .granular_drag_status
                                .get_or_insert_with(|| GranularDragStatus {
                                    starting_x_coordinate: *x,
                                    starting_value: self.param_base.unmodulated_normalized_value(),
                                });

                        // These positions should be compensated for the DPI scale so it remains
                        // consistent
                        let start_x =
                            util::remap_current_entity_x_t(cx, granular_drag_status.starting_value);
                        let delta_x = ((*x - granular_drag_status.starting_x_coordinate)
                            * GRANULAR_DRAG_MULTIPLIER)
                            * cx.style.dpi_factor as f32;

                        self.set_normalized_value_drag(
                            cx,
                            util::remap_current_entity_x_coordinate(cx, start_x + delta_x),
                        );
                    } else {
                        self.granular_drag_status = None;

                        self.set_normalized_value_drag(
                            cx,
                            util::remap_current_entity_x_coordinate(cx, *x),
                        );
                    }
                }
            }
            WindowEvent::KeyUp(_, Some(Key::Shift)) => {
                // If this happens while dragging, snap back to reality uh I mean the current screen
                // position
                if self.drag_active && self.granular_drag_status.is_some() {
                    self.granular_drag_status = None;
                    self.param_base.set_normalized_value(
                        cx,
                        util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
                    );
                }
            }
            WindowEvent::MouseScroll(_scroll_x, scroll_y) if self.use_scroll_wheel => {
                // With a regular scroll wheel `scroll_y` will only ever be -1 or 1, but with smooth
                // scrolling trackpads being a thing `scroll_y` could be anything.
                self.scrolled_lines += scroll_y;

                if self.scrolled_lines.abs() >= 1.0 {
                    let use_finer_steps = cx.modifiers.shift();

                    // Scrolling while dragging needs to be taken into account here
                    if !self.drag_active {
                        self.param_base.begin_set_parameter(cx);
                    }

                    let mut current_value = self.param_base.unmodulated_normalized_value();

                    while self.scrolled_lines >= 1.0 {
                        current_value = self
                            .param_base
                            .next_normalized_step(current_value, use_finer_steps);
                        self.param_base.set_normalized_value(cx, current_value);
                        self.scrolled_lines -= 1.0;
                    }

                    while self.scrolled_lines <= -1.0 {
                        current_value = self
                            .param_base
                            .previous_normalized_step(current_value, use_finer_steps);
                        self.param_base.set_normalized_value(cx, current_value);
                        self.scrolled_lines += 1.0;
                    }

                    if !self.drag_active {
                        self.param_base.end_set_parameter(cx);
                    }
                }

                meta.consume();
            }
            _ => {}
        });
    }
}
