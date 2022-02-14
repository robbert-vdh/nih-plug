//! Enum parameters. `enum` is a keyword, so `enums` it is.

use std::fmt::Display;

use super::internals::ParamPtr;
use super::range::Range;
use super::{IntParam, Param};

// Re-export for the [EnumParam]
// TODO: Consider re-exporting this from a non-root module to make it a bit less spammy:w
pub use strum::{Display, EnumIter, EnumMessage, IntoEnumIterator as EnumIter};

/// An [IntParam]-backed categorical parameter that allows convenient conversion to and from a
/// simple enum. This enum must derive the re-exported [EnumIter] and [EnumMessage] and [Display]
/// traits. You can use the `#[strum(message = "Foo Bar")]` to override the name of the variant.
//
// TODO: Figure out a more sound way to get the same interface
pub struct EnumParam<T: EnumIter + EnumMessage + Eq + Copy + Display> {
    /// The integer parameter backing this enum parameter.
    pub inner: IntParam,
    /// An associative list of the variants converted to an i32 and their names. We need this
    /// because we're doing some nasty type erasure things with [ParamPtr::EnumParam], so we can't
    /// directly query the associated functions on `T` after the parameter when handling function
    /// calls from the wrapper.
    variants: Vec<(T, String)>,
}

impl<T: EnumIter + EnumMessage + Eq + Copy + Display + Default> Default for EnumParam<T> {
    fn default() -> Self {
        let variants: Vec<_> = Self::build_variants();
        let default = T::default();

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
                ..Default::default()
            },
            variants,
        }
    }
}

impl<T: EnumIter + EnumMessage + Eq + Copy + Display> Display for EnumParam<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.variants[self.inner.plain_value() as usize].1)
    }
}

impl<T: EnumIter + EnumMessage + Eq + Copy + Display> Param for EnumParam<T> {
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

    fn as_ptr(&self) -> ParamPtr {
        ParamPtr::EnumParam(
            self as *const EnumParam<T> as *mut EnumParam<T>
                as *mut EnumParam<super::internals::AnyEnum>,
        )
    }
}

impl<T: EnumIter + EnumMessage + Eq + Copy + Display> EnumParam<T> {
    /// Build a new [Self]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: &'static str, default: T) -> Self {
        let variants: Vec<_> = Self::build_variants();

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

impl<T: EnumIter + EnumMessage + Eq + Copy + Display> EnumParam<T> {
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
    pub fn len(&self) -> usize {
        self.variants.len()
    }

    /// Get the index associated to an enum variant.
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
    fn from_index(&self, index: i32) -> T {
        self.variants[index as usize].0
    }

    fn build_variants() -> Vec<(T, String)> {
        T::iter()
            .map(|v| {
                (
                    v,
                    v.get_message()
                        .map(|custom_name| custom_name.to_string())
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect()
    }
}
