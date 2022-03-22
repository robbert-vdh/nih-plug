//! A slider that integrates with NIH-plug's [`Param`] types.

use nih_plug::param::internals::ParamPtr;
use nih_plug::prelude::Param;
use vizia::*;

use super::util::{self, ModifiersExt};
use super::RawParamEvent;

/// When shift+dragging a parameter, one pixel dragged corresponds to this much change in the
/// noramlized parameter.
const GRANULAR_DRAG_MULTIPLIER: f32 = 0.1;

/// A slider that integrates with NIH-plug's [`Param`] types. Use the
/// [`set_style()`][Self::set_style()] method to change how the value gets displayed.
///
/// TODO: Handle scrolling for steps (and shift+scroll for smaller steps?)
/// TODO: We may want to add a couple dedicated event handlers if it seems like those would be
///       useful, having a completely self contained widget is perfectly fine for now though
pub struct ParamSlider {
    // We're not allowed to store a reference to the parameter internally, at least not in the
    // struct that implements [`View`]
    param_ptr: ParamPtr,

    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,
    /// Whether the next click is a double click. Vizia will send a double click event followed by a
    /// regular mouse down event when double clicking.
    is_double_click: bool,
    /// We keep track of the start coordinate and normalized value when holding down Shift while
    /// dragging for higher precision dragging. This is a `None` value when granular dragging is not
    /// active.
    granular_drag_start_x_value: Option<(f32, f32)>,
}

/// How the [`ParamSlider`] should display its values. Set this using
/// [`ParamSliderExt::slider_style()`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Data)]
pub enum ParamSliderStyle {
    /// Visualize the offset from the default value for continuous parameters with a default value
    /// at around half of its range, fill the bar from the left for discrete parameters and
    /// continous parameters without centered default values.
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
    /// A new value has been sent by the text input dialog after pressint Enter.
    TextInput(String),
}

/// Internal param slider state the view needs to react to.
#[derive(Lens)]
// TODO: Lens requires everything to be marked as `pub`
pub struct ParamSliderInternal {
    /// What style to use for the slider.
    style: ParamSliderStyle,
    /// Will be set to `true` when the field gets Alt+Click'ed which will replae the label with a
    /// text box.
    text_input_active: bool,
}

enum ParamSliderInternalEvent {
    SetStyle(ParamSliderStyle),
    SetTextInputActive(bool),
}

impl Model for ParamSliderInternal {
    fn event(&mut self, _cx: &mut Context, event: &mut Event) {
        if let Some(param_slider_internal_event) = event.message.downcast() {
            match param_slider_internal_event {
                ParamSliderInternalEvent::SetStyle(style) => self.style = *style,
                ParamSliderInternalEvent::SetTextInputActive(active) => {
                    // When this gets set to `true` the textbox widget will be created, and when it
                    // gets created we'll focus it and select all text
                    self.text_input_active = *active;
                }
            }
        }
    }
}

impl ParamSlider {
    /// Creates a new [`ParamSlider`] for the given parameter. To accomdate VIZIA's mapping system,
    /// you'll need to provide a lens containing your `Params` implementation object (check out how
    /// the `Data` struct is used in `gain_gui_vizia`) and a projection function that maps the
    /// `Params` object to the parameter you want to display a widget for. Parameter changes are
    /// handled by emitting [`ParamEvent`][super::ParamEvent]s which are automatically handled by
    /// the VIZIA wrapper.
    ///
    /// See [`ParamSliderExt`] for additonal options.
    pub fn new<L, Params, P, F>(
        cx: &mut Context,
        params: L,
        params_to_param: F,
    ) -> Handle<'_, ParamSlider>
    where
        L: Lens<Target = Params> + Copy,
        F: 'static + Fn(&Params) -> &P + Copy,
        Params: 'static,
        P: Param,
    {
        // We'll visualize the difference between the current value and the default value if the
        // default value lies somewhere in the middle and the parameter is continuous. Otherwise
        // this appraoch looks a bit jarring.
        // We need to do a bit of a nasty and erase the lifetime bound by going through the raw
        // GuiContext and a ParamPtr.
        let param_ptr = *params
            .map(move |params| params_to_param(params).as_ptr())
            .get(cx);
        let default_value = *params
            .map(move |params| params_to_param(params).default_normalized_value())
            .get(cx);
        let step_count = *params
            .map(move |params| params_to_param(params).step_count())
            .get(cx);

        Self {
            param_ptr,

            drag_active: false,
            is_double_click: false,
            granular_drag_start_x_value: None,
        }
        .build2(cx, move |cx| {
            ParamSliderInternal {
                style: ParamSliderStyle::Centered,
                text_input_active: false,
            }
            .build(cx);

            Binding::new(cx, ParamSliderInternal::style, move |cx, style| {
                let style = *style.get(cx);
                let draw_fill_from_default = matches!(style, ParamSliderStyle::Centered)
                    && step_count.is_none()
                    && (0.45..=0.55).contains(&default_value);

                // Only draw the text input widget when it gets focussed. Otherwise, overlay the
                // label with the slider. Creating the textbox based on
                // `ParamSliderInternal::text_input_active` lets us focus the textbox when it gets
                // created.
                Binding::new(
                    cx,
                    ParamSliderInternal::text_input_active,
                    move |cx, text_input_active| {
                        let param_display_value_lens =
                            params.map(move |params| params_to_param(params).to_string());
                        let param_preview_display_value_lens = |normalized_value| {
                            params.map(move |params| {
                                params_to_param(params)
                                    .normalized_value_to_string(normalized_value, true)
                            })
                        };
                        let normalized_param_value_lens =
                            params.map(move |params| params_to_param(params).normalized_value());

                        if *text_input_active.get(cx) {
                            Textbox::new(cx, param_display_value_lens)
                                .class("value-entry")
                                .on_submit(|cx, string| {
                                    cx.emit(ParamSliderEvent::TextInput(string))
                                })
                                .on_edit_end(|cx| {
                                    cx.emit(ParamSliderEvent::CancelTextInput);
                                })
                                .on_build(|cx| {
                                    cx.emit(TextEvent::StartEdit);
                                    cx.emit(TextEvent::SelectAll);
                                })
                                .child_space(Stretch(1.0))
                                .height(Stretch(1.0))
                                .width(Stretch(1.0));
                        } else {
                            ZStack::new(cx, move |cx| {
                                // The filled bar portion. This can be visualized in a couple
                                // different ways depending on the current style property. See
                                // [`ParamSliderStyle`].
                                Element::new(cx)
                                    .class("fill")
                                    .height(Stretch(1.0))
                                    .bind(normalized_param_value_lens, move |handle, value| {
                                        let current_value = *value.get(handle.cx);
                                        let (start_t, delta) = match style {
                                            ParamSliderStyle::Centered
                                                if draw_fill_from_default =>
                                            {
                                                let delta = (default_value - current_value).abs();
                                                (
                                                    default_value.min(current_value),
                                                    // Don't draw the filled portion at all if it
                                                    // could have been a rounding error since those
                                                    // slivers just look weird
                                                    if delta >= 1e-3 { delta } else { 0.0 },
                                                )
                                            }
                                            ParamSliderStyle::Centered
                                            | ParamSliderStyle::FromLeft => (0.0, current_value),
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
                                                let previous_step = unsafe {
                                                    param_ptr
                                                        .previous_normalized_step(current_value)
                                                };
                                                let next_step = unsafe {
                                                    param_ptr.next_normalized_step(current_value)
                                                };
                                                (
                                                    (previous_step + current_value) / 2.0,
                                                    ((next_step - current_value)
                                                        + (current_value - previous_step))
                                                        / 2.0,
                                                )
                                            }
                                        };

                                        handle
                                            .left(Percentage(start_t * 100.0))
                                            .width(Percentage(delta * 100.0));
                                    })
                                    // Hovering is handled on the param slider as a whole, this
                                    // should not affect that
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
                                                Label::new(
                                                    cx,
                                                    param_preview_display_value_lens(
                                                        normalized_value,
                                                    ),
                                                )
                                                .class("value")
                                                .class("value--multiple")
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
                                        Label::new(cx, param_display_value_lens)
                                            .class("value")
                                            .class("value--single")
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
        })
    }

    /// Set the normalized value for a parameter if that would change the parameter's plain value
    /// (to avoid unnecessary duplicate parameter changes). The begin- and end set parameter
    /// messages need to be sent before calling this function.
    fn set_normalized_value(&self, cx: &mut Context, normalized_value: f32) {
        // This snaps to the nearest plain value if the parameter is stepped in some way.
        // TODO: As an optimization, we could add a `const CONTINUOUS: bool` to the parameter to
        //       avoid this normalized->plain->normalized conversion for parameters that don't need
        //       it
        let plain_value = unsafe { self.param_ptr.preview_plain(normalized_value) };
        let current_plain_value = unsafe { self.param_ptr.plain_value() };
        if plain_value != current_plain_value {
            // For the aforementioned snapping
            let normalized_plain_value = unsafe { self.param_ptr.preview_normalized(plain_value) };
            cx.emit(RawParamEvent::SetParameterNormalized(
                self.param_ptr,
                normalized_plain_value,
            ));
        }
    }

    /// `set_normalized_value()`, but resulting from a mouse drag. When using the 'even' stepped
    /// slider styles from [`ParamSliderStyle`] this will remap the normalized range to match up
    /// with the fill value display.
    fn set_normalized_value_drag(&self, cx: &mut Context, normalized_value: f32) {
        let normalized_value = match (cx.data(), unsafe { self.param_ptr.step_count() }) {
            (
                Some(ParamSliderInternal {
                    style:
                        ParamSliderStyle::CurrentStep { even: true }
                        | ParamSliderStyle::CurrentStepLabeled { even: true },
                    ..
                }),
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

        self.set_normalized_value(cx, normalized_value);
    }
}

impl View for ParamSlider {
    fn element(&self) -> Option<String> {
        Some(String::from("param-slider"))
    }

    fn event(&mut self, cx: &mut Context, event: &mut Event) {
        if let Some(param_slider_event) = event.message.downcast() {
            match param_slider_event {
                ParamSliderEvent::CancelTextInput => {
                    cx.emit(ParamSliderInternalEvent::SetTextInputActive(false));
                    cx.current.set_active(cx, false);
                }
                ParamSliderEvent::TextInput(string) => {
                    if let Some(normalized_value) =
                        unsafe { self.param_ptr.string_to_normalized_value(string) }
                    {
                        cx.emit(RawParamEvent::BeginSetParameter(self.param_ptr));
                        self.set_normalized_value(cx, normalized_value);
                        cx.emit(RawParamEvent::EndSetParameter(self.param_ptr));
                    }

                    cx.emit(ParamSliderInternalEvent::SetTextInputActive(false));
                }
            }
        }

        if let Some(window_event) = event.message.downcast() {
            match window_event {
                WindowEvent::MouseDown(MouseButton::Left) => {
                    if cx.modifiers.alt() {
                        // ALt+Click brings up a text entry dialog
                        cx.emit(ParamSliderInternalEvent::SetTextInputActive(true));
                        cx.current.set_active(cx, true);
                    } else if cx.modifiers.command() || self.is_double_click {
                        // Ctrl+Click and double click should reset the parameter instead of initiating
                        // a drag operation
                        cx.emit(RawParamEvent::BeginSetParameter(self.param_ptr));
                        cx.emit(RawParamEvent::SetParameterNormalized(
                            self.param_ptr,
                            unsafe { self.param_ptr.default_normalized_value() },
                        ));
                        cx.emit(RawParamEvent::EndSetParameter(self.param_ptr));
                    } else {
                        self.drag_active = true;
                        cx.capture();
                        // NOTE: Otherwise we don't get key up events
                        cx.focused = cx.current;
                        cx.current.set_active(cx, true);

                        // When holding down shift while clicking on a parameter we want to
                        // granuarly edit the parameter without jumping to a new value
                        cx.emit(RawParamEvent::BeginSetParameter(self.param_ptr));
                        if cx.modifiers.shift() {
                            self.granular_drag_start_x_value = Some((cx.mouse.cursorx, unsafe {
                                self.param_ptr.normalized_value()
                            }));
                        } else {
                            self.granular_drag_start_x_value = None;
                            self.set_normalized_value_drag(
                                cx,
                                util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
                            );
                        }
                    }

                    // We'll set this here because weird things like Alt+double click should not
                    // cause the next click to become a reset
                    self.is_double_click = false;
                }
                WindowEvent::MouseDoubleClick(MouseButton::Left) => {
                    // Vizia will send a regular mouse down after this, so we'll handle the reset
                    // there
                    self.is_double_click = true;
                }
                WindowEvent::MouseUp(MouseButton::Left) => {
                    if self.drag_active {
                        self.drag_active = false;
                        cx.release();
                        cx.current.set_active(cx, false);

                        cx.emit(RawParamEvent::EndSetParameter(self.param_ptr));
                    }
                }
                WindowEvent::MouseMove(x, _y) => {
                    if self.drag_active {
                        // If shift is being held then the drag should be more granular instead of
                        // absolute
                        if cx.modifiers.shift() {
                            let (drag_start_x, drag_start_value) =
                                *self.granular_drag_start_x_value.get_or_insert_with(|| {
                                    (cx.mouse.cursorx, unsafe {
                                        self.param_ptr.normalized_value()
                                    })
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
                        self.set_normalized_value(
                            cx,
                            util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

/// Extension methods for [`ParamSlider`] handles.
pub trait ParamSliderExt {
    /// Change how the [`ParamSlider`] visualizes the current value.
    fn set_style(self, style: ParamSliderStyle) -> Self;
}

impl ParamSliderExt for Handle<'_, ParamSlider> {
    fn set_style(self, style: ParamSliderStyle) -> Self {
        self.cx.event_queue.push_back(
            Event::new(ParamSliderInternalEvent::SetStyle(style))
                .target(self.entity)
                .origin(self.entity)
                .propagate(Propagation::Subtree),
        );

        self
    }
}
