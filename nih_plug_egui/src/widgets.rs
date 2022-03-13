//! Custom egui widgets for displaying parameter values.
//!
//! # Note
//!
//! None of these widgets are finalized, and their sizes or looks can change at any point. Feel free
//! to copy the widgets and modify them to your personal taste.

pub mod generic_ui;
mod param_slider;
pub mod util;

pub use param_slider::ParamSlider;
