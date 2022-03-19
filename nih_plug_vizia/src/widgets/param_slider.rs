//! A slider that integrates with NIH-plug's [`Param`] types.

use nih_plug::param::internals::ParamPtr;
use nih_plug::prelude::{Param, ParamSetter};
use vizia::*;

use super::util::{self, ModifiersExt};
use super::RawParamEvent;

/// When shift+dragging a parameter, one pixel dragged corresponds to this much change in the
/// noramlized parameter.
const GRANULAR_DRAG_MULTIPLIER: f32 = 0.1;

/// A slider that integrates with NIH-plug's [`Param`] types.
///
/// TODO: Handle scrolling for steps (and shift+scroll for smaller steps?)
/// TODO: We may want to add a couple dedicated event handlers if it seems like those would be
///       useful, having a completely self contained widget is perfectly fine for now though
/// TODO: Text entry doesn't work correctly yet because vizia's still missing some functionality
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
    /// Show the current step instead of filling a portion fothe bar, useful for discrete
    /// parameters.
    CurrentStep,
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
    fn event(&mut self, cx: &mut Context, event: &mut Event) {
        if let Some(param_slider_internal_event) = event.message.downcast() {
            match param_slider_internal_event {
                ParamSliderInternalEvent::SetStyle(style) => self.style = *style,
                ParamSliderInternalEvent::SetTextInputActive(value) => {
                    cx.current.set_active(cx, *value);
                    self.text_input_active = *value;
                }
            }
        }
    }
}

impl ParamSlider {
    /// Creates a new [`ParamSlider`] for the given parameter. To accomdate VIZIA's mapping system,
    /// you'll need to provide a lens containing your `Params` implementation object (check out how
    /// the `Data` struct is used in `gain_gui_vizia`), the `ParamSetter` for retrieving the
    /// parameter's default value, and a projection function that maps the `Params` object to the
    /// parameter you want to display a widget for.
    ///
    /// See [`ParamSliderExt`] for additonal options.
    pub fn new<'a, L, Params, P, F>(
        cx: &'a mut Context,
        params: L,
        setter: &ParamSetter,
        params_to_param: F,
    ) -> Handle<'a, ParamSlider>
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
        let default_value = unsafe {
            setter
                .raw_context
                .raw_default_normalized_param_value(param_ptr)
        };
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

                Binding::new(
                    cx,
                    ParamSliderInternal::text_input_active,
                    move |cx, text_input_active| {
                        let param_display_value_lens =
                            params.map(move |params| params_to_param(params).to_string());
                        let normalized_param_value_lens =
                            params.map(move |params| params_to_param(params).normalized_value());

                        // Only draw the text input widget when it gets focussed. Otherwise, overlay the
                        // label with the slider.
                        if *text_input_active.get(cx) {
                            Textbox::new(cx, param_display_value_lens)
                                .class("value-entry")
                                .on_submit(|cx, string| {
                                    cx.emit(ParamSliderEvent::TextInput(string))
                                })
                                .on_focus_out(|cx| cx.emit(ParamSliderEvent::CancelTextInput))
                                .child_space(Stretch(1.0))
                                .height(Stretch(1.0))
                                .width(Stretch(1.0));
                        } else {
                            ZStack::new(cx, move |cx| {
                                // The filled bar portion. This can be visualized in a couple different
                                // ways depending on the current style property. See
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
                                                    // Don't draw the filled portion at all if it could have been a
                                                    // rounding error since those slivers just look weird
                                                    if delta >= 1e-3 { delta } else { 0.0 },
                                                )
                                            }
                                            ParamSliderStyle::Centered
                                            | ParamSliderStyle::FromLeft => (0.0, current_value),
                                            ParamSliderStyle::CurrentStep => {
                                                let previous_step = unsafe {
                                                    param_ptr
                                                        .previous_normalized_step(current_value)
                                                };
                                                let next_step = unsafe {
                                                    param_ptr.next_normalized_step(current_value)
                                                };
                                                (
                                                    (previous_step + current_value) / 2.0,
                                                    (next_step + current_value) / 2.0,
                                                )
                                            }
                                        };

                                        handle
                                            .left(Percentage(start_t * 100.0))
                                            .width(Percentage(delta * 100.0));
                                    })
                                    // Hovering is handled on the param slider as a whole, this should
                                    // not affect that
                                    .hoverable(false);

                                Label::new(cx, param_display_value_lens)
                                    .class("value")
                                    .height(Stretch(1.0))
                                    .width(Stretch(1.0))
                                    .hoverable(false);
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
}

impl View for ParamSlider {
    fn element(&self) -> Option<String> {
        Some(String::from("param-slider"))
    }

    fn event(&mut self, cx: &mut Context, event: &mut Event) {
        if let Some(param_slider_event) = event.message.downcast() {
            match param_slider_event {
                ParamSliderEvent::CancelTextInput => {
                    cx.emit(ParamSliderInternalEvent::SetTextInputActive(false))
                }
                ParamSliderEvent::TextInput(string) => {
                    if let Some(normalized_value) =
                        unsafe { self.param_ptr.string_to_normalized_value(string) }
                    {
                        cx.emit(RawParamEvent::BeginSetParameter(self.param_ptr));
                        self.set_normalized_value(cx, normalized_value);
                        cx.emit(RawParamEvent::EndSetParameter(self.param_ptr));
                    }

                    cx.emit(ParamSliderInternalEvent::SetTextInputActive(false))
                }
            }
        }

        if let Some(window_event) = event.message.downcast() {
            match window_event {
                WindowEvent::MouseDown(MouseButton::Left) => {
                    if cx.modifiers.alt() {
                        cx.emit(ParamSliderInternalEvent::SetTextInputActive(true));
                        // TODO: Once vizia implements it: (and probably do this from
                        //       `SetTextInputActive`)
                        //       - Focus the text box
                        //       - Select all text
                        //       - Move the caret to the end
                    } else if cx.modifiers.command() || self.is_double_click {
                        // Ctrl+Click and double click should reset the parameter instead of initiating
                        // a drag operation
                        cx.emit(RawParamEvent::BeginSetParameter(self.param_ptr));
                        cx.emit(RawParamEvent::ResetParameter(self.param_ptr));
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
                            self.set_normalized_value(
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

                            self.set_normalized_value(
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

                            self.set_normalized_value(
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
