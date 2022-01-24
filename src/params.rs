// nih-plugs: plugins, but rewritten in Rust
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

use std::{fmt::Display, sync::atomic::Ordering};

use crate::atomic::AtomicType;

/// Describes a single normalized parameter and also stores its value.
pub enum Param {
    FloatParam(PlainParam<f32>),
    IntParam(PlainParam<i32>),
}

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
pub struct PlainParam<T: AtomicType> {
    /// The field's current, normalized value. Should be initialized with the default value using
    /// `T::new_atomic(...)` ([AtomicType::new_atomic]). Storing parameter values like this instead
    /// of in a single contiguous array is bad for cache locality, but it does allow for a much
    /// nicer declarative API.
    pub value: <T as AtomicType>::AtomicType,

    /// The distribution of the parameter's values.
    pub range: Range<T>,
    /// The parameter's human readable display name.
    pub name: &'static str,
    /// The parameter value's unit, added after `value_to_string` if that is set.
    pub unit: &'static str,
    /// Optional custom conversion function from an **unnormalized** value to a string.
    pub value_to_string: Option<Box<dyn Fn(T) -> String>>,
    /// Optional custom conversion function from a string to an **unnormalized** value. If the
    /// string cannot be parsed, then this should return a `None`. If this happens while the
    /// parameter is being updated then the update will be canceled.
    pub string_to_value: Option<Box<dyn Fn(&str) -> Option<T>>>,
}

impl Param {
    /// Get the human readable name for this parameter.
    pub fn name(&self) -> &'static str {
        match &self {
            Param::FloatParam(p) => p.name,
            Param::IntParam(p) => p.name,
        }
    }

    /// Set this parameter based on a string. Returns whether the updating succeeded. That can fail
    /// if the string cannot be parsed.
    ///
    /// TODO: After implementing VST3, check if we handle parsing failures correctly
    pub fn from_string(&self, string: &str) -> bool {
        // TODO: Debug asserts on failures
        match &self {
            Param::FloatParam(p) => {
                let value = match &p.string_to_value {
                    Some(f) => f(string),
                    // TODO: Check how Rust's parse function handles trailing garbage
                    None => string.parse().ok(),
                };

                match value {
                    Some(unnormalized) => {
                        p.value.store(unnormalized, Ordering::Relaxed);
                        true
                    }
                    None => false,
                }
            }
            Param::IntParam(p) => {
                let value = match &p.string_to_value {
                    Some(f) => f(string),
                    None => string.parse().ok(),
                };

                match value {
                    Some(unnormalized) => {
                        p.value.store(unnormalized, Ordering::Relaxed);
                        true
                    }
                    None => false,
                }
            }
        }
    }

    /// Get the normalized `[0, 1]` value for this parameter.
    pub fn normalized_value(&self) -> f32 {
        match &self {
            Param::FloatParam(p) => p.range.normalize(p.value.load(Ordering::Relaxed)),
            Param::IntParam(p) => p.range.normalize(p.value.load(Ordering::Relaxed)),
        }
    }

    /// Set this parameter based on a normalized value.
    pub fn set_normalized_value(&self, normalized: f32) {
        match &self {
            Param::FloatParam(p) => p
                .value
                .store(p.range.unnormalize(normalized), Ordering::Relaxed),
            Param::IntParam(p) => p
                .value
                .store(p.range.unnormalize(normalized), Ordering::Relaxed),
        }
    }
}

impl Display for Param {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            Param::FloatParam(p) => {
                let unnormalized = p.value.load(Ordering::Relaxed);
                match &p.value_to_string {
                    Some(func) => write!(f, "{}{}", func(unnormalized), p.unit),
                    None => write!(f, "{}{}", unnormalized, p.unit),
                }
            }
            Param::IntParam(p) => {
                let unnormalized = p.value.load(Ordering::Relaxed);
                match &p.value_to_string {
                    Some(func) => write!(f, "{}{}", func(unnormalized), p.unit),
                    None => write!(f, "{}{}", unnormalized, p.unit),
                }
            }
        }
    }
}

// TODO: Clamping
impl NormalizebleRange<f32> for Range<f32> {
    fn normalize(&self, unnormalized: f32) -> f32 {
        match &self {
            Range::Linear { min, max } => (unnormalized - min) / (max - min),
        }
    }

    fn unnormalize(&self, normalized: f32) -> f32 {
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
    }

    fn unnormalize(&self, normalized: f32) -> i32 {
        match &self {
            Range::Linear { min, max } => (normalized * (max - min) as f32) as i32 + min,
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
}
