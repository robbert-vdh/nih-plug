//! NIH-plug can handle floating point, integer, boolean, and enum parameters. Parameters are
//! managed by creating a struct deriving the [`Params`][internals::Params] trait containing fields
//! for those parameter types, and then returning a reference to that object from your
//! [`Plugin::params()`][crate::prelude::Plugin::params()] method. See the `Params` trait for more
//! information.

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
        /// The parameter cannot be changed from an automation lane. The parameter can however still
        /// be manually changed by the user from either the plugin's own GUI or from the host's
        /// generic UI.
        const NON_AUTOMATABLE = 1 << 1;
        /// Hides the parameter in the host's generic UI for this plugin. This also implies
        /// `NON_AUTOMATABLE`. Setting this does not prevent you from changing the parameter in the
        /// plugin's editor GUI.
        const HIDDEN = 1 << 2;
        /// Don't show this parameter when generating a generic UI for the plugin using one of
        /// NIH-plug's generic UI widgets.
        const HIDE_IN_GENERIC_UI = 1 << 3;
    }
}

/// Describes a single parameter of any type. Most parameter implementations also have a field
/// called `value` that and a field called `smoothed`. The former stores the latest unsmoothed
/// value, and the latter can be used to access the smoother. These two fields should be used in DSP
/// code to either get the parameter's current (smoothed) value. In UI code the getters from this
/// trait should be used instead.
pub trait Param: Display {
    /// The plain parameter type.
    type Plain: PartialEq;

    /// Get the human readable name for this parameter.
    fn name(&self) -> &str;

    /// Get the unit label for this parameter, if any.
    fn unit(&self) -> &'static str;

    /// Get this parameter's polyphonic modulation ID. If this is set for a parameter in a CLAP
    /// plugin, then polyphonic modulation will be enabled for that parameter. Polyphonic modulation
    /// is communicated to the plugin through
    /// [`NoteEvent::PolyModulation][crate::prelude::NoteEvent::PolyModulation`] and
    /// [`NoteEvent::MonoAutomation][crate::prelude::NoteEvent::MonoAutomation`] events. See the
    /// documentation on those events for more information.
    ///
    /// # Important
    ///
    /// After enabling polyphonic modulation, the plugin **must** start sending
    /// [`NoteEvent::VoiceTerminated`][crate::prelude::NoteEvent::VoiceEnd] events to the host when a voice
    /// has fully ended. This allows the host to reuse its modulation resources.
    fn poly_modulation_id(&self) -> Option<u32>;

    /// Get the unnormalized value for this parameter.
    fn plain_value(&self) -> Self::Plain;

    /// Get the normalized `[0, 1]` value for this parameter.
    fn normalized_value(&self) -> f32;

    /// Get the unnormalized value for this parameter before any (monophonic) modulation coming from
    /// the host has been applied. If the host is not currently modulating this parameter than this
    /// will be the same as [`plain_value()`][Self::plain_value()]. This may be useful for
    /// displaying modulation differently in plugin GUIs. Right now only CLAP plugins in Bitwig
    /// Studio use modulation.
    fn unmodulated_plain_value(&self) -> Self::Plain;

    /// Get the normalized `[0, 1]` value for this parameter before any (monophonic) modulation
    /// coming from the host has been applied. If the host is not currently modulating this
    /// parameter than this will be the same as [`plain_value()`][Self::plain_value()]. This may be
    /// useful for displaying modulation differently in plugin GUIs. Right now only CLAP plugins in
    /// Bitwig Studio use modulation.
    fn unmodulated_normalized_value(&self) -> f32;

    /// Get the unnormalized default value for this parameter.
    fn default_plain_value(&self) -> Self::Plain;

    /// Get the normalized `[0, 1]` default value for this parameter.
    #[inline]
    fn default_normalized_value(&self) -> f32 {
        self.preview_normalized(self.default_plain_value())
    }

    /// Get the number of steps for this parameter, if it is discrete. Used for the host's generic
    /// UI.
    fn step_count(&self) -> Option<usize>;

    /// Returns the previous step from a specific value for this parameter. This can be the same as
    /// `from` if the value is at the start of its range. This is mainly used for scroll wheel
    /// interaction in plugin GUIs. When the parameter is not discrete then a step should cover one
    /// hundredth of the normalized range instead.
    fn previous_step(&self, from: Self::Plain) -> Self::Plain;

    /// Returns the next step from a specific value for this parameter. This can be the same as
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

    /// Get the plain, unnormalized value for this parameter after polyphonic modulation has been
    /// applied. This is a convenience method for calling [`preview_plain()`][Self::preview_plain()]
    /// with `unmodulated_normalized_value() + normalized_offset`.`
    #[inline]
    fn preview_modulated(&self, normalized_offset: f32) -> Self::Plain {
        self.preview_plain(self.unmodulated_normalized_value() + normalized_offset)
    }

    /// Flags to control the parameter's behavior. See [`ParamFlags`].
    fn flags(&self) -> ParamFlags;

    /// Internal implementation detail for implementing [`Params`][internals::Params]. This should
    /// not be used directly.
    fn as_ptr(&self) -> internals::ParamPtr;
}

/// Contains the setters for parameters. These should not be exposed to plugins to avoid confusion.
pub(crate) trait ParamMut: Param {
    /// Set this parameter based on a plain, unnormalized value. This does not snap to step sizes
    /// for continuous parameters (i.e. [`FloatParam`]). If
    /// [`modulate_value()`][Self::modulate_value()] has previously been called with a non zero
    /// value then this offset is taken into account to form the effective value.
    ///
    /// This does **not** update the smoother.
    fn set_plain_value(&self, plain: Self::Plain);

    /// Set this parameter based on a normalized value. The normalized value will be snapped to the
    /// step size for continuous parameters (i.e. [`FloatParam`]). If
    /// [`modulate_value()`][Self::modulate_value()] has previously been called with a non zero
    /// value then this offset is taken into account to form the effective value.
    ///
    /// This does **not** update the smoother.
    fn set_normalized_value(&self, normalized: f32);

    /// Add a modulation offset to the value's unmodulated value. This value sticks until this
    /// function is called again with a 0.0 value. Out of bound values will be clamped to the
    /// parameter's range. The normalized value will be snapped to the step size for continuous
    /// parameters (i.e. [`FloatParam`]).
    ///
    /// This does **not** update the smoother.
    fn modulate_value(&self, modulation_offset: f32);

    /// Update the smoother state to point to the current value. Also used when initializing and
    /// restoring a plugin so everything is in sync. In that case the smoother should completely
    /// reset to the current value.
    fn update_smoother(&self, sample_rate: f32, reset: bool);
}
