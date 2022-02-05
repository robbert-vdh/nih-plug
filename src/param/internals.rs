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

//! Implementation details for the parameter management.

use std::collections::HashMap;
use std::pin::Pin;

use super::{NormalizebleRange, Param};

/// Re-export for use in the [Params] proc-macro.
pub use serde_json::from_str as deserialize_field;
/// Re-export for use in the [Params] proc-macro.
pub use serde_json::to_string as serialize_field;

/// Describes a struct containing parameters and other persistent fields. The idea is that we can
/// have a normal struct containing [super::FloatParam] and other parameter types with attributes
/// assigning a unique identifier to each parameter. We can then build a mapping from those
/// parameter IDs to the parameters using the [Params::param_map] function. That way we can have
/// easy to work with JUCE-style parameter objects in the plugin without needing to manually
/// register each parameter, like you would in JUCE.
///
/// The other persistent parameters should be [PersistentField]s containing types that can be
/// serialized and deserialized with Serde.
///
/// Take a look at the example gain plugin to see how this should be used.
///
/// # Safety
///
/// This implementation is safe when using from the wrapper because the plugin object needs to be
/// pinned, and it can never outlive the wrapper.
pub trait Params {
    /// Create a mapping from unique parameter IDs to parameters. This is done for every parameter
    /// field marked with `#[id = "stable_name"]`. Dereferencing the pointers stored in the values
    /// is only valid as long as this pinned object is valid.
    fn param_map(self: Pin<&Self>) -> HashMap<&'static str, ParamPtr>;

    /// All parameter IDs from `param_map`, in a stable order. This order will be used to display
    /// the parameters.
    fn param_ids(self: Pin<&Self>) -> &'static [&'static str];

    /// Serialize all fields marked with `#[persist = "stable_name"]` into a hash map containing
    /// JSON-representations of those fields so they can be written to the plugin's state and
    /// recalled later. This uses [serialize_field] under the hood.
    fn serialize_fields(&self) -> HashMap<String, String>;

    /// Restore all fields marked with `#[persist = "stable_name"]` from a hashmap created by
    /// [Self::serialize_fields]. All of thse fields should be wrapped in a [PersistentField] with
    /// thread safe interior mutability, like an `RwLock` or a `Mutex`. This gets called when the
    /// plugin's state is being restored. This uses [deserialize_field] under the hood.
    fn deserialize_fields(&self, serialized: &HashMap<String, String>);
}

/// Internal pointers to parameters. This is an implementation detail used by the wrappers.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ParamPtr {
    FloatParam(*mut super::FloatParam),
    IntParam(*mut super::IntParam),
    BoolParam(*mut super::BoolParam),
}

// These pointers only point to fields on pinned structs, and the caller always needs to make sure
// that dereferencing them is safe
unsafe impl Send for ParamPtr {}
unsafe impl Sync for ParamPtr {}

/// The functinoality needed for persisting a field to the plugin's state, and for restoring values
/// when loading old state.
///
/// TODO: Modifying these fields (or any parameter for that matter) should mark the plugin's state
///       as dirty.
pub trait PersistentField<'a, T>: Send + Sync
where
    T: serde::Serialize + serde::Deserialize<'a>,
{
    fn set(&self, new_value: T);
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&T) -> R;
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
            ParamPtr::BoolParam(p) => (**p).name,
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
            ParamPtr::BoolParam(_) => "",
        }
    }

    /// Update the smoother state to point to the current value. Also used when initializing and
    /// restoring a plugin so everything is in sync. In that case the smoother should completely
    /// reset to the current value.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn update_smoother(&self, sample_rate: f32, reset: bool) {
        match &self {
            ParamPtr::FloatParam(p) => (**p).update_smoother(sample_rate, reset),
            ParamPtr::IntParam(p) => (**p).update_smoother(sample_rate, reset),
            ParamPtr::BoolParam(p) => (**p).update_smoother(sample_rate, reset),
        }
    }

    /// Set this parameter based on a string. Returns whether the updating succeeded. That can fail
    /// if the string cannot be parsed.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn set_from_string(&mut self, string: &str) -> bool {
        match &self {
            ParamPtr::FloatParam(p) => (**p).set_from_string(string),
            ParamPtr::IntParam(p) => (**p).set_from_string(string),
            ParamPtr::BoolParam(p) => (**p).set_from_string(string),
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
            ParamPtr::BoolParam(p) => (**p).normalized_value(),
        }
    }

    /// Set this parameter based on a normalized value.
    ///
    /// This does **not** update the smoother.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn set_normalized_value(&self, normalized: f32) {
        match &self {
            ParamPtr::FloatParam(p) => (**p).set_normalized_value(normalized),
            ParamPtr::IntParam(p) => (**p).set_normalized_value(normalized),
            ParamPtr::BoolParam(p) => (**p).set_normalized_value(normalized),
        }
    }

    /// Get the normalized value for a plain, unnormalized value, as a float. Used as part of the
    /// wrappers.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn preview_normalized(&self, plain: f32) -> f32 {
        match &self {
            ParamPtr::FloatParam(p) => (**p).range.normalize(plain),
            ParamPtr::IntParam(p) => (**p).range.normalize(plain as i32),
            ParamPtr::BoolParam(_) => plain,
        }
    }

    /// Get the plain, unnormalized value for a normalized value, as a float. Used as part of the
    /// wrappers.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn preview_plain(&self, normalized: f32) -> f32 {
        match &self {
            ParamPtr::FloatParam(p) => (**p).range.unnormalize(normalized),
            ParamPtr::IntParam(p) => (**p).range.unnormalize(normalized) as f32,
            ParamPtr::BoolParam(_) => normalized,
        }
    }

    /// Get the string representation for a normalized value. Used as part of the wrappers. Most
    /// plugin formats already have support for units, in which case it shouldn't be part of this
    /// string or some DAWs may show duplicate units.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String {
        match &self {
            ParamPtr::FloatParam(p) => (**p).normalized_value_to_string(normalized, include_unit),
            ParamPtr::IntParam(p) => (**p).normalized_value_to_string(normalized, include_unit),
            ParamPtr::BoolParam(p) => (**p).normalized_value_to_string(normalized, include_unit),
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
            ParamPtr::BoolParam(p) => (**p).string_to_normalized_value(string),
        }
    }
}

impl<'a, T> PersistentField<'a, T> for std::sync::RwLock<T>
where
    T: serde::Serialize + serde::Deserialize<'a> + Send + Sync,
{
    fn set(&self, new_value: T) {
        *self.write().expect("Poisoned RwLock on write") = new_value;
    }
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&T) -> R,
    {
        f(&self.read().expect("Poisoned RwLock on read"))
    }
}

impl<'a, T> PersistentField<'a, T> for parking_lot::RwLock<T>
where
    T: serde::Serialize + serde::Deserialize<'a> + Send + Sync,
{
    fn set(&self, new_value: T) {
        *self.write() = new_value;
    }
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&T) -> R,
    {
        f(&self.read())
    }
}

impl<'a, T> PersistentField<'a, T> for std::sync::Mutex<T>
where
    T: serde::Serialize + serde::Deserialize<'a> + Send + Sync,
{
    fn set(&self, new_value: T) {
        *self.lock().expect("Poisoned Mutex") = new_value;
    }
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&T) -> R,
    {
        f(&self.lock().expect("Poisoned Mutex"))
    }
}

macro_rules! impl_persistent_field_parking_lot_mutex {
    ($ty:ty) => {
        impl<'a, T> PersistentField<'a, T> for $ty
        where
            T: serde::Serialize + serde::Deserialize<'a> + Send + Sync,
        {
            fn set(&self, new_value: T) {
                *self.lock() = new_value;
            }
            fn map<F, R>(&self, f: F) -> R
            where
                F: Fn(&T) -> R,
            {
                f(&self.lock())
            }
        }
    };
}

impl_persistent_field_parking_lot_mutex!(parking_lot::Mutex<T>);
impl_persistent_field_parking_lot_mutex!(parking_lot::FairMutex<T>);
