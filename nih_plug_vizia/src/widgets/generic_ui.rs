//! Generic UIs for NIH-plug using VIZIA.

use nih_plug::prelude::{ParamFlags, ParamPtr, Params};
use vizia::prelude::*;

use super::{ParamSlider, ParamSliderExt, ParamSliderStyle};

/// Shows a generic UI for a [`Params`] object. For additional flexibility you can either use the
/// [`new()`][`Self::new()`] method to have the generic UI decide which widget to use for your
/// parameters, or you can use the [`new_custom()`][`Self::new_custom()`] method to determine this
/// yourself.
pub struct GenericUi;

impl GenericUi {
    /// Creates a new [`GenericUi`] for all provided parameters. Use
    /// [`new_custom()`][Self::new_custom()] to decide which widget gets used for each parameter.
    ///
    /// Wrap this in a [`ScrollView`] for plugins with longer parameter lists:
    ///
    /// ```ignore
    /// ScrollView::new(cx, 0.0, 0.0, false, true, |cx| {
    ///     GenericUi::new(cx, Data::params);
    /// })
    /// .width(Percentage(100.0));
    ///```
    pub fn new<L, PsRef, Ps>(cx: &mut Context, params: L) -> Handle<'_, GenericUi>
    where
        L: Lens<Target = PsRef> + Clone,
        PsRef: AsRef<Ps> + 'static,
        Ps: Params + 'static,
    {
        // Basic styling is done in the `theme.css` style sheet
        Self::new_custom(cx, params, move |cx, param_ptr| {
            HStack::new(cx, |cx| {
                // Align this on the right
                Label::new(cx, unsafe { param_ptr.name() }).class("label");

                Self::draw_widget(cx, params, param_ptr);
            })
            .class("row");
        })
    }

    /// Creates a new [`GenericUi`] for all provided parameters using a custom closure that receives
    /// a function that should draw some widget for each parameter.
    pub fn new_custom<L, PsRef, Ps>(
        cx: &mut Context,
        params: L,
        mut make_widget: impl FnMut(&mut Context, ParamPtr),
    ) -> Handle<Self>
    where
        L: Lens<Target = PsRef>,
        PsRef: AsRef<Ps> + 'static,
        Ps: Params + 'static,
    {
        // Basic styling is done in the `theme.css` style sheet
        Self.build(cx, |cx| {
            // Rust does not have existential types, otherwise we could have passed functions that
            // map `params` to some `impl Param` and everything would have been a lot neater
            let param_map = params.map(|params| params.as_ref().param_map()).get(cx);
            for (_, param_ptr, _) in param_map {
                let flags = unsafe { param_ptr.flags() };
                if flags.contains(ParamFlags::HIDE_IN_GENERIC_UI) {
                    continue;
                }

                make_widget(cx, param_ptr);
            }
        })
    }

    /// The standard widget drawing function. This can be used together with `.new_custom()` to only
    /// draw the labels differently.
    pub fn draw_widget<L, PsRef, Ps>(cx: &mut Context, params: L, param_ptr: ParamPtr)
    where
        L: Lens<Target = PsRef>,
        PsRef: AsRef<Ps> + 'static,
        Ps: Params + 'static,
    {
        unsafe {
            match param_ptr {
                ParamPtr::FloatParam(p) => ParamSlider::new(cx, params, move |_| &*p),
                ParamPtr::IntParam(p) => ParamSlider::new(cx, params, move |_| &*p),
                ParamPtr::BoolParam(p) => ParamSlider::new(cx, params, move |_| &*p),
                ParamPtr::EnumParam(p) => ParamSlider::new(cx, params, move |_| &*p),
            }
        }
        .set_style(match unsafe { param_ptr.step_count() } {
            // This looks nice for boolean values, but it's too crowded for anything beyond
            // that without making the widget wider
            Some(step_count) if step_count <= 1 => {
                ParamSliderStyle::CurrentStepLabeled { even: true }
            }
            Some(step_count) if step_count <= 2 => ParamSliderStyle::CurrentStep { even: true },
            Some(_) => ParamSliderStyle::FromLeft,
            // This is already the default, but continuous parameters should be drawn from
            // the center if the default is also centered, or from the left if it is not
            None => ParamSliderStyle::Centered,
        })
        .class("widget");
    }
}

impl View for GenericUi {
    fn element(&self) -> Option<&'static str> {
        Some("generic-ui")
    }
}
