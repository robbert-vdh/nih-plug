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
    /// `ParamLabel`.
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
                                .child_top(Stretch(1.0))
                                .child_bottom(Stretch(1.0))
                                .height(Stretch(1.0))
                                .width(Stretch(1.0));
                        } else {
                            Label::new(cx, &param_name)
                                .class("param-name")
                                .child_space(Stretch(1.0))
                                .height(Stretch(1.0))
                                .width(Stretch(1.0));
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
