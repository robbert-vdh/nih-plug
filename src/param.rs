//! TODO: Document how to use the [Param] trait. Also mention both interfaces: direct initialization
//!       + `..Default::default()`, and the builder interface. For the moment, just look at the gain
//!       example.

use std::fmt::Display;
use std::sync::Arc;

use self::range::{NormalizebleRange, Range};
use self::smoothing::{Smoother, SmoothingStyle};

pub mod internals;
pub mod range;
pub mod smoothing;

pub type FloatParam = PlainParam<f32>;
pub type IntParam = PlainParam<i32>;

/// Describes a single parameter of any type.
pub trait Param: Display {
    /// The plain parameter type.
    type Plain;

    /// Update the smoother state to point to the current value. Also used when initializing and
    /// restoring a plugin so everything is in sync. In that case the smoother should completely
    /// reset to the current value.
    fn update_smoother(&mut self, sample_rate: f32, reset: bool);

    /// Set this parameter based on a string. Returns whether the updating succeeded. That can fail
    /// if the string cannot be parsed.
    ///
    /// TODO: After implementing VST3, check if we handle parsing failures correctly
    fn set_from_string(&mut self, string: &str) -> bool;

    /// Get the unnormalized value for this parameter.
    fn plain_value(&self) -> Self::Plain;

    /// Set this parameter based on a plain, unnormalized value.
    ///
    /// This does **not** update the smoother.
    ///
    /// TDOO: Decide on whether this should update the smoother or not. That wouldn't be compatible
    /// with sample accurate automation when we add that.
    fn set_plain_value(&mut self, plain: Self::Plain);

    /// Get the normalized `[0, 1]` value for this parameter.
    fn normalized_value(&self) -> f32;

    /// Set this parameter based on a normalized value.
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
    /// wrappers.
    fn preview_plain(&self, normalized: f32) -> Self::Plain;

    /// Internal implementation detail for implementing [internals::Params]. This should not be used
    /// directly.
    fn as_ptr(&self) -> internals::ParamPtr;
}

/// A numerical parameter that's stored unnormalized. The range is used for the normalization
/// process.
///
/// You can either initialize the struct directly, using `..Default::default()` to fill in the
/// unused fields, or you can use the builder interface with [Self::new()].
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
pub struct PlainParam<T> {
    /// The field's current plain, unnormalized value. Should be initialized with the default value.
    /// Storing parameter values like this instead of in a single contiguous array is bad for cache
    /// locality, but it does allow for a much nicer declarative API.
    pub value: T,
    /// An optional smoother that will automatically interpolate between the new automation values
    /// set by the host.
    pub smoothed: Smoother<T>,
    /// Optional callback for listening to value changes. The argument passed to this function is
    /// the parameter's new **plain** value. This should not do anything expensive as it may be
    /// called multiple times in rapid succession.
    ///
    /// To use this, you'll probably want to store an `Arc<Atomic*>` alongside the parmater in the
    /// parmaeters struct, move a clone of that `Arc` into this closure, and then modify that.
    ///
    /// TODO: We probably also want to pass the old value to this function.
    pub value_changed: Option<Arc<dyn Fn(T) + Send + Sync>>,

    /// The distribution of the parameter's values.
    pub range: Range<T>,
    /// The distance between steps of a [FloatParam]. Ignored for [IntParam]. Mostly useful for
    /// quantizing GUI input. If this is set and if [Self::value_to_string] is not set, then this is
    /// also used when formatting the parameter. This must be a positive, nonzero number.
    pub step_size: Option<f32>,
    /// The parameter's human readable display name.
    pub name: &'static str,
    /// The parameter value's unit, added after `value_to_string` if that is set. NIH-plug will not
    /// automatically add a space before the unit.
    pub unit: &'static str,
    /// Optional custom conversion function from a plain **unnormalized** value to a string.
    pub value_to_string: Option<Arc<dyn Fn(T) -> String + Send + Sync>>,
    /// Optional custom conversion function from a string to a plain **unnormalized** value. If the
    /// string cannot be parsed, then this should return a `None`. If this happens while the
    /// parameter is being updated then the update will be canceled.
    pub string_to_value: Option<Arc<dyn Fn(&str) -> Option<T> + Send + Sync>>,
}

/// A simple boolean parmaeter.
#[repr(C, align(4))]
pub struct BoolParam {
    /// The field's current, normalized value. Should be initialized with the default value.
    pub value: bool,

    /// Optional callback for listening to value changes. The argument passed to this function is
    /// the parameter's new value. This should not do anything expensive as it may be called
    /// multiple times in rapid succession, and it can be run from both the GUI and the audio
    /// thread.
    pub value_changed: Option<Arc<dyn Fn(bool) + Send + Sync>>,

    /// The parameter's human readable display name.
    pub name: &'static str,
    /// Optional custom conversion function from a boolean value to a string.
    pub value_to_string: Option<Arc<dyn Fn(bool) -> String + Send + Sync>>,
    /// Optional custom conversion function from a string to a boolean value. If the string cannot
    /// be parsed, then this should return a `None`. If this happens while the parameter is being
    /// updated then the update will be canceled.
    pub string_to_value: Option<Arc<dyn Fn(&str) -> Option<bool> + Send + Sync>>,
}

impl<T> Default for PlainParam<T>
where
    T: Default,
    Range<T>: Default,
{
    fn default() -> Self {
        Self {
            value: T::default(),
            smoothed: Smoother::none(),
            value_changed: None,
            range: Range::default(),
            step_size: None,
            name: "",
            unit: "",
            value_to_string: None,
            string_to_value: None,
        }
    }
}

#[allow(clippy::derivable_impls)]
impl Default for BoolParam {
    fn default() -> Self {
        Self {
            value: false,
            value_changed: None,
            name: "",
            value_to_string: None,
            string_to_value: None,
        }
    }
}

macro_rules! impl_plainparam {
    ($ty:ident, $plain:ty) => {
        impl Param for $ty {
            type Plain = $plain;

            fn update_smoother(&mut self, sample_rate: f32, reset: bool) {
                if reset {
                    self.smoothed.reset(self.value);
                } else {
                    self.smoothed.set_target(sample_rate, self.value);
                }
            }

            fn set_from_string(&mut self, string: &str) -> bool {
                let value = match &self.string_to_value {
                    Some(f) => f(string),
                    // TODO: Check how Rust's parse function handles trailing garbage
                    None => string.parse().ok(),
                };

                match value {
                    Some(plain) => {
                        self.value = plain;
                        true
                    }
                    None => false,
                }
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
                self.range.normalize(self.value)
            }

            fn set_normalized_value(&mut self, normalized: f32) {
                self.set_plain_value(self.range.unnormalize(normalized));
            }

            fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String {
                let value = self.range.unnormalize(normalized);
                match (&self.value_to_string, include_unit) {
                    (Some(f), true) => format!("{}{}", f(value), self.unit),
                    (Some(f), false) => format!("{}", f(value)),
                    (None, true) => format!("{}{}", value, self.unit),
                    (None, false) => format!("{}", value),
                }
            }

            fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
                let value = match &self.string_to_value {
                    Some(f) => f(string),
                    // TODO: Check how Rust's parse function handles trailing garbage
                    None => string.parse().ok(),
                }?;

                Some(self.range.normalize(value))
            }

            fn preview_normalized(&self, plain: Self::Plain) -> f32 {
                self.range.normalize(plain)
            }

            fn preview_plain(&self, normalized: f32) -> Self::Plain {
                self.range.unnormalize(normalized)
            }

            fn as_ptr(&self) -> internals::ParamPtr {
                internals::ParamPtr::$ty(self as *const $ty as *mut $ty)
            }
        }
    };
}

impl_plainparam!(FloatParam, f32);
impl_plainparam!(IntParam, i32);

impl Param for BoolParam {
    type Plain = bool;

    fn update_smoother(&mut self, _sample_rate: f32, _init: bool) {
        // Can't really smooth a binary parameter now can you
    }

    fn set_from_string(&mut self, string: &str) -> bool {
        let value = match &self.string_to_value {
            Some(f) => f(string),
            None => Some(string.eq_ignore_ascii_case("true") || string.eq_ignore_ascii_case("on")),
        };

        match value {
            Some(plain) => {
                self.value = plain;
                true
            }
            None => false,
        }
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
        if self.value {
            1.0
        } else {
            0.0
        }
    }

    fn set_normalized_value(&mut self, normalized: f32) {
        self.set_plain_value(normalized > 0.5);
    }

    fn normalized_value_to_string(&self, normalized: f32, _include_unit: bool) -> String {
        let value = normalized > 0.5;
        match (value, &self.value_to_string) {
            (v, Some(f)) => f(v),
            (true, None) => String::from("On"),
            (false, None) => String::from("Off"),
        }
    }

    fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
        let value = match &self.string_to_value {
            Some(f) => f(string),
            None => Some(string.eq_ignore_ascii_case("true") || string.eq_ignore_ascii_case("on")),
        }?;

        Some(if value { 1.0 } else { 0.0 })
    }

    fn preview_normalized(&self, plain: Self::Plain) -> f32 {
        if plain {
            1.0
        } else {
            0.0
        }
    }

    fn preview_plain(&self, normalized: f32) -> Self::Plain {
        normalized > 0.5
    }

    fn as_ptr(&self) -> internals::ParamPtr {
        internals::ParamPtr::BoolParam(self as *const BoolParam as *mut BoolParam)
    }
}

impl<T: Display + Copy> Display for PlainParam<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.value_to_string, &self.step_size) {
            (Some(func), _) => write!(f, "{}{}", func(self.value), self.unit),
            (None, Some(step_size)) => {
                // Determine the number of digits based on the step size. We'll perform some
                // rounding to ignore spurious extra precision caused by the floating point
                // quantization.
                const SCALE: f32 = 1_000_000.0; // 10.0f32.powi(f32::DIGITS as i32)
                let step_size = (step_size * SCALE).round() / SCALE;

                let mut num_digits = 0usize;
                for decimals in 0..f32::DIGITS as usize {
                    if step_size * decimals as f32 >= 1.0 {
                        num_digits = decimals;
                        break;
                    }
                }

                write!(f, "{:.num_digits$}{}", self.value, self.unit)
            }
            _ => write!(f, "{}{}", self.value, self.unit),
        }
    }
}

impl Display for BoolParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.value, &self.value_to_string) {
            (v, Some(func)) => write!(f, "{}", func(v)),
            (true, None) => write!(f, "On"),
            (false, None) => write!(f, "Off"),
        }
    }
}

impl<T: Default> PlainParam<T>
where
    Range<T>: Default,
{
    /// Build a new [Self]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: &'static str, default: T, range: Range<T>) -> Self {
        Self {
            value: default,
            range,
            name,
            ..Default::default()
        }
    }

    /// Run a callback whenever this parameter's value changes. The argument passed to this function
    /// is the parameter's new value. This should not do anything expensive as it may be called
    /// multiple times in rapid succession, and it can be run from both the GUI and the audio
    /// thread.
    pub fn with_smoother(mut self, style: SmoothingStyle) -> Self {
        self.smoothed = Smoother::new(style);
        self
    }

    /// Run a callback whenever this parameter's value changes. The argument passed to this function
    /// is the parameter's new value. This should not do anything expensive as it may be called
    /// multiple times in rapid succession, and it can be run from both the GUI and the audio
    /// thread.
    pub fn with_callback(mut self, callback: Arc<dyn Fn(T) + Send + Sync>) -> Self {
        self.value_changed = Some(callback);
        self
    }

    /// Display a unit when rendering this parameter to a string. Appended after the
    /// [Self::value_to_string] function if that is also set. NIH-plug will not
    /// automatically add a space before the unit.
    pub fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    /// Use a custom conversion function to convert the plain, unnormalized value to a
    /// string.
    pub fn with_value_to_string(
        mut self,
        callback: Arc<dyn Fn(T) -> String + Send + Sync>,
    ) -> Self {
        self.value_to_string = Some(callback);
        self
    }

    // `with_step_size` is only implemented for the f32 version

    /// Use a custom conversion function to convert from a string to a plain, unnormalized
    /// value. If the string cannot be parsed, then this should return a `None`. If this
    /// happens while the parameter is being updated then the update will be canceled.
    pub fn with_string_to_value<F>(
        mut self,
        callback: Arc<dyn Fn(&str) -> Option<T> + Send + Sync>,
    ) -> Self {
        self.string_to_value = Some(callback);
        self
    }
}

impl PlainParam<f32> {
    /// Set the distance between steps of a [FloatParam]. Mostly useful for quantizing GUI input. If
    /// this is set and if [Self::value_to_string] is not set, then this is also used when
    /// formatting the parameter. This must be a positive, nonzero number.
    pub fn with_step_size(mut self, step_size: f32) -> Self {
        self.step_size = Some(step_size);
        self
    }
}

impl BoolParam {
    /// Build a new [Self]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: &'static str, default: bool) -> Self {
        Self {
            value: default,
            name,
            ..Default::default()
        }
    }

    /// Run a callback whenever this parameter's value changes. The argument passed to this function
    /// is the parameter's new value. This should not do anything expensive as it may be called
    /// multiple times in rapid succession, and it can be run from both the GUI and the audio
    /// thread.
    pub fn with_callback(mut self, callback: Arc<dyn Fn(bool) + Send + Sync>) -> Self {
        self.value_changed = Some(callback);
        self
    }

    /// Use a custom conversion function to convert the boolean value to a string.
    pub fn with_value_to_string(
        mut self,
        callback: Arc<dyn Fn(bool) -> String + Send + Sync>,
    ) -> Self {
        self.value_to_string = Some(callback);
        self
    }

    /// Use a custom conversion function to convert from a string to a boolean value. If the string
    /// cannot be parsed, then this should return a `None`. If this happens while the parameter is
    /// being updated then the update will be canceled.
    pub fn with_string_to_value(
        mut self,
        callback: Arc<dyn Fn(&str) -> Option<bool> + Send + Sync>,
    ) -> Self {
        self.string_to_value = Some(callback);
        self
    }
}
