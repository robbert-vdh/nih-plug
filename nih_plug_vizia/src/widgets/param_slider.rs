//! A slider that integrates with NIH-plug's [`Param`] types.

use nih_plug::prelude::Param;
use vizia::prelude::*;

use super::param_base::ParamWidgetBase;
use super::util::{self, ModifiersExt};

/// When shift+dragging a parameter, one pixel dragged corresponds to this much change in the
/// normalized parameter.
const GRANULAR_DRAG_MULTIPLIER: f32 = 0.1;

/// A slider that integrates with NIH-plug's [`Param`] types. Use the
/// [`set_style()`][ParamSliderExt::set_style()] method to change how the value gets displayed.
///
/// TODO: Handle scrolling for steps (and shift+scroll for smaller steps?)
#[derive(Lens)]
pub struct ParamSlider {
    param_base: ParamWidgetBase,

    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,
    /// We keep track of the start coordinate and normalized value when holding down Shift while
    /// dragging for higher precision dragging. This is a `None` value when granular dragging is not
    /// active.
    granular_drag_start_x_value: Option<(f32, f32)>,

    /// What style to use for the slider.
    style: ParamSliderStyle,
    /// Will be set to `true` when the field gets Alt+Click'ed which will replace the label with a
    /// text box.
    text_input_active: bool,
}

/// How the [`ParamSlider`] should display its values. Set this using
/// [`ParamSliderExt::set_style()`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Data)]
pub enum ParamSliderStyle {
    /// Visualize the offset from the default value for continuous parameters with a default value
    /// at around half of its range, fill the bar from the left for discrete parameters and
    /// continuous parameters without centered default values.
    Centered,
    /// Always fill the bar starting from the left.
    FromLeft,
    /// Show the current step instead of filling a portion of the bar, useful for discrete
    /// parameters. Set `even` to `true` to distribute the ticks evenly instead of following the
    /// parameter's distribution. This can be desireable because discrete parameters have smaller
    /// ranges near the edges (they'll span only half the range, which can make the display look
    /// odd).
    CurrentStep { even: bool },
    /// The same as `CurrentStep`, but overlay the labels over the steps instead of showing the
    /// active value. Only useful for discrete parameters with two, maybe three possible values.
    CurrentStepLabeled { even: bool },
}

enum ParamSliderEvent {
    /// Text input has been cancelled without submitting a new value.
    CancelTextInput,
    /// A new value has been sent by the text input dialog after pressing Enter.
    TextInput(String),
}

impl ParamSlider {
    /// Creates a new [`ParamSlider`] for the given parameter. To accommodate VIZIA's mapping system,
    /// you'll need to provide a lens containing your `Params` implementation object (check out how
    /// the `Data` struct is used in `gain_gui_vizia`) and a projection function that maps the
    /// `Params` object to the parameter you want to display a widget for. Parameter changes are
    /// handled by emitting [`ParamEvent`][super::ParamEvent]s which are automatically handled by
    /// the VIZIA wrapper.
    ///
    /// See [`ParamSliderExt`] for additional options.
    pub fn new<L, Params, P, FMap>(
        cx: &mut Context,
        params: L,
        params_to_param: FMap,
    ) -> Handle<Self>
    where
        L: Lens<Target = Params> + Clone,
        Params: 'static,
        P: Param + 'static,
        FMap: Fn(&Params) -> &P + Copy + 'static,
    {
        // We'll visualize the difference between the current value and the default value if the
        // default value lies somewhere in the middle and the parameter is continuous. Otherwise
        // this approach looks a bit jarring.
        Self {
            param_base: ParamWidgetBase::new(cx, params.clone(), params_to_param),

            drag_active: false,
            granular_drag_start_x_value: None,

            style: ParamSliderStyle::Centered,
            text_input_active: false,
        }
        .build(
            cx,
            ParamWidgetBase::view(params, params_to_param, move |cx, param_data| {
                Binding::new(cx, ParamSlider::style, move |cx, style| {
                    let style = style.get(cx);

                    let default_value = param_data.param().default_normalized_value();
                    let step_count = param_data.param().step_count();

                    // Can't use `.to_string()` here as that would include the modulation.
                    let unmodulated_normalized_value_lens =
                        param_data.make_lens(|param| param.unmodulated_normalized_value());
                    let display_value_lens = param_data.make_lens(|param| {
                        param.normalized_value_to_string(param.unmodulated_normalized_value(), true)
                    });

                    // This is used to draw labels for `CurrentStepLabeled`
                    let make_preview_value_lens = {
                        let param_data = param_data.clone();
                        move |normalized_value| {
                            param_data.make_lens(move |param| {
                                param.normalized_value_to_string(normalized_value, true)
                            })
                        }
                    };

                    // The resulting tuple `(start_t, delta)` corresponds to the start and the
                    // signed width of the bar. `start_t` is in `[0, 1]`, and `delta` is in
                    // `[-1, 1]`.
                    let draw_fill_from_default = matches!(style, ParamSliderStyle::Centered)
                        && step_count.is_none()
                        && (0.45..=0.55).contains(&default_value);
                    let fill_start_delta_lens = unmodulated_normalized_value_lens.map({
                        let param_data = param_data.clone();

                        move |current_value| {
                            match style {
                                ParamSliderStyle::Centered if draw_fill_from_default => {
                                    let delta = (default_value - current_value).abs();

                                    // Don't draw the filled portion at all if it could have been a
                                    // rounding error since those slivers just look weird
                                    (
                                        default_value.min(*current_value),
                                        if delta >= 1e-3 { delta } else { 0.0 },
                                    )
                                }
                                ParamSliderStyle::Centered | ParamSliderStyle::FromLeft => {
                                    (0.0, *current_value)
                                }
                                ParamSliderStyle::CurrentStep { even: true }
                                | ParamSliderStyle::CurrentStepLabeled { even: true }
                                    if step_count.is_some() =>
                                {
                                    // Assume the normalized value is distributed evenly
                                    // across the range.
                                    let step_count = step_count.unwrap() as f32;
                                    let discrete_values = step_count + 1.0;
                                    let previous_step =
                                        (current_value * step_count) / discrete_values;

                                    (previous_step, discrete_values.recip())
                                }
                                ParamSliderStyle::CurrentStep { .. }
                                | ParamSliderStyle::CurrentStepLabeled { .. } => {
                                    let previous_step =
                                        param_data.param().previous_normalized_step(*current_value);
                                    let next_step =
                                        param_data.param().next_normalized_step(*current_value);

                                    (
                                        (previous_step + current_value) / 2.0,
                                        ((next_step - current_value)
                                            + (current_value - previous_step))
                                            / 2.0,
                                    )
                                }
                            }
                        }
                    });

                    // If the parameter is being modulated by the host (this only works for CLAP
                    // plugins with hosts that support this), then this is the difference
                    // between the 'true' value and the current value after modulation has been
                    // applied.
                    let modulation_start_delta_lens = param_data.make_lens(move |param| {
                        match style {
                            // Don't show modulation for stepped parameters since it wouldn't
                            // make a lot of sense visually
                            ParamSliderStyle::CurrentStep { .. }
                            | ParamSliderStyle::CurrentStepLabeled { .. } => (0.0, 0.0),
                            ParamSliderStyle::Centered | ParamSliderStyle::FromLeft => {
                                let modulation_start = param.unmodulated_normalized_value();

                                (
                                    modulation_start,
                                    param.modulated_normalized_value() - modulation_start,
                                )
                            }
                        }
                    });

                    // Only draw the text input widget when it gets focussed. Otherwise, overlay the
                    // label with the slider. Creating the textbox based on
                    // `ParamSliderInternal::text_input_active` lets us focus the textbox when it gets
                    // created.
                    Binding::new(
                        cx,
                        ParamSlider::text_input_active,
                        move |cx, text_input_active| {
                            if text_input_active.get(cx) {
                                Textbox::new(cx, display_value_lens.clone())
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
                            } else {
                                let display_value_lens = display_value_lens.clone();
                                let fill_start_delta_lens = fill_start_delta_lens.clone();
                                let modulation_start_delta_lens =
                                    modulation_start_delta_lens.clone();
                                let make_preview_value_lens = make_preview_value_lens.clone();

                                ZStack::new(cx, move |cx| {
                                    // The filled bar portion. This can be visualized in a couple
                                    // different ways depending on the current style property. See
                                    // [`ParamSliderStyle`].
                                    Element::new(cx)
                                        .class("fill")
                                        .height(Stretch(1.0))
                                        .left(
                                            fill_start_delta_lens
                                                .clone()
                                                .map(|(start_t, _)| Percentage(start_t * 100.0)),
                                        )
                                        .width(
                                            fill_start_delta_lens
                                                .map(|(_, delta)| Percentage(delta * 100.0)),
                                        )
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
                                        .left(modulation_start_delta_lens.map(
                                            |(start_t, delta)| {
                                                if *delta < 0.0 {
                                                    Percentage((start_t + delta) * 100.0)
                                                } else {
                                                    Percentage(start_t * 100.0)
                                                }
                                            },
                                        ))
                                        .hoverable(false);

                                    // Either display the current value, or display all values over the
                                    // parameter's steps
                                    // TODO: Do the same thing as in the iced widget where we draw the
                                    //       text overlapping the fill area slightly differently. We can
                                    //       set the cip region directly in vizia.
                                    match (style, step_count) {
                                        (
                                            ParamSliderStyle::CurrentStepLabeled { .. },
                                            Some(step_count),
                                        ) => {
                                            HStack::new(cx, |cx| {
                                                // There are step_count + 1 possible values for a
                                                // discrete parameter
                                                for value in 0..step_count + 1 {
                                                    let normalized_value =
                                                        value as f32 / step_count as f32;
                                                    let preview_lens =
                                                        make_preview_value_lens(normalized_value);

                                                    Label::new(cx, preview_lens)
                                                        .class("value")
                                                        .class("value--multiple")
                                                        .child_space(Stretch(1.0))
                                                        .height(Stretch(1.0))
                                                        .width(Stretch(1.0))
                                                        .hoverable(false);
                                                }
                                            })
                                            .height(Stretch(1.0))
                                            .width(Stretch(1.0))
                                            .hoverable(false);
                                        }
                                        _ => {
                                            Label::new(cx, display_value_lens)
                                                .class("value")
                                                .class("value--single")
                                                .child_space(Stretch(1.0))
                                                .height(Stretch(1.0))
                                                .width(Stretch(1.0))
                                                .hoverable(false);
                                        }
                                    };
                                })
                                .hoverable(false);
                            }
                        },
                    );
                });
            }),
        )
    }

    /// `self.param_base.set_normalized_value()`, but resulting from a mouse drag. When using the
    /// 'even' stepped slider styles from [`ParamSliderStyle`] this will remap the normalized range
    /// to match up with the fill value display. This still needs to be wrapped in a parameter
    /// automation gesture.
    fn set_normalized_value_drag(&self, cx: &mut EventContext, normalized_value: f32) {
        let normalized_value = match (self.style, self.param_base.step_count()) {
            (
                ParamSliderStyle::CurrentStep { even: true }
                | ParamSliderStyle::CurrentStepLabeled { even: true },
                Some(step_count),
            ) => {
                // We'll remap the value range to be the same as the displayed range, e.g. with each
                // value occupying an equal area on the slider instead of the centers of those
                // ranges being distributed over the entire `[0, 1]` range.
                let discrete_values = step_count as f32 + 1.0;
                let rounded_value = ((normalized_value * discrete_values) - 0.5).round();
                rounded_value / step_count as f32
            }
            _ => normalized_value,
        };

        self.param_base.set_normalized_value(cx, normalized_value);
    }
}

impl View for ParamSlider {
    fn element(&self) -> Option<&'static str> {
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
                    // Ctrl+Click and double click should reset the parameter instead of initiating
                    // a drag operation
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
                        self.granular_drag_start_x_value = Some((
                            cx.mouse.cursorx,
                            self.param_base.unmodulated_normalized_value(),
                        ));
                    } else {
                        self.granular_drag_start_x_value = None;
                        self.set_normalized_value_drag(
                            cx,
                            util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
                        );
                    }
                }

                meta.consume();
            }
            WindowEvent::MouseDoubleClick(MouseButton::Left) => {
                // Ctrl+Click and double click should reset the parameter instead of initiating
                // a drag operation
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
                        let (drag_start_x, drag_start_value) =
                            *self.granular_drag_start_x_value.get_or_insert_with(|| {
                                (
                                    cx.mouse.cursorx,
                                    self.param_base.unmodulated_normalized_value(),
                                )
                            });

                        self.set_normalized_value_drag(
                            cx,
                            util::remap_current_entity_x_coordinate(
                                cx,
                                // This can be optimized a bit
                                util::remap_current_entity_x_t(cx, drag_start_value)
                                    + (*x - drag_start_x) * GRANULAR_DRAG_MULTIPLIER,
                            ),
                        );
                    } else {
                        self.granular_drag_start_x_value = None;

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
                if self.drag_active && self.granular_drag_start_x_value.is_some() {
                    self.granular_drag_start_x_value = None;
                    self.param_base.set_normalized_value(
                        cx,
                        util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
                    );
                }
            }
            _ => {}
        });
    }
}

/// Extension methods for [`ParamSlider`] handles.
pub trait ParamSliderExt {
    /// Change how the [`ParamSlider`] visualizes the current value.
    fn set_style(self, style: ParamSliderStyle) -> Self;
}

impl ParamSliderExt for Handle<'_, ParamSlider> {
    fn set_style(self, style: ParamSliderStyle) -> Self {
        self.modify(|param_slider: &mut ParamSlider| param_slider.style = style)
    }
}
