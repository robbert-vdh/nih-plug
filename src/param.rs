//! TODO: Document how to use the [Param] trait. Also mention both interfaces: direct initialization
//!       + `..Default::default()`, and the builder interface. For the moment, just look at the gain
//!       example.

use std::fmt::Display;

// Parameter types
mod boolean;
pub mod enums;
mod float;
mod integer;

pub mod internals;
pub mod range;
pub mod smoothing;

pub use boolean::BoolParam;
pub use enums::EnumParam;
pub use float::FloatParam;
pub use integer::IntParam;

/// Describes a single parameter of any type.
pub trait Param: Display {
    /// The plain parameter type.
    type Plain;

    /// Get the human readable name for this parameter.
    fn name(&self) -> &'static str;

    /// Get the unit label for this parameter, if any.
    fn unit(&self) -> &'static str;

    /// Get the number of steps for this paramter, if it is stepped. Used for the host's generic UI.
    fn step_count(&self) -> Option<usize>;

    /// Get the unnormalized value for this parameter.
    fn plain_value(&self) -> Self::Plain;

    /// Set this parameter based on a plain, unnormalized value. This does **not** snap to step
    /// sizes for continuous parameters (i.e. [FloatParam]).
    ///
    /// This does **not** update the smoother.
    fn set_plain_value(&mut self, plain: Self::Plain);

    /// Get the normalized `[0, 1]` value for this parameter.
    fn normalized_value(&self) -> f32;

    /// Set this parameter based on a normalized value. This **does** snap to step sizes for
    /// continuous parameters (i.e. [FloatParam]).
    ///
    /// This does **not** update the smoother.
    fn set_normalized_value(&mut self, normalized: f32);

    /// Get the string representation for a normalized value. Used as part of the wrappers. Most
    /// plugin formats already have support for units, in which case it shouldn't be part of this
    /// string or some DAWs may show duplicate units.
    fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String;

    /// Get the string representation for a normalized value. Used as part of the wrappers.
    fn string_to_normalized_value(&self, string: &str) -> Option<f32>;

    /// Get the normalized value for a plain, unnormalized value, as a float. Used as part of the
    /// wrappers.
    fn preview_normalized(&self, plain: Self::Plain) -> f32;

    /// Get the plain, unnormalized value for a normalized value, as a float. Used as part of the
    /// wrappers. This **does** snap to step sizes for continuous parameters (i.e. [FloatParam]).
    fn preview_plain(&self, normalized: f32) -> Self::Plain;

    /// Set this parameter based on a string. Returns whether the updating succeeded. That can fail
    /// if the string cannot be parsed.
    ///
    /// TODO: After implementing VST3, check if we handle parsing failures correctly
    fn set_from_string(&mut self, string: &str) -> bool;

    /// Update the smoother state to point to the current value. Also used when initializing and
    /// restoring a plugin so everything is in sync. In that case the smoother should completely
    /// reset to the current value.
    fn update_smoother(&mut self, sample_rate: f32, reset: bool);

    /// Allocate memory for block-based smoothing. The [crate::Plugin::initialize_block_smoothers()]
    /// method will do this for every smoother.
    fn initialize_block_smoother(&mut self, max_block_size: usize);

    /// Internal implementation detail for implementing [internals::Params]. This should not be used
    /// directly.
    fn as_ptr(&self) -> internals::ParamPtr;
}
