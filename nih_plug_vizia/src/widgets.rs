//! Widgets and utilities for making widgets to integrate VIZIA with NIH-plug.
//!
//! # Note
//!
//! None of these widgets are finalized, and their sizes or looks can change at any point. Feel free
//! to copy the widgets and modify them to your personal taste.

use nih_plug::param::internals::ParamPtr;
use nih_plug::prelude::{GuiContext, Param};
use std::sync::Arc;

use vizia::{Context, Model};

mod param_slider;
pub mod util;

pub use param_slider::{ParamSlider, ParamSliderExt, ParamSliderStyle};

/// Register the default theme for the widgets exported by this module. This is automatically called
/// for you when using [`create_vizia_editor()`][super::create_vizia_editor()].
pub fn register_theme(cx: &mut Context) {
    cx.add_theme(include_str!("../assets/theme.css"));
}

/// An event that updates a parameter's value. Since NIH-plug manages the parameters, interacting
/// with parameter values with VIZIA works a little different from updating any other state. These
/// events are automatically handled by `nih_plug_vizia`.
///
/// Call the [`upcast()`][Self::upcast()] method to be able to emit this event through a
/// [`Context`][vizia::Context].
#[derive(Debug, Clone, Copy)]
pub enum ParamEvent<'a, P: Param> {
    /// Begin an automation gesture for a parameter.
    BeginSetParameter(&'a P),
    /// Set a parameter to a new normalized value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    SetParameter(&'a P, P::Plain),
    /// Set a parameter to a new normalized value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    SetParameterNormalized(&'a P, f32),
    /// Reset a parameter to its default value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    ResetParameter(&'a P),
    /// End an automation gesture for a parameter.
    EndSetParameter(&'a P),
}

/// The same as [`ParamEvent`], but type erased.
#[derive(Debug, Clone, Copy)]
pub enum RawParamEvent {
    /// Begin an automation gesture for a parameter.
    BeginSetParameter(ParamPtr),
    /// Set a parameter to a new normalized value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    SetParameterNormalized(ParamPtr, f32),
    /// Reset a parameter to its default value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    ResetParameter(ParamPtr),
    /// End an automation gesture for a parameter.
    EndSetParameter(ParamPtr),
}

/// Handles parameter updates for VIZIA GUIs. Registered in
/// [`ViziaEditor::spawn()`][super::ViziaEditor::spawn()].
pub(crate) struct ParamModel {
    pub context: Arc<dyn GuiContext>,
}

impl Model for ParamModel {
    fn event(&mut self, _cx: &mut vizia::Context, event: &mut vizia::Event) {
        if let Some(param_event) = event.message.downcast() {
            // `ParamEvent` gets downcast into `NormalizedParamEvent` by the `Message`
            // implementation below
            match *param_event {
                RawParamEvent::BeginSetParameter(p) => unsafe {
                    self.context.raw_begin_set_parameter(p)
                },
                RawParamEvent::SetParameterNormalized(p, v) => unsafe {
                    self.context.raw_set_parameter_normalized(p, v)
                },
                RawParamEvent::ResetParameter(p) => unsafe {
                    let default_value = self.context.raw_default_normalized_param_value(p);
                    self.context.raw_set_parameter_normalized(p, default_value);
                },
                RawParamEvent::EndSetParameter(p) => unsafe {
                    self.context.raw_end_set_parameter(p)
                },
            }
        }
    }
}

impl<P: Param> From<ParamEvent<'_, P>> for RawParamEvent {
    fn from(event: ParamEvent<'_, P>) -> Self {
        match event {
            ParamEvent::BeginSetParameter(p) => RawParamEvent::BeginSetParameter(p.as_ptr()),
            ParamEvent::SetParameter(p, v) => {
                RawParamEvent::SetParameterNormalized(p.as_ptr(), p.preview_normalized(v))
            }
            ParamEvent::SetParameterNormalized(p, v) => {
                RawParamEvent::SetParameterNormalized(p.as_ptr(), v)
            }
            ParamEvent::ResetParameter(p) => RawParamEvent::ResetParameter(p.as_ptr()),
            ParamEvent::EndSetParameter(p) => RawParamEvent::EndSetParameter(p.as_ptr()),
        }
    }
}

impl<P: Param> ParamEvent<'_, P> {
    /// Convert this event into a type erased version of itself that can be emitted through
    /// [`Context::emit()`][vizia::Context::emit()].
    ///
    /// TODO: Think of a better, clearer term for this
    pub fn upcast(self) -> RawParamEvent {
        self.into()
    }
}
