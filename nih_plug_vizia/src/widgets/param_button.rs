//! A toggleable button that integrates with NIH-plug's [`Param`] types.

use nih_plug::prelude::Param;
use vizia::prelude::*;

use super::param_base::ParamWidgetBase;

/// A toggleable button that integrates with NIH-plug's [`Param`] types. Only makes sense with
/// [`BoolParam`][nih_plug::prelude::BoolParam]s. Clicking on the button will toggle between the
/// parameter's minimum and maximum value. The `:checked` pseudoclass indicates whether or not the
/// button is currently pressed.
#[derive(Lens)]
pub struct ParamButton {
    param_base: ParamWidgetBase,

    // These fields are set through modifiers:
    /// Whether or not to listen to scroll events for changing the parameter's value in steps.
    use_scroll_wheel: bool,
    /// A specific label to use instead of displaying the parameter's value.
    label_override: Option<String>,

    /// The number of (fractional) scrolled lines that have not yet been turned into parameter
    /// change events. This is needed to support trackpads with smooth scrolling.
    scrolled_lines: f32,
}

impl ParamButton {
    /// Creates a new [`ParamButton`] for the given parameter. See
    /// [`ParamSlider`][super::ParamSlider] for more information on this function's arguments.
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
        Self {
            param_base: ParamWidgetBase::new(cx, params, params_to_param),

            use_scroll_wheel: true,
            label_override: None,

            scrolled_lines: 0.0,
        }
        .build(
            cx,
            ParamWidgetBase::build_view(params, params_to_param, move |cx, param_data| {
                Binding::new(cx, Self::label_override, move |cx, label_override| {
                    match label_override.get(cx) {
                        Some(label_override) => Label::new(cx, &label_override),
                        None => Label::new(cx, param_data.param().name()),
                    }
                    .hoverable(false);
                })
            }),
        )
        // We'll add the `:checked` pseudoclass when the button is pressed
        // NOTE: We use the normalized value _with modulation_ for this. There's no convenient way
        //       to show both modulated and unmodulated values here.
        .checked(ParamWidgetBase::make_lens(
            params,
            params_to_param,
            |param| param.modulated_normalized_value() >= 0.5,
        ))
    }

    /// Set the parameter's normalized value to either 0.0 or 1.0 depending on its current value.
    fn toggle_value(&self, cx: &mut EventContext) {
        let current_value = self.param_base.unmodulated_normalized_value();
        let new_value = if current_value >= 0.5 { 0.0 } else { 1.0 };

        self.param_base.begin_set_parameter(cx);
        self.param_base.set_normalized_value(cx, new_value);
        self.param_base.end_set_parameter(cx);
    }
}

impl View for ParamButton {
    fn element(&self) -> Option<&'static str> {
        Some("param-button")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|window_event, meta| match window_event {
            // We don't need special double and triple click handling
            WindowEvent::MouseDown(MouseButton::Left)
            | WindowEvent::MouseDoubleClick(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                self.toggle_value(cx);
                meta.consume();
            }
            WindowEvent::MouseScroll(_scroll_x, scroll_y) if self.use_scroll_wheel => {
                // With a regular scroll wheel `scroll_y` will only ever be -1 or 1, but with smooth
                // scrolling trackpads being a thing `scroll_y` could be anything.
                self.scrolled_lines += scroll_y;

                if self.scrolled_lines.abs() >= 1.0 {
                    self.param_base.begin_set_parameter(cx);

                    if self.scrolled_lines >= 1.0 {
                        self.param_base.set_normalized_value(cx, 1.0);
                        self.scrolled_lines -= 1.0;
                    } else {
                        self.param_base.set_normalized_value(cx, 0.0);
                        self.scrolled_lines += 1.0;
                    }

                    self.param_base.end_set_parameter(cx);
                }

                meta.consume();
            }
            _ => {}
        });
    }
}

/// Extension methods for [`ParamButton`] handles.
pub trait ParamButtonExt {
    /// Don't respond to scroll wheel events. Useful when this button is used as part of a scrolling
    /// view.
    fn disable_scroll_wheel(self) -> Self;

    /// Change the colors scheme for a bypass button. This simply adds the `bypass` class.
    fn for_bypass(self) -> Self;

    /// Change the label used for the button. If this is not set, then the parameter's name will be
    /// used.
    fn with_label(self, value: impl Into<String>) -> Self;
}

impl ParamButtonExt for Handle<'_, ParamButton> {
    fn disable_scroll_wheel(self) -> Self {
        self.modify(|param_slider: &mut ParamButton| param_slider.use_scroll_wheel = false)
    }

    fn for_bypass(self) -> Self {
        self.class("bypass")
    }

    fn with_label(self, value: impl Into<String>) -> Self {
        self.modify(|param_button: &mut ParamButton| {
            param_button.label_override = Some(value.into())
        })
    }
}
