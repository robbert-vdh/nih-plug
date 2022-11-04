//! A toggleable button that integrates with NIH-plug's [`Param`] types.

use nih_plug::prelude::{Param, ParamPtr};
use vizia::prelude::*;

use super::RawParamEvent;

/// A toggleable button that integrates with NIH-plug's [`Param`] types. Only makes sense with
/// [`BoolParam`][nih_plug::prelude::BoolParam]s. Clicking on the button will toggle between the
/// parameter's minimum and maximum value. The `:checked` pseudoclass indicates whether or not the
/// button is currently pressed.
#[derive(Lens)]
pub struct ParamButton {
    // We're not allowed to store a reference to the parameter internally, at least not in the
    // struct that implements [`View`]
    param_ptr: ParamPtr,
}

impl ParamButton {
    /// Creates a new [`ParamButton`] for the given parameter. To accommodate VIZIA's mapping system,
    /// you'll need to provide a lens containing your `Params` implementation object (check out how
    /// the `Data` struct is used in `gain_gui_vizia`) and a projection function that maps the
    /// `Params` object to the parameter you want to display a widget for. Parameter changes are
    /// handled by emitting [`ParamEvent`][super::ParamEvent]s which are automatically handled by
    /// the VIZIA wrapper.
    pub fn new<L, Params, P, F>(cx: &mut Context, params: L, params_to_param: F) -> Handle<Self>
    where
        L: Lens<Target = Params> + Clone,
        F: 'static + Fn(&Params) -> &P + Copy,
        Params: 'static,
        P: Param,
    {
        let param_ptr = params
            .clone()
            .map(move |params| params_to_param(params).as_ptr())
            .get(cx);
        let param_name = params
            .clone()
            .map(move |params| params_to_param(params).name().to_owned())
            .get(cx);

        // We'll add the `:checked` pseudoclass when the button is pressed
        // NOTE: We use the normalized value _with modulation_ for this. There's no convenient way
        //       to show both modulated and unmodulated values here.
        let param_value_lens = params.map(move |params| params_to_param(params).normalized_value());

        Self { param_ptr }
            .build(cx, move |cx| {
                Label::new(cx, &param_name);
            })
            .checked(param_value_lens.map(|v| v >= &0.5))
    }

    /// Set the parameter's normalized value to either 0.0 or 1.0 depending on its current value.
    fn toggle_value(&self, cx: &mut EventContext) {
        let current_value = unsafe { self.param_ptr.unmodulated_normalized_value() };
        let new_value = if current_value >= 0.5 { 0.0 } else { 1.0 };

        cx.emit(RawParamEvent::BeginSetParameter(self.param_ptr));
        cx.emit(RawParamEvent::SetParameterNormalized(
            self.param_ptr,
            new_value,
        ));
        cx.emit(RawParamEvent::EndSetParameter(self.param_ptr));
    }
}

impl View for ParamButton {
    fn element(&self) -> Option<&'static str> {
        Some("param-button")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|window_event, meta| match window_event {
            WindowEvent::MouseDown(MouseButton::Left)
            // We don't need special double and triple click handling
            | WindowEvent::MouseDoubleClick(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                self.toggle_value(cx);
                meta.consume();
            }
            _ => {}
        });
    }
}

/// Extension methods for [`ParamButton`] handles.
pub trait ParamButtonExt {
    /// Change the colors scheme for a bypass button. This simply adds the `bypass` class.
    fn for_bypass(self) -> Self;
}

impl ParamButtonExt for Handle<'_, ParamButton> {
    fn for_bypass(self) -> Self {
        self.class("bypass")
    }
}
