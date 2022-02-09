//! Custom egui widgets for displaying parameter values.
//!
//! # Note
//!
//! None of these widgets are finalized, and their sizes or looks can change at any point. Feel free
//! to copy the widgets and modify them to your personal taste.

mod param_slider;
pub mod util;

pub use param_slider::ParamSlider;

// TODO: At some opint add some generic UI widget that shows an entire Params struct (in order)
//       along with the parameter's names as sliders
