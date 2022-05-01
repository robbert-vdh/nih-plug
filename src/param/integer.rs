//! Stepped integer parameters.

use std::fmt::Display;
use std::sync::Arc;

use super::internals::ParamPtr;
use super::range::IntRange;
use super::smoothing::{Smoother, SmoothingStyle};
use super::{Param, ParamFlags};

/// A discrete integer parameter that's stored unnormalized. The range is used for the normalization
/// process.
//
// XXX: To keep the API simple and to allow the optimizer to do its thing, the values are stored as
//      plain primitive values that are modified through the `*mut` pointers from the plugin's
//      `Params` object. Technically modifying these while the GUI is open is unsound. We could
//      remedy this by changing `value` to be an atomic type and adding a function also called
//      `value()` to load that value, but in practice that should not be necessary if we don't do
//      anything crazy other than modifying this value. On both AArch64 and x86(_64) reads and
//      writes to naturally aligned values up to word size are atomic, so there's no risk of reading
//      a partially written to value here. We should probably reconsider this at some point though.
#[repr(C, align(4))]
pub struct IntParam {
    /// The field's current plain, unnormalized value.
    pub value: i32,
    /// The field's current value normalized to the `[0, 1]` range.
    normalized_value: f32,
    /// The field's default plain, unnormalized value.
    default: i32,
    /// An optional smoother that will automatically interpolate between the new automation values
    /// set by the host.
    pub smoothed: Smoother<i32>,

    /// Flags to control the parameter's behavior. See [`ParamFlags`].
    flags: ParamFlags,
    /// Optional callback for listening to value changes. The argument passed to this function is
    /// the parameter's new **plain** value. This should not do anything expensive as it may be
    /// called multiple times in rapid succession.
    ///
    /// To use this, you'll probably want to store an `Arc<Atomic*>` alongside the parmater in the
    /// parameters struct, move a clone of that `Arc` into this closure, and then modify that.
    ///
    /// TODO: We probably also want to pass the old value to this function.
    value_changed: Option<Arc<dyn Fn(i32) + Send + Sync>>,

    /// The distribution of the parameter's values.
    range: IntRange,
    /// The parameter's human readable display name.
    name: String,
    /// The parameter value's unit, added after `value_to_string` if that is set. NIH-plug will not
    /// automatically add a space before the unit.
    unit: &'static str,
    /// Optional custom conversion function from a plain **unnormalized** value to a string.
    value_to_string: Option<Arc<dyn Fn(i32) -> String + Send + Sync>>,
    /// Optional custom conversion function from a string to a plain **unnormalized** value. If the
    /// string cannot be parsed, then this should return a `None`. If this happens while the
    /// parameter is being updated then the update will be canceled.
    ///
    /// The input string may or may not contain the unit, so you will need to be able to handle
    /// that.
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>>,
}

impl Default for IntParam {
    fn default() -> Self {
        Self {
            value: 0,
            normalized_value: 0.0,
            default: 0,
            smoothed: Smoother::none(),
            flags: ParamFlags::default(),
            value_changed: None,
            range: IntRange::default(),
            name: String::new(),
            unit: "",
            value_to_string: None,
            string_to_value: None,
        }
    }
}

impl Display for IntParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.value_to_string {
            Some(func) => write!(f, "{}{}", func(self.value), self.unit),
            _ => write!(f, "{}{}", self.value, self.unit),
        }
    }
}

impl Param for IntParam {
    type Plain = i32;

    fn name(&self) -> &str {
        &self.name
    }

    fn unit(&self) -> &'static str {
        self.unit
    }

    #[inline]
    fn plain_value(&self) -> Self::Plain {
        self.value
    }

    #[inline]
    fn normalized_value(&self) -> f32 {
        self.normalized_value
    }

    #[inline]
    fn default_plain_value(&self) -> Self::Plain {
        self.default
    }

    fn step_count(&self) -> Option<usize> {
        Some(self.range.step_count())
    }

    fn previous_step(&self, from: Self::Plain) -> Self::Plain {
        (from - 1).clamp(self.range.min(), self.range.max())
    }

    fn next_step(&self, from: Self::Plain) -> Self::Plain {
        (from + 1).clamp(self.range.min(), self.range.max())
    }

    fn set_plain_value(&mut self, plain: Self::Plain) {
        self.value = plain;
        self.normalized_value = self.preview_normalized(plain);
        if let Some(f) = &self.value_changed {
            f(self.value);
        }
    }

    fn set_normalized_value(&mut self, normalized: f32) {
        self.value = self.preview_plain(normalized);
        self.normalized_value = normalized;
        if let Some(f) = &self.value_changed {
            f(self.value);
        }
    }

    fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String {
        let value = self.preview_plain(normalized);
        match (&self.value_to_string, include_unit) {
            (Some(f), true) => format!("{}{}", f(value), self.unit),
            (Some(f), false) => f(value),
            (None, true) => format!("{}{}", value, self.unit),
            (None, false) => format!("{}", value),
        }
    }

    fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
        let value = match &self.string_to_value {
            Some(f) => f(string.trim()),
            // In the CLAP wrapper the unit will be included, so make sure to handle that
            None => string.trim().trim_end_matches(self.unit).parse().ok(),
        }?;

        Some(self.preview_normalized(value))
    }

    fn preview_normalized(&self, plain: Self::Plain) -> f32 {
        self.range.normalize(plain)
    }

    fn preview_plain(&self, normalized: f32) -> Self::Plain {
        self.range.unnormalize(normalized)
    }

    fn update_smoother(&mut self, sample_rate: f32, reset: bool) {
        if reset {
            self.smoothed.reset(self.value);
        } else {
            self.smoothed.set_target(sample_rate, self.value);
        }
    }

    fn initialize_block_smoother(&mut self, max_block_size: usize) {
        self.smoothed.initialize_block_smoother(max_block_size);
    }

    fn flags(&self) -> ParamFlags {
        self.flags
    }

    fn as_ptr(&self) -> ParamPtr {
        ParamPtr::IntParam(self as *const _ as *mut _)
    }
}

impl IntParam {
    /// Build a new [`IntParam`]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: impl Into<String>, default: i32, range: IntRange) -> Self {
        Self {
            value: default,
            normalized_value: range.normalize(default),
            default,
            range,
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set up a smoother that can gradually interpolate changes made to this parameter, preventing
    /// clicks and zipper noises.
    pub fn with_smoother(mut self, style: SmoothingStyle) -> Self {
        // Logarithmic smoothing will cause problems if the range goes through zero since then you
        // end up multplying by zero
        let goes_through_zero = match (&style, &self.range) {
            (SmoothingStyle::Logarithmic(_), IntRange::Linear { min, max }) => {
                *min == 0 || *max == 0 || min.signum() != max.signum()
            }
            _ => false,
        };
        nih_debug_assert!(
            !goes_through_zero,
            "Logarithmic smoothing does not work with ranges that go through zero"
        );

        self.smoothed = Smoother::new(style);
        self
    }

    /// Run a callback whenever this parameter's value changes. The argument passed to this function
    /// is the parameter's new value. This should not do anything expensive as it may be called
    /// multiple times in rapid succession, and it can be run from both the GUI and the audio
    /// thread.
    pub fn with_callback(mut self, callback: Arc<dyn Fn(i32) + Send + Sync>) -> Self {
        self.value_changed = Some(callback);
        self
    }

    /// Display a unit when rendering this parameter to a string. Appended after the
    /// [`value_to_string`][Self::value_to_string] function if that is also set. NIH-plug will not
    /// automatically add a space before the unit.
    pub fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    /// Use a custom conversion function to convert the plain, unnormalized value to a
    /// string.
    pub fn with_value_to_string(
        mut self,
        callback: Arc<dyn Fn(i32) -> String + Send + Sync>,
    ) -> Self {
        self.value_to_string = Some(callback);
        self
    }

    // `with_step_size` is only implemented for the f32 version

    /// Use a custom conversion function to convert from a string to a plain, unnormalized
    /// value. If the string cannot be parsed, then this should return a `None`. If this
    /// happens while the parameter is being updated then the update will be canceled.
    ///
    /// The input string may or may not contain the unit, so you will need to be able to handle
    /// that.
    pub fn with_string_to_value(
        mut self,
        callback: Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>,
    ) -> Self {
        self.string_to_value = Some(callback);
        self
    }

    /// Mark the paramter as non-automatable. This means that the parameter cannot be automated from
    /// the host. Setting this flag also prevents it from showing up in the host's own generic UI
    /// for this plugin. The parameter can still be changed from the plugin's editor GUI.
    pub fn non_automatable(mut self) -> Self {
        self.flags.insert(ParamFlags::NON_AUTOMATABLE);
        self
    }

    /// Don't show this parameter when generating a generic UI for the plugin using one of
    /// NIH-plug's generic UI widgets.
    pub fn hide_in_generic_ui(mut self) -> Self {
        self.flags.insert(ParamFlags::HIDE_IN_GENERIC_UI);
        self
    }
}
