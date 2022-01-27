// nih-plug: plugins, but rewritten in Rust
// Copyright (C) 2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::collections::HashMap;
use std::fmt::Display;
use std::pin::Pin;

pub type FloatParam = PlainParam<f32>;
pub type IntParam = PlainParam<i32>;

/// A distribution for a parameter's range. Probably need to add some forms of skewed ranges and
/// maybe a callback based implementation at some point.
#[derive(Debug)]
pub enum Range<T> {
    Linear { min: T, max: T },
}

/// A normalizable range for type `T`, where `self` is expected to be a type `R<T>`. Higher kinded
/// types would have made this trait definition a lot clearer.
trait NormalizebleRange<T> {
    /// Normalize an unnormalized value. Will be clamped to the bounds of the range if the
    /// normalized value exceeds `[0, 1]`.
    fn normalize(&self, unnormalized: T) -> f32;

    /// Unnormalize a normalized value. Will be clamped to `[0, 1]` if the unnormalized value would
    /// exceed that range.
    fn unnormalize(&self, normalized: f32) -> T;
}

/// A numerical parameter that's stored unnormalized. The range is used for the normalization
/// process.
pub struct PlainParam<T> {
    /// The field's current, normalized value. Should be initialized with the default value. Storing
    /// parameter values like this instead of in a single contiguous array is bad for cache
    /// locality, but it does allow for a much nicer declarative API.
    pub value: T,

    /// The distribution of the parameter's values.
    pub range: Range<T>,
    /// The parameter's human readable display name.
    pub name: &'static str,
    /// The parameter value's unit, added after `value_to_string` if that is set.
    pub unit: &'static str,
    /// Optional custom conversion function from an **unnormalized** value to a string.
    pub value_to_string: Option<Box<dyn Send + Sync + Fn(T) -> String>>,
    /// Optional custom conversion function from a string to an **unnormalized** value. If the
    /// string cannot be parsed, then this should return a `None`. If this happens while the
    /// parameter is being updated then the update will be canceled.
    pub string_to_value: Option<Box<dyn Send + Sync + Fn(&str) -> Option<T>>>,
}

macro_rules! impl_plainparam {
    ($ty:ident) => {
        impl $ty {
            /// Set this parameter based on a string. Returns whether the updating succeeded. That
            /// can fail if the string cannot be parsed.
            ///
            /// TODO: After implementing VST3, check if we handle parsing failures correctly
            pub fn from_string(&mut self, string: &str) -> bool {
                let value = match &self.string_to_value {
                    Some(f) => f(string),
                    // TODO: Check how Rust's parse function handles trailing garbage
                    None => string.parse().ok(),
                };

                match value {
                    Some(unnormalized) => {
                        self.value = unnormalized;
                        true
                    }
                    None => false,
                }
            }

            /// Get the normalized `[0, 1]` value for this parameter.
            pub fn normalized_value(&self) -> f32 {
                self.range.normalize(self.value)
            }

            /// Set this parameter based on a normalized value.
            pub fn set_normalized_value(&mut self, normalized: f32) {
                self.value = self.range.unnormalize(normalized);
            }

            /// Get the string representation for a normalized value. Used as part of the wrappers.
            pub fn normalized_value_to_string(&self, normalized: f32) -> String {
                let value = self.range.unnormalize(normalized);
                match &self.value_to_string {
                    Some(f) => format!("{}{}", f(value), self.unit),
                    None => format!("{}{}", value, self.unit),
                }
            }

            /// Get the string representation for a normalized value. Used as part of the wrappers.
            pub fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
                let value = match &self.string_to_value {
                    Some(f) => f(string),
                    // TODO: Check how Rust's parse function handles trailing garbage
                    None => string.parse().ok(),
                }?;

                Some(self.range.normalize(value))
            }

            /// Implementation detail for implementing [Params]. This should not be used directly.
            pub fn as_ptr(&self) -> ParamPtr {
                ParamPtr::$ty(self as *const $ty as *mut $ty)
            }
        }
    };
}

impl_plainparam!(FloatParam);
impl_plainparam!(IntParam);

impl<T: Display + Copy> Display for PlainParam<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.value_to_string {
            Some(func) => write!(f, "{}{}", func(self.value), self.unit),
            None => write!(f, "{}{}", self.value, self.unit),
        }
    }
}

impl NormalizebleRange<f32> for Range<f32> {
    fn normalize(&self, unnormalized: f32) -> f32 {
        match &self {
            Range::Linear { min, max } => (unnormalized - min) / (max - min),
        }
        .clamp(0.0, 1.0)
    }

    fn unnormalize(&self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match &self {
            Range::Linear { min, max } => (normalized * (max - min)) + min,
        }
    }
}

impl NormalizebleRange<i32> for Range<i32> {
    fn normalize(&self, unnormalized: i32) -> f32 {
        match &self {
            Range::Linear { min, max } => (unnormalized - min) as f32 / (max - min) as f32,
        }
        .clamp(0.0, 1.0)
    }

    fn unnormalize(&self, normalized: f32) -> i32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match &self {
            Range::Linear { min, max } => (normalized * (max - min) as f32).round() as i32 + min,
        }
    }
}

/// Describes a struct containing parameters. The idea is that we can have a normal struct
/// containing [FloatParam] and other parameter types with attributes describing a unique identifier
/// for each parameter. We can then build a mapping from those parameter IDs to the parameters using
/// the [Params::param_map] function. That way we can have easy to work with JUCE-style parameter
/// objects in the plugin without needing to manually register each parameter, like you would in
/// JUCE.
///
/// # Safety
///
/// This implementation is safe when using from the wrapper because the plugin object needs to be
/// pinned, and it can never outlive the wrapper.
pub trait Params {
    /// Create a mapping from unique parameter IDs to parameters. Dereferencing the pointers stored
    /// in the values is only valid as long as this pinned object is valid.
    fn param_map(self: Pin<&Self>) -> HashMap<&'static str, ParamPtr>;
}

/// Internal pointers to parameters. This is an implementation detail used by the wrappers.
#[derive(Debug)]
pub enum ParamPtr {
    FloatParam(*mut FloatParam),
    IntParam(*mut IntParam),
}

impl ParamPtr {
    /// Get the human readable name for this parameter.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn name(&self) -> &'static str {
        match &self {
            ParamPtr::FloatParam(p) => (**p).name,
            ParamPtr::IntParam(p) => (**p).name,
        }
    }

    /// Get the unit label for this parameter.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn unit(&self) -> &'static str {
        match &self {
            ParamPtr::FloatParam(p) => (**p).unit,
            ParamPtr::IntParam(p) => (**p).unit,
        }
    }

    /// Set this parameter based on a string. Returns whether the updating succeeded. That can fail
    /// if the string cannot be parsed.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn from_string(&mut self, string: &str) -> bool {
        match &self {
            ParamPtr::FloatParam(p) => (**p).from_string(string),
            ParamPtr::IntParam(p) => (**p).from_string(string),
        }
    }

    /// Get the normalized `[0, 1]` value for this parameter.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn normalized_value(&self) -> f32 {
        match &self {
            ParamPtr::FloatParam(p) => (**p).normalized_value(),
            ParamPtr::IntParam(p) => (**p).normalized_value(),
        }
    }

    /// Set this parameter based on a normalized value.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn set_normalized_value(&self, normalized: f32) {
        match &self {
            ParamPtr::FloatParam(p) => (**p).set_normalized_value(normalized),
            ParamPtr::IntParam(p) => (**p).set_normalized_value(normalized),
        }
    }

    /// Get the normalized value for an unnormalized value, as a float. Used as part of the
    /// wrappers.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn preview_normalized(&self, unnormalized: f32) -> f32 {
        match &self {
            ParamPtr::FloatParam(p) => (**p).range.normalize(unnormalized),
            ParamPtr::IntParam(p) => (**p).range.normalize(unnormalized as i32),
        }
    }

    /// Get the unnormalized value for a normalized value, as a float. Used as part of the wrappers.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn preview_unnormalized(&self, normalized: f32) -> f32 {
        match &self {
            ParamPtr::FloatParam(p) => (**p).range.unnormalize(normalized),
            ParamPtr::IntParam(p) => (**p).range.unnormalize(normalized) as f32,
        }
    }

    /// Get the string representation for a normalized value. Used as part of the wrappers.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn normalized_value_to_string(&self, normalized: f32) -> String {
        match &self {
            ParamPtr::FloatParam(p) => (**p).normalized_value_to_string(normalized),
            ParamPtr::IntParam(p) => (**p).normalized_value_to_string(normalized),
        }
    }

    /// Get the string representation for a normalized value. Used as part of the wrappers.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
        match &self {
            ParamPtr::FloatParam(p) => (**p).string_to_normalized_value(string),
            ParamPtr::IntParam(p) => (**p).string_to_normalized_value(string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_linear_float_range() -> Range<f32> {
        Range::Linear {
            min: 10.0,
            max: 20.0,
        }
    }

    fn make_linear_int_range() -> Range<i32> {
        Range::Linear { min: -10, max: 10 }
    }

    #[test]
    fn range_normalize_linear_float() {
        let range = make_linear_float_range();
        assert_eq!(range.normalize(17.5), 0.75);
    }

    #[test]
    fn range_normalize_linear_int() {
        let range = make_linear_int_range();
        assert_eq!(range.normalize(-5), 0.25);
    }

    #[test]
    fn range_unnormalize_linear_float() {
        let range = make_linear_float_range();
        assert_eq!(range.unnormalize(0.25), 12.5);
    }

    #[test]
    fn range_unnormalize_linear_int() {
        let range = make_linear_int_range();
        assert_eq!(range.unnormalize(0.75), 5);
    }

    #[test]
    fn range_unnormalize_linear_int_rounding() {
        let range = make_linear_int_range();
        assert_eq!(range.unnormalize(0.73), 5);
    }
}
