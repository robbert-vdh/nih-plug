//! A special label that integrates with NIH-plug's [`Param`] types.

use nih_plug::prelude::Param;
use vizia::prelude::*;

use super::param_base::ParamWidgetBase;
use super::util::ModifiersExt;

/// A special label that integrates with NIH-plug's [`Param`] types. This should only be used to
/// allow text entry for parameters with no dedicated control, like the two parameters bound to an
/// X-Y pad. Use regular [`Label`]s instead when the label accompanies a parameter widget.
#[derive(Lens)]
pub struct ParamLabel {
    param_base: ParamWidgetBase,

    /// Will be set to `true` when the field gets Alt+Click'ed which will replace the label with a
    /// text box.
    text_input_active: bool,

    /// The label's orientation. Automatically deduced from the widget's width and height.
    orientation: Orientation,
    // HACK: These two fields are needed because vizia doesn't support rotating in layouts, so you
    //       need to hack around this using explicit sizes.
    /// This element's width in pixels, used to explicitly set the size of the contained label or textbox.
    width: Units,
    /// This element's height in pixels, used to explicitly set the size of the contained label or textbox.
    height: Units,
}

enum ParamLabelEvent {
    /// Text input has been cancelled without submitting a new value.
    CancelTextInput,
    /// A new value has been sent by the text input dialog after pressing Enter.
    TextInput(String),
}

impl ParamLabel {
    /// Creates a new [`ParamLabel`] for the given parameter. See
    /// [`ParamSlider`][super::ParamSlider] for more information on this function's arguments.
    ///
    /// To make this work, you'll need to set a fixed (non-auto) width and height on the
    /// `ParamLabel`. If the label is taller than it is wide, then the widget will be drawn
    /// vertically.
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
        // This is in essence a super stripped down version of `ParamSlider`
        Self {
            param_base: ParamWidgetBase::new(cx, params.clone(), params_to_param),

            text_input_active: false,

            // Automatically overridden in the `WindowEvent::GeometryChanged` handler
            orientation: Orientation::Horizontal,
            width: Pixels(0.0),
            height: Pixels(0.0),
        }
        .build(
            cx,
            ParamWidgetBase::view(params, params_to_param, move |cx, param_data| {
                let param_name = param_data.param().name().to_owned();

                // Can't use `.to_string()` here as that would include modulation
                let display_value_lens = param_data.make_lens(|param| {
                    param.normalized_value_to_string(param.unmodulated_normalized_value(), true)
                });

                Binding::new(
                    cx,
                    ParamLabel::text_input_active,
                    move |cx, text_input_active| {
                        if text_input_active.get(cx) {
                            Textbox::new(cx, display_value_lens.clone())
                                .class("value-entry")
                                .on_submit(|cx, string, success| {
                                    if success {
                                        cx.emit(ParamLabelEvent::TextInput(string))
                                    } else {
                                        cx.emit(ParamLabelEvent::CancelTextInput);
                                    }
                                })
                                .on_build(|cx| {
                                    cx.emit(TextEvent::StartEdit);
                                    cx.emit(TextEvent::SelectAll);
                                })
                                .class("align_center")
                                .child_space(Stretch(1.0))
                                .rotate(ParamLabel::orientation.map(orientation_to_rotation))
                                // HACK: Work around for vizia not supporting rotations
                                .height(ParamLabel::height)
                                .width(ParamLabel::width);
                        } else {
                            Label::new(cx, &param_name)
                                .class("param-name")
                                .child_space(Stretch(1.0))
                                .rotate(ParamLabel::orientation.map(orientation_to_rotation))
                                // Same as above
                                .height(ParamLabel::height)
                                .width(ParamLabel::width);
                        }
                    },
                );
            }),
        )
    }
}

impl View for ParamLabel {
    fn element(&self) -> Option<&'static str> {
        Some("param-label")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|param_slider_event, meta| match param_slider_event {
            ParamLabelEvent::CancelTextInput => {
                self.text_input_active = false;
                cx.set_active(false);

                meta.consume();
            }
            ParamLabelEvent::TextInput(string) => {
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
            WindowEvent::GeometryChanged(_) => {
                let width = cx.cache.get_width(cx.current());
                let height = cx.cache.get_height(cx.current());

                self.width = Pixels(width);
                self.height = Pixels(height);

                // The orientiation is automatically set based on the widget's aspect ratio
                if width >= height {
                    self.orientation = Orientation::Horizontal;
                } else {
                    self.orientation = Orientation::Vertical;
                }
            }

            // We don't handle Ctrl+click/double click for reset right now, only value entry is
            // supported here
            WindowEvent::MouseDown(MouseButton::Left)
            | WindowEvent::MouseDoubleClick(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                if cx.modifiers.alt() {
                    // ALt+Click brings up a text entry dialog
                    self.text_input_active = true;
                    cx.set_active(true);

                    meta.consume();
                }
            }
            _ => {}
        });
    }
}

/// Convert an [`Orientation`] to the desired rotation in degrees.
fn orientation_to_rotation(orientation: &Orientation) -> f32 {
    match orientation {
        Orientation::Horizontal => 0.0,
        // 90 degrees counterclickwise, so the text goes from bottom to top
        Orientation::Vertical => 270.0,
    }
}
