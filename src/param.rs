//! TODO: Document how to use the [Param] trait. Also mention both interfaces: direct initialization
//!       + `..Default::default()`, and the builder interface. For the moment, just look at the gain
//!       example.

use std::fmt::Display;
use std::sync::Arc;

// Re-export for the [EnumParam]
// TODO: Consider re-exporting this from a non-root module to make it a bit less spammy:w
pub use strum::{Display, EnumIter, IntoEnumIterator as EnumIter};

pub mod internals;
pub mod range;
pub mod smoothing;

use self::range::{NormalizebleRange, Range};
use self::smoothing::{Smoother, SmoothingStyle};

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

/// An [IntParam]-backed categorical parameter that allows convenient conversion to and from a
/// simple enum. This enum must derive the re-exported [EnumIter], [EnumString] and [Display]
/// traits.
//
// TODO: Figure out a more sound way to get the same interface
pub struct EnumParam<T: EnumIter + Eq + Copy + Display> {
    /// The integer parameter backing this enum parameter.
    pub inner: IntParam,
    /// An associative list of the variants converted to an i32 and their names. We need this
    /// because we're doing some nasty type erasure things with [ParamPtr::Enum], so we can't
    /// directly query the associated functions on `T` after the parameter when handling function
    /// calls from the wrapper.
    variants: Vec<(T, String)>,
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

impl<T: EnumIter + Eq + Copy + Display + Default> Default for EnumParam<T> {
    fn default() -> Self {
        let variants: Vec<_> = T::iter().map(|v| (v, v.to_string())).collect();
        let default = T::default();

        Self {
            inner: IntParam {
                value: T::iter()
                    .position(|v| v == default)
                    .expect("Invalid variant in init") as i32,
                range: Range::Linear {
                    min: 0,
                    max: variants.len() as i32 - 1,
                },
                ..Default::default()
            },
            variants,
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
                        self.set_plain_value(plain);
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
                self.preview_normalized(self.value)
            }

            fn set_normalized_value(&mut self, normalized: f32) {
                self.set_plain_value(self.preview_plain(normalized));
            }

            fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String {
                let value = self.preview_plain(normalized);
                match (&self.value_to_string, &self.step_size, include_unit) {
                    (Some(f), _, true) => format!("{}{}", f(value), self.unit),
                    (Some(f), _, false) => format!("{}", f(value)),
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
                    Some(f) => f(string),
                    // TODO: Check how Rust's parse function handles trailing garbage
                    None => string.parse().ok(),
                }?;

                Some(self.preview_normalized(value))
            }

            fn preview_normalized(&self, plain: Self::Plain) -> f32 {
                self.range.normalize(plain)
            }

            fn preview_plain(&self, normalized: f32) -> Self::Plain {
                let value = self.range.unnormalize(normalized);
                match &self.step_size {
                    // Step size snapping is not defined for [IntParam], so this cast is here just
                    // so we can keep everything in this macro
                    Some(step_size) => self.range.snap_to_step(value, *step_size as Self::Plain),
                    None => value,
                }
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
                self.set_plain_value(plain);
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
        self.preview_normalized(self.value)
    }

    fn set_normalized_value(&mut self, normalized: f32) {
        self.set_plain_value(self.preview_plain(normalized));
    }

    fn normalized_value_to_string(&self, normalized: f32, _include_unit: bool) -> String {
        let value = self.preview_plain(normalized);
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

        Some(self.preview_normalized(value))
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

impl<T: EnumIter + Eq + Copy + Display> Param for EnumParam<T> {
    type Plain = T;

    fn update_smoother(&mut self, sample_rate: f32, reset: bool) {
        self.inner.update_smoother(sample_rate, reset)
    }

    fn set_from_string(&mut self, string: &str) -> bool {
        match self.variants.iter().find(|(_, repr)| repr == string) {
            Some((variant, _)) => {
                self.inner.set_plain_value(self.to_index(*variant));
                true
            }
            None => false,
        }
    }

    fn plain_value(&self) -> Self::Plain {
        self.from_index(self.inner.plain_value())
    }

    fn set_plain_value(&mut self, plain: Self::Plain) {
        self.inner.set_plain_value(self.to_index(plain))
    }

    fn normalized_value(&self) -> f32 {
        self.inner.normalized_value()
    }

    fn set_normalized_value(&mut self, normalized: f32) {
        self.inner.set_normalized_value(normalized)
    }

    fn normalized_value_to_string(&self, normalized: f32, _include_unit: bool) -> String {
        // XXX: As mentioned below, our type punning would cause `.to_string()` to print the
        //      incorect value. Because of that, we already stored the string representations for
        //      variants values in this struct.
        let plain = self.preview_plain(normalized);
        let index = self.to_index(plain);
        self.variants[index as usize].1.clone()
    }

    fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
        self.variants
            .iter()
            .find(|(_, repr)| repr == string)
            .map(|(variant, _)| self.preview_normalized(*variant))
    }

    fn preview_normalized(&self, plain: Self::Plain) -> f32 {
        self.inner.preview_normalized(self.to_index(plain))
    }

    fn preview_plain(&self, normalized: f32) -> Self::Plain {
        self.from_index(self.inner.preview_plain(normalized))
    }

    fn as_ptr(&self) -> internals::ParamPtr {
        internals::ParamPtr::EnumParam(
            self as *const EnumParam<T> as *mut EnumParam<T> as *mut EnumParam<internals::AnyEnum>,
        )
    }
}

impl<T: Display + Copy> Display for PlainParam<T> {
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

impl Display for BoolParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.value, &self.value_to_string) {
            (v, Some(func)) => write!(f, "{}", func(v)),
            (true, None) => write!(f, "On"),
            (false, None) => write!(f, "Off"),
        }
    }
}

impl<T: EnumIter + Eq + Copy + Display> Display for EnumParam<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.variants[self.inner.plain_value() as usize].1)
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

impl<T: EnumIter + Eq + Copy + Display> EnumParam<T> {
    /// Build a new [Self]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: &'static str, default: T) -> Self {
        let variants: Vec<_> = T::iter().map(|v| (v, v.to_string())).collect();

        Self {
            inner: IntParam {
                value: variants
                    .iter()
                    .position(|(v, _)| v == &default)
                    .expect("Invalid variant in init") as i32,
                range: Range::Linear {
                    min: 0,
                    max: variants.len() as i32 - 1,
                },
                name,
                ..Default::default()
            },
            variants,
        }
    }

    // We currently don't implement callbacks here. If we want to do that, then we'll need to add
    // the IntParam fields to the parameter itself.
    // TODO: Do exactly that
}

impl<T: EnumIter + Eq + Copy + Display> EnumParam<T> {
    // TODO: There doesn't seem to be a single enum crate that gives you a dense [0, n_variatns)
    //       mapping between integers and enum variants. So far linear search over this variants has
    //       been the best approach. We should probably replace this with our own macro at some
    //       point.

    /// The number of variants for this parameter
    //
    // This is part of the magic sauce that lets [ParamPtr::Enum] work. The type parmaeter there is
    // a dummy type, acting as a somewhat unsound way to do type erasure. Because all data is stored
    // in the struct after initialization (i.e. we no longer rely on T's specifics) and AnyParam is
    // represented by an i32 this EnumParam behaves correctly even when casted between Ts.
    //
    // TODO: Come up with a sounder way to do this.
    #[allow(clippy::len_without_is_empty)]
    #[inline(never)]
    pub fn len(&self) -> usize {
        self.variants.len()
    }

    /// Get the index associated to an enum variant.
    #[inline(never)]
    fn to_index(&self, variant: T) -> i32 {
        self.variants
            .iter()
            // This is somewhat shady, as `T` is going to be `AnyEnum` when this is indirectly
            // called from the wrapper.
            .position(|(v, _)| v == &variant)
            .expect("Invalid enum variant") as i32
    }

    /// Get a variant from a index.
    ///
    /// # Panics
    ///
    /// indices `>= Self::len()` will trigger a panic.
    #[allow(clippy::wrong_self_convention)]
    #[inline(never)]
    fn from_index(&self, index: i32) -> T {
        self.variants[index as usize].0
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
