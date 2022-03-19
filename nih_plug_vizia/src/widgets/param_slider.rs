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
/// TODO: Implement ALt+Click text input in this version
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
    /// We keep track of the start coordinate holding down Shift while dragging for higher precision
    /// dragging. This is a `None` value when granular dragging is not active.
    granular_drag_start_x: Option<f32>,
}

impl ParamSlider {
    /// Creates a new [`ParamSlider`] for the given parameter. To accomdate VIZIA's mapping system,
    /// you'll need to provide a lens containing your `Params` implementation object (check out how
    /// the `Data` struct is used in `gain_gui_vizia`), the `ParamSetter` for retrieving the
    /// parameter's default value, and a projection function that maps the `Params` object to the
    /// parameter you want to display a widget for.
    pub fn new<'a, L, Params, P, F>(
        cx: &'a mut Context,
        params: L,
        setter: &ParamSetter,
        params_to_param: F,
    ) -> Handle<'a, Self>
    where
        L: Lens<Target = Params>,
        F: 'static + Fn(&Params) -> &P + Copy,
        Params: 'static,
        P: Param,
    {
        let param_display_value_lens = params
            .clone()
            .map(move |params| params_to_param(params).to_string());
        let normalized_param_value_lens = params
            .clone()
            .map(move |params| params_to_param(params).normalized_value());

        // We'll visualize the difference between the current value and the default value if the
        // default value lies somewhere in the middle and the parameter is continuous. Otherwise
        // this appraoch looks a bit jarring.
        // We need to do a bit of a nasty and erase the lifetime bound by going through the raw
        // GuiContext and a ParamPtr.
        let param_ptr = *params
            .clone()
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
        let draw_fill_from_default = step_count.is_none() && (0.45..=0.55).contains(&default_value);

        Self {
            param_ptr,

            drag_active: false,
            is_double_click: false,
            granular_drag_start_x: None,
        }
        .build2(cx, |cx| {
            ZStack::new(cx, move |cx| {
                // The filled bar portion
                Element::new(cx).class("fill").height(Stretch(1.0)).bind(
                    normalized_param_value_lens,
                    move |handle, value| {
                        let current_value = *value.get(handle.cx);
                        if draw_fill_from_default {
                            handle
                                .left(Percentage(default_value.min(current_value) * 100.0))
                                .right(Percentage(
                                    100.0 - (default_value.max(current_value) * 100.0),
                                ));
                        } else {
                            handle
                                .left(Percentage(0.0))
                                .right(Percentage(100.0 - (current_value * 100.0)));
                        }
                    },
                );

                // Only draw the text input widget when it gets focussed. Otherwise, overlay the label with
                // the slider.
                // TODO: Text entry stuff
                Label::new(cx, param_display_value_lens)
                    .height(Stretch(1.0))
                    .width(Stretch(1.0));
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
        if let Some(window_event) = event.message.downcast() {
            match window_event {
                WindowEvent::MouseDown(MouseButton::Left) => {
                    // Ctrl+Click and double click should reset the parameter instead of initiating
                    // a drag operation
                    // TODO: Handle Alt+Click for text entry
                    if cx.modifiers.command() || self.is_double_click {
                        self.is_double_click = false;

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
                            self.granular_drag_start_x = Some(cx.mouse.cursorx);
                        } else {
                            self.granular_drag_start_x = None;
                            self.set_normalized_value(
                                cx,
                                util::remap_current_entity_x_coordinate(cx, cx.mouse.cursorx),
                            );
                        }
                    }
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
                            let drag_start_x =
                                *self.granular_drag_start_x.get_or_insert(cx.mouse.cursorx);

                            self.set_normalized_value(
                                cx,
                                util::remap_current_entity_x_coordinate(
                                    cx,
                                    drag_start_x + (*x - drag_start_x) * GRANULAR_DRAG_MULTIPLIER,
                                ),
                            );
                        } else {
                            self.granular_drag_start_x = None;

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
                    if self.drag_active && self.granular_drag_start_x.is_some() {
                        self.granular_drag_start_x = None;
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
