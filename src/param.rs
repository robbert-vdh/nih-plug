//! TODO: Document how to use the [`Param`] trait. For now, just look at the gain example.

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

bitflags::bitflags! {
    /// Flags for controlling a parameter's behavior.
    #[repr(transparent)]
    #[derive(Default)]
    pub struct ParamFlags: u32 {
        /// When applied to a [`BoolParam`], this will cause the parameter to be linked to the
        /// host's bypass control. Only a single parameter can be marked as a bypass parameter. If
        /// you don't have a bypass parameter, then NIH-plug will add one for you. You will need to
        /// implement this yourself if your plugin introduces latency.
        const BYPASS = 1 << 0;
        /// The parameter cannot be automated from the host. Setting this flag also prevents it from
        /// showing up in the host's own generic UI for this plugin. The parameter can still be
        /// changed from the plugin's editor GUI.
        const NON_AUTOMATABLE = 1 << 1;
        /// Don't show this parameter when generating a generic UI for the plugin using one of
        /// NIH-plug's generic UI widgets.
        const HIDE_IN_GENERIC_UI = 1 << 2;
    }
}

/// Describes a single parameter of any type.
pub trait Param: Display {
    /// The plain parameter type.
    type Plain: PartialEq;

    /// Get the human readable name for this parameter.
    fn name(&self) -> &'static str;

    /// Get the unit label for this parameter, if any.
    fn unit(&self) -> &'static str;

    /// Get the unnormalized value for this parameter.
    fn plain_value(&self) -> Self::Plain;

    /// Get the normalized `[0, 1]` value for this parameter.
    fn normalized_value(&self) -> f32 {
        self.preview_normalized(self.plain_value())
    }

    /// Get the unnormalized default value for this parameter.
    fn default_plain_value(&self) -> Self::Plain;

    /// Get the normalized `[0, 1]` default value for this parameter.
    fn default_normalized_value(&self) -> f32 {
        self.preview_normalized(self.default_plain_value())
    }

    /// Get the number of steps for this paramter, if it is discrete. Used for the host's generic
    /// UI.
    fn step_count(&self) -> Option<usize>;

    /// Return the previous step from a specific value for this parameter. This can be the same as
    /// `from` if the value is at the start of its range. This is mainly used for scroll wheel
    /// interaction in plugin GUIs. When the parameter is not discrete then a step should cover one
    /// hundredth of the normalized range instead.
    fn previous_step(&self, from: Self::Plain) -> Self::Plain;

    /// Return the next step from a specific value for this parameter. This can be the same as
    /// `from` if the value is at the end of its range. This is mainly used for scroll wheel
    /// interaction in plugin GUIs. When the parameter is not discrete then a step should cover one
    /// hundredth of the normalized range instead.
    fn next_step(&self, from: Self::Plain) -> Self::Plain;

    /// The same as [`previous_step()`][Self::previous_step()], but for normalized values. This is
    /// mostly useful for GUI widgets.
    fn previous_normalized_step(&self, from: f32) -> f32 {
        self.preview_normalized(self.previous_step(self.preview_plain(from)))
    }

    /// The same as [`next_step()`][Self::next_step()], but for normalized values. This is mostly
    /// useful for GUI widgets.
    fn next_normalized_step(&self, from: f32) -> f32 {
        self.preview_normalized(self.next_step(self.preview_plain(from)))
    }

    /// Set this parameter based on a plain, unnormalized value. This does **not** snap to step
    /// sizes for continuous parameters (i.e. [`FloatParam`]).
    ///
    /// This does **not** update the smoother.
    fn set_plain_value(&mut self, plain: Self::Plain);

    /// Set this parameter based on a normalized value. This **does** snap to step sizes for
    /// continuous parameters (i.e. [`FloatParam`]).
    ///
    /// This does **not** update the smoother.
    fn set_normalized_value(&mut self, normalized: f32) {
        self.set_plain_value(self.preview_plain(normalized))
    }

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
    /// wrappers. This **does** snap to step sizes for continuous parameters (i.e. [`FloatParam`]).
    fn preview_plain(&self, normalized: f32) -> Self::Plain;

    /// Update the smoother state to point to the current value. Also used when initializing and
    /// restoring a plugin so everything is in sync. In that case the smoother should completely
    /// reset to the current value.
    fn update_smoother(&mut self, sample_rate: f32, reset: bool);

    /// Allocate memory for block-based smoothing. The
    /// [`Plugin::initialize_block_smoothers()`][crate::prelude::Plugin::initialize_block_smoothers()] method
    /// will do this for every smoother.
    fn initialize_block_smoother(&mut self, max_block_size: usize);

    /// Flags to control the parameter's behavior. See [`ParamFlags`].
    fn flags(&self) -> ParamFlags;

    /// Internal implementation detail for implementing [`Params`][internals::Params]. This should
    /// not be used directly.
    fn as_ptr(&self) -> internals::ParamPtr;
}
