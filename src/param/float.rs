//! Continuous (or discrete, with a step size) floating point parameters.

use std::fmt::Display;
use std::sync::Arc;

use super::internals::ParamPtr;
use super::range::FloatRange;
use super::smoothing::{Smoother, SmoothingStyle};
use super::Param;

/// A floating point parameter that's stored unnormalized. The range is used for the normalization
/// process.
///
/// You can either initialize the struct directly, using `..Default::default()` to fill in the
/// unused fields, or you can use the builder interface with [`FloatParam::new()`].
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
pub struct FloatParam {
    /// The field's current plain, unnormalized value. Should be initialized with the default value.
    /// Storing parameter values like this instead of in a single contiguous array is bad for cache
    /// locality, but it does allow for a much nicer declarative API.
    pub value: f32,
    /// An optional smoother that will automatically interpolate between the new automation values
    /// set by the host.
    pub smoothed: Smoother<f32>,
    /// Optional callback for listening to value changes. The argument passed to this function is
    /// the parameter's new **plain** value. This should not do anything expensive as it may be
    /// called multiple times in rapid succession.
    ///
    /// To use this, you'll probably want to store an `Arc<Atomic*>` alongside the parmater in the
    /// parmaeters struct, move a clone of that `Arc` into this closure, and then modify that.
    ///
    /// TODO: We probably also want to pass the old value to this function.
    pub value_changed: Option<Arc<dyn Fn(f32) + Send + Sync>>,

    /// The distribution of the parameter's values.
    pub range: FloatRange,
    /// The distance between discrete steps in this parameter. Mostly useful for quantizing GUI
    /// input. If this is set and if [`value_to_string`][Self::value_to_string] is not set, then
    /// this is also used when formatting the parameter. This must be a positive, nonzero number.
    pub step_size: Option<f32>,
    /// The parameter's human readable display name.
    pub name: &'static str,
    /// The parameter value's unit, added after [`value_to_string`][Self::value_to_string] if that
    /// is set. NIH-plug will not automatically add a space before the unit.
    pub unit: &'static str,
    /// Optional custom conversion function from a plain **unnormalized** value to a string.
    pub value_to_string: Option<Arc<dyn Fn(f32) -> String + Send + Sync>>,
    /// Optional custom conversion function from a string to a plain **unnormalized** value. If the
    /// string cannot be parsed, then this should return a `None`. If this happens while the
    /// parameter is being updated then the update will be canceled.
    ///
    /// The input string may or may not contain the unit, so you will need to be able to handle
    /// that.
    pub string_to_value: Option<Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>>,
}

impl Default for FloatParam {
    fn default() -> Self {
        Self {
            value: 0.0,
            smoothed: Smoother::none(),
            value_changed: None,
            range: FloatRange::default(),
            step_size: None,
            name: "",
            unit: "",
            value_to_string: None,
            string_to_value: None,
        }
    }
}

impl Display for FloatParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.value_to_string, &self.step_size) {
            (Some(func), _) => write!(f, "{}{}", func(self.value), self.unit),
            (None, Some(step_size)) => {
                let num_digits = decimals_from_step_size(*step_size);
                write!(f, "{:.num_digits$}{}", self.value, self.unit)
            }
            _ => write!(f, "{}{}", self.value, self.unit),
        }
    }
}

impl Param for FloatParam {
    type Plain = f32;

    fn name(&self) -> &'static str {
        self.name
    }

    fn unit(&self) -> &'static str {
        self.unit
    }

    fn step_count(&self) -> Option<usize> {
        None
    }

    fn plain_value(&self) -> Self::Plain {
        self.value
    }

    fn set_plain_value(&mut self, plain: Self::Plain) {
        self.value = plain;
        if let Some(f) = &self.value_changed {
            f(plain);
        }
    }

    fn normalized_value(&self) -> f32 {
        self.preview_normalized(self.value)
    }

    fn set_normalized_value(&mut self, normalized: f32) {
        self.set_plain_value(self.preview_plain(normalized));
    }

    fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String {
        let value = self.preview_plain(normalized);
        match (&self.value_to_string, &self.step_size, include_unit) {
            (Some(f), _, true) => format!("{}{}", f(value), self.unit),
            (Some(f), _, false) => f(value),
            (None, Some(step_size), true) => {
                let num_digits = decimals_from_step_size(*step_size);
                format!("{:.num_digits$}{}", value, self.unit)
            }
            (None, Some(step_size), false) => {
                let num_digits = decimals_from_step_size(*step_size);
                format!("{:.num_digits$}", value)
            }
            (None, None, true) => format!("{}{}", value, self.unit),
            (None, None, false) => format!("{}", value),
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
        let value = self.range.unnormalize(normalized);
        match &self.step_size {
            Some(step_size) => self.range.snap_to_step(value, *step_size as Self::Plain),
            None => value,
        }
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

    fn as_ptr(&self) -> ParamPtr {
        ParamPtr::FloatParam(self as *const _ as *mut _)
    }
}

impl FloatParam {
    /// Build a new [`FloatParam`]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: &'static str, default: f32, range: FloatRange) -> Self {
        Self {
            value: default,
            range,
            name,
            ..Default::default()
        }
    }

    /// Set up a smoother that can gradually interpolate changes made to this parameter, preventing
    /// clicks and zipper noises.
    pub fn with_smoother(mut self, style: SmoothingStyle) -> Self {
        // Logarithmic smoothing will cause problems if the range goes through zero since then you
        // end up multplying by zero
        let goes_through_zero = match (&style, &self.range) {
            (
                SmoothingStyle::Logarithmic(_),
                FloatRange::Linear { min, max }
                | FloatRange::Skewed { min, max, .. }
                | FloatRange::SymmetricalSkewed { min, max, .. },
            ) => *min == 0.0 || *max == 0.0 || min.signum() != max.signum(),
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
    pub fn with_callback(mut self, callback: Arc<dyn Fn(f32) + Send + Sync>) -> Self {
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

    /// Set the distance between steps of a [FloatParam]. Mostly useful for quantizing GUI input. If
    /// this is set and if [`value_to_string`][Self::value_to_string] is not set, then this is also
    /// used when formatting the parameter. This must be a positive, nonzero number.
    pub fn with_step_size(mut self, step_size: f32) -> Self {
        self.step_size = Some(step_size);
        self
    }

    /// Use a custom conversion function to convert the plain, unnormalized value to a
    /// string.
    pub fn with_value_to_string(
        mut self,
        callback: Arc<dyn Fn(f32) -> String + Send + Sync>,
    ) -> Self {
        self.value_to_string = Some(callback);
        self
    }

    /// Use a custom conversion function to convert from a string to a plain, unnormalized
    /// value. If the string cannot be parsed, then this should return a `None`. If this
    /// happens while the parameter is being updated then the update will be canceled.
    ///
    /// The input string may or may not contain the unit, so you will need to be able to handle
    /// that.
    pub fn with_string_to_value(
        mut self,
        callback: Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>,
    ) -> Self {
        self.string_to_value = Some(callback);
        self
    }
}

/// Caldculate how many decimals to round to when displaying a floating point value with a specific
/// step size. We'll perform some rounding to ignore spurious extra precision caused by the floating
/// point quantization.
fn decimals_from_step_size(step_size: f32) -> usize {
    const SCALE: f32 = 1_000_000.0; // 10.0f32.powi(f32::DIGITS as i32)
    let step_size = (step_size * SCALE).round() / SCALE;

    let mut num_digits = 0;
    for decimals in 0..f32::DIGITS as i32 {
        if step_size * 10.0f32.powi(decimals) as f32 >= 1.0 {
            num_digits = decimals;
            break;
        }
    }

    num_digits as usize
}
