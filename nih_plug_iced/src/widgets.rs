//! Widgets and utilities for making widgets to integrate iced with NIH-plug.
//!
//! # Note
//!
//! None of these widgets are finalized, and their sizes or looks can change at any point. Feel free
//! to copy the widgets and modify them to your personal taste.

use nih_plug::prelude::ParamPtr;

pub mod generic_ui;
pub mod param_slider;
pub mod peak_meter;
pub mod util;

pub use param_slider::ParamSlider;
pub use peak_meter::PeakMeter;

/// A message to update a parameter value. Since NIH-plug manages the parameters, interacting with
/// parameter values with iced works a little different from updating any other state. This main
/// [`IcedEditor`][super::IcedEditor] should have a [`Message`][super::IcedEditor::Message] variant
/// containing this `ParamMessage`. When it receives one of those messages, it can pass it through
/// to [`self.handle_param_message()`][super::IcedEditor::handle_param_message].
#[derive(Debug, Clone, Copy)]
pub enum ParamMessage {
    /// Begin an automation gesture for a parameter.
    BeginSetParameter(ParamPtr),
    /// Set a parameter to a new normalized value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    SetParameterNormalized(ParamPtr, f32),
    /// End an automation gesture for a parameter.
    EndSetParameter(ParamPtr),
}
