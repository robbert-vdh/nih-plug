//! Implementation details for the parameter management.

use super::{Param, ParamFlags, ParamMut};

/// Re-export for use in the [`Params`] proc-macro.
pub use serde_json::from_str as deserialize_field;
/// Re-export for use in the [`Params`] proc-macro.
pub use serde_json::to_string as serialize_field;

/// Can be used with the `#[serde(with = "nih_plug::param::internals::serialize_atomic_cell")]`
/// attribute to serialize `AtomicCell<T>`s.
pub mod serialize_atomic_cell {
    use crossbeam::atomic::AtomicCell;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, T>(cell: &AtomicCell<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize + Copy,
    {
        cell.load().serialize(serializer)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<AtomicCell<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Copy,
    {
        T::deserialize(deserializer).map(AtomicCell::new)
    }
}

/// Internal pointers to parameters. This is an implementation detail used by the wrappers for type
/// erasure.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ParamPtr {
    FloatParam(*const super::FloatParam),
    IntParam(*const super::IntParam),
    BoolParam(*const super::BoolParam),
    /// Since we can't encode the actual enum here, this inner parameter struct contains all of the
    /// relevant information from the enum so it can be type erased.
    EnumParam(*const super::enums::EnumParamInner),
}

// These pointers only point to fields on structs kept in an `Arc<dyn Params>`, and the caller
// always needs to make sure that dereferencing them is safe. To do that the plugin wrappers will
// keep references to that `Arc` around for the entire lifetime of the plugin.
unsafe impl Send for ParamPtr {}
unsafe impl Sync for ParamPtr {}

/// Handles the functionality needed for persisting a non-parameter fields in a plugin's state.
/// These types can be used with [`Params`]' `#[persist = "..."]` attributes.
///
/// This should be implemented for some type with interior mutability containing a `T`.
//
// TODO: Modifying these fields (or any parameter for that matter) should mark the plugin's state
//       as dirty.
pub trait PersistentField<'a, T>: Send + Sync
where
    T: serde::Serialize + serde::Deserialize<'a>,
{
    /// Update the stored `T` value using interior mutability.
    fn set(&self, new_value: T);

    /// Get a reference to the stored `T` value, and apply a function to it. This is used to
    /// serialize the `T` value.
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&T) -> R;
}

/// Generate a [`ParamPtr`] function that forwards the function call to the underlying `Param`. We
/// can't have an `.as_param()` function since the return type would differ depending on the
/// underlying parameter type, so instead we need to type erase all of the functions individually.
macro_rules! param_ptr_forward(
    ($vis:vis unsafe fn $method:ident(&self $(, $arg_name:ident: $arg_ty:ty)*) $(-> $ret:ty)?) => {
        /// Calls the corresponding method on the underlying [`Param`] object.
        ///
        /// # Safety
        ///
        /// Calling this function is only safe as long as the object this [`ParamPtr`] was created
        /// for is still alive.
        $vis unsafe fn $method(&self $(, $arg_name: $arg_ty)*) $(-> $ret)? {
            match self {
                ParamPtr::FloatParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::IntParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::BoolParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::EnumParam(p) => (**p).$method($($arg_name),*),
            }
        }
    };
    // XXX: Is there a way to combine these two? Hygienic macros don't let you call `&self` without
    //      it being defined in the macro.
    ($vis:vis unsafe fn $method:ident(&mut self $(, $arg_name:ident: $arg_ty:ty)*) $(-> $ret:ty)?) => {
        /// Calls the corresponding method on the underlying [`Param`] object.
        ///
        /// # Safety
        ///
        /// Calling this function is only safe as long as the object this [`ParamPtr`] was created
        /// for is still alive.
        $vis unsafe fn $method(&mut self $(, $arg_name: $arg_ty)*) $(-> $ret)? {
            match self {
                ParamPtr::FloatParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::IntParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::BoolParam(p) => (**p).$method($($arg_name),*),
                ParamPtr::EnumParam(p) => (**p).$method($($arg_name),*),
            }
        }
    };
);

impl ParamPtr {
    param_ptr_forward!(pub unsafe fn name(&self) -> &str);
    param_ptr_forward!(pub unsafe fn unit(&self) -> &'static str);
    param_ptr_forward!(pub unsafe fn poly_modulation_id(&self) -> Option<u32>);
    param_ptr_forward!(pub unsafe fn normalized_value(&self) -> f32);
    param_ptr_forward!(pub unsafe fn unmodulated_normalized_value(&self) -> f32);
    param_ptr_forward!(pub unsafe fn default_normalized_value(&self) -> f32);
    param_ptr_forward!(pub unsafe fn step_count(&self) -> Option<usize>);
    param_ptr_forward!(pub unsafe fn previous_normalized_step(&self, from: f32) -> f32);
    param_ptr_forward!(pub unsafe fn next_normalized_step(&self, from: f32) -> f32);
    param_ptr_forward!(pub unsafe fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String);
    param_ptr_forward!(pub unsafe fn string_to_normalized_value(&self, string: &str) -> Option<f32>);
    param_ptr_forward!(pub unsafe fn flags(&self) -> ParamFlags);

    param_ptr_forward!(pub(crate) unsafe fn set_normalized_value(&self, normalized: f32));
    param_ptr_forward!(pub(crate) unsafe fn modulate_value(&self, modulation_offset: f32));
    param_ptr_forward!(pub(crate) unsafe fn update_smoother(&self, sample_rate: f32, reset: bool));

    // These functions involve casts since the plugin formats only do floating point types, so we
    // can't generate them with the macro:

    /// Get the parameter's plain, unnormalized value, converted to a float. Useful in conjunction
    /// with [`preview_plain()`][Self::preview_plain()] to compare a snapped discrete value to a
    /// parameter's current snapped value without having to do a back and forth conversion using
    /// normalized values.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn plain_value(&self) -> f32 {
        match self {
            ParamPtr::FloatParam(p) => (**p).plain_value(),
            ParamPtr::IntParam(p) => (**p).plain_value() as f32,
            ParamPtr::BoolParam(p) => (**p).normalized_value(),
            ParamPtr::EnumParam(p) => (**p).plain_value() as f32,
        }
    }

    /// Get the parameter's plain, unnormalized value, converted to a float, before any monophonic
    /// host modulation has been applied. This is useful for handling modulated parameters for CLAP
    /// plugins in Bitwig in a way where the actual parameter does not move in the GUI while the
    /// parameter is being modulated. You can also use this to show the difference between the
    /// unmodulated value and the current value. Useful in conjunction with
    /// [`preview_plain()`][Self::preview_plain()] to compare a snapped discrete value to a
    /// parameter's current snapped value without having to do a back and forth conversion using
    /// normalized values.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn unmodulated_plain_value(&self) -> f32 {
        match self {
            ParamPtr::FloatParam(p) => (**p).unmodulated_plain_value(),
            ParamPtr::IntParam(p) => (**p).unmodulated_plain_value() as f32,
            ParamPtr::BoolParam(p) => (**p).unmodulated_normalized_value(),
            ParamPtr::EnumParam(p) => (**p).unmodulated_plain_value() as f32,
        }
    }

    /// Get the parameter's default value as a plain, unnormalized value, converted to a float.
    ///
    /// # Safety
    ///
    /// Calling this function is only safe as long as the object this `ParamPtr` was created for is
    /// still alive.
    pub unsafe fn default_plain_value(&self) -> f32 {
        match self {
            ParamPtr::FloatParam(p) => (**p).default_plain_value(),
            ParamPtr::IntParam(p) => (**p).default_plain_value() as f32,
            ParamPtr::BoolParam(p) => (**p).normalized_value(),
            ParamPtr::EnumParam(p) => (**p).default_plain_value() as f32,
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
        match self {
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
        match self {
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
