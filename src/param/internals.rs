//! Implementation details for the parameter management.

use std::collections::HashMap;
use std::pin::Pin;

use super::Param;

pub use nih_plug_derive::Params;
/// Re-export for use in the [`Params`] proc-macro.
pub use serde_json::from_str as deserialize_field;
/// Re-export for use in the [`Params`] proc-macro.
pub use serde_json::to_string as serialize_field;

/// Describes a struct containing parameters and other persistent fields.
///
/// This trait can be derived on a struct containing [`FloatParam`][super::FloatParam] and other
/// parameter fields. When deriving this trait, any of those parameter fields should have the `#[id
/// = "stable"]` attribute, where `stable` is an up to 6 character long string (to avoid collisions)
/// that will be used to identify the parameter internall so you can safely move it around and
/// rename the field without breaking compatibility with old presets.
///
/// The struct can also contain other fields that should be persisted along with the rest of the
/// preset data. These fields should be [`PersistentField`]s annotated with the `#[persist = "key"]`
/// attribute containing types that can be serialized and deserialized with
/// [Serde](https://serde.rs/).
///
/// And finally when deriving this trait, it is also possible to inherit the parameters from other
/// `Params` objects by adding the `#[nested = "Group Name"]` attribute to those fields. These
/// groups will be displayed as a tree-like structure if your DAW supports it. Parameter IDs and
/// persisting keys still need to be **unique** when usting nested parameter structs. This currently
/// has the following caveats:
///
/// - Enforcing that parameter IDs and persist keys are unique does not work across nested structs.
/// - Deserializing persisted fields will give false positives about fields not existing.
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

    /// Contains group names for each parameter in [`param_map()`][Self::param_map()]. This is
    /// either an empty string for top level parameters, or a slash/delimited `"Group Name 1/Group
    /// Name 2"` string for parameters that belong to `#[nested = "Name"]` parameter objects.
    fn param_groups(self: Pin<&Self>) -> HashMap<&'static str, String>;

    /// All parameter IDs from [`param_map()`][Self::param_map()], in a stable order. This order
    /// will be used to display the parameters.
    ///
    /// TODO: This used to be a static slice, but now that we supported nested parameter objects
    ///       that's become a bit more difficult since Rust does not have a convenient way to
    ///       concatenate an arbitrary number of static slices. There's probably a better way to do
    ///       this.
    fn param_ids(self: Pin<&Self>) -> Vec<&'static str>;

    /// Serialize all fields marked with `#[persist = "stable_name"]` into a hash map containing
    /// JSON-representations of those fields so they can be written to the plugin's state and
    /// recalled later. This uses [`serialize_field()`] under the hood.
    fn serialize_fields(&self) -> HashMap<String, String>;

    /// Restore all fields marked with `#[persist = "stable_name"]` from a hashmap created by
    /// [`serialize_fields()`][Self::serialize_fields()]. All of thse fields should be wrapped in a
    /// [`PersistentField`] with thread safe interior mutability, like an `RwLock` or a `Mutex`.
    /// This gets called when the plugin's state is being restored. This uses [deserialize_field()]
    /// under the hood.
    fn deserialize_fields(&self, serialized: &HashMap<String, String>);
}

/// Internal pointers to parameters. This is an implementation detail used by the wrappers for type
/// erasure.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ParamPtr {
    FloatParam(*mut super::FloatParam),
    IntParam(*mut super::IntParam),
    BoolParam(*mut super::BoolParam),
    /// Since we can't encode the actual enum here, this inner parameter struct contains all of the
    /// relevant information from the enum so it can be type erased.
    EnumParam(*mut super::enums::EnumParamInner),
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

/// Generate a [`ParamPtr`] function that forwards the function call to the underlying `Param`. We
/// can't have an `.as_param()` function since the return type would differ depending on the
/// underlying parameter type, so instead we need to type erase all of the functions individually.
macro_rules! param_ptr_forward(
    (pub unsafe fn $method:ident(&self $(, $arg_name:ident: $arg_ty:ty)*) $(-> $ret:ty)?) => {
        /// Calls the corresponding method on the underlying [`Param`] object.
        ///
        /// # Safety
        ///
        /// Calling this function is only safe as long as the object this [`ParamPtr`] was created
        /// for is still alive.
        pub unsafe fn $method(&self $(, $arg_name: $arg_ty)*) $(-> $ret)? {
            match &self {
                ParamPtr::FloatParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::IntParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::BoolParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::EnumParam(p) => (**p).$method($($arg_name),*),
            }
        }
    };
    // XXX: Is there a way to combine these two? Hygienic macros don't let you call `&self` without
    //      it being defined in the macro.
    (pub unsafe fn $method:ident(&mut self $(, $arg_name:ident: $arg_ty:ty)*) $(-> $ret:ty)?) => {
        /// Calls the corresponding method on the underlying [`Param`] object.
        ///
        /// # Safety
        ///
        /// Calling this function is only safe as long as the object this [`ParamPtr`] was created
        /// for is still alive.
        pub unsafe fn $method(&mut self $(, $arg_name: $arg_ty)*) $(-> $ret)? {
            match &self {
                ParamPtr::FloatParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::IntParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::BoolParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::EnumParam(p) => (**p).$method($($arg_name),*),
            }
        }
    };
);

impl ParamPtr {
    param_ptr_forward!(pub unsafe fn name(&self) -> &'static str);
    param_ptr_forward!(pub unsafe fn step_count(&self) -> Option<usize>);
    param_ptr_forward!(pub unsafe fn unit(&self) -> &'static str);
    param_ptr_forward!(pub unsafe fn update_smoother(&self, sample_rate: f32, reset: bool));
    param_ptr_forward!(pub unsafe fn initialize_block_smoother(&mut self, max_block_size: usize));
    param_ptr_forward!(pub unsafe fn normalized_value(&self) -> f32);
    param_ptr_forward!(pub unsafe fn set_normalized_value(&self, normalized: f32));
    param_ptr_forward!(pub unsafe fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String);
    param_ptr_forward!(pub unsafe fn string_to_normalized_value(&self, string: &str) -> Option<f32>);

    // These functions involve casts since the plugin formats only do floating point types, so we
    // can't generate them with the macro:

    /// Get the plain value for a plain, unnormalized value, as a float. Useful in conjunction with
    /// [`preview_plain()`][Self::preview_plain()] to compare a snapped discrete value to a
    /// parameter's current snapped value without having to do a back and forth conversion using
    /// normalized values.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn plain_value(&self) -> f32 {
        match &self {
            ParamPtr::FloatParam(p) => (**p).plain_value(),
            ParamPtr::IntParam(p) => (**p).plain_value() as f32,
            ParamPtr::BoolParam(p) => (**p).normalized_value(),
            ParamPtr::EnumParam(p) => (**p).plain_value() as f32,
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
            ParamPtr::FloatParam(p) => (**p).preview_normalized(plain),
            ParamPtr::IntParam(p) => (**p).preview_normalized(plain as i32),
            ParamPtr::BoolParam(_) => plain,
            ParamPtr::EnumParam(p) => (**p).preview_normalized(plain as i32),
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
            ParamPtr::FloatParam(p) => (**p).preview_plain(normalized),
            ParamPtr::IntParam(p) => (**p).preview_plain(normalized) as f32,
            ParamPtr::BoolParam(_) => normalized,
            ParamPtr::EnumParam(p) => (**p).preview_plain(normalized) as f32,
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

impl<'a, T> PersistentField<'a, T> for atomic_refcell::AtomicRefCell<T>
where
    T: serde::Serialize + serde::Deserialize<'a> + Send + Sync,
{
    fn set(&self, new_value: T) {
        *self.borrow_mut() = new_value;
    }
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&T) -> R,
    {
        f(&self.borrow())
    }
}

impl_persistent_field_parking_lot_mutex!(parking_lot::Mutex<T>);
impl_persistent_field_parking_lot_mutex!(parking_lot::FairMutex<T>);
