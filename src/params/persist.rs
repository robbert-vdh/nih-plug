//! Traits and helpers for persistent fields. See the [`Params`][super::Params] trait for more
//! information.

use std::sync::Arc;

/// Re-export for use in the [`Params`][super::Params] proc-macro.
pub use serde_json::from_str as deserialize_field;
/// Re-export for use in the [`Params`][super::Params] proc-macro.
pub use serde_json::to_string as serialize_field;

/// Handles the functionality needed for persisting a non-parameter fields in a plugin's state.
/// These types can be used with [`Params`][super::Params]' `#[persist = "..."]` attributes.
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

/// Wrapper for implementing an `Arc<I>` wrapper for an `I: PersistentField<T>`. Having both options
/// gives you more flexibility in data can be shared with an editor.
macro_rules! impl_persistent_arc {
    ($ty:ty, T) => {
        impl<'a, T> PersistentField<'a, T> for Arc<$ty>
        where
            T: serde::Serialize + serde::Deserialize<'a> + Send + Sync,
        {
            fn set(&self, new_value: T) {
                PersistentField::set(self.as_ref(), new_value);
            }
            fn map<F, R>(&self, f: F) -> R
            where
                F: Fn(&T) -> R,
            {
                self.as_ref().map(f)
            }
        }
    };

    ($ty:ty, T: $($bounds:tt)*) => {
        impl<'a, T> PersistentField<'a, T> for Arc<$ty>
        where
            T: $($bounds)*,
        {
            fn set(&self, new_value: T) {
                self.as_ref().set(new_value);
            }
            fn map<F, R>(&self, f: F) -> R
            where
                F: Fn(&T) -> R,
            {
                self.as_ref().map(f)
            }
        }
    };
    ($ty:ty, $inner_ty:ty) => {
        impl<'a> PersistentField<'a, $inner_ty> for Arc<$ty> {
            fn set(&self, new_value: $inner_ty) {
                self.as_ref().set(new_value);
            }
            fn map<F, R>(&self, f: F) -> R
            where
                F: Fn(&$inner_ty) -> R,
            {
                self.as_ref().map(f)
            }
        }
    };
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
impl_persistent_arc!(std::sync::RwLock<T>, T);

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
impl_persistent_arc!(parking_lot::RwLock<T>, T);

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
impl_persistent_arc!(std::sync::Mutex<T>, T);

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
impl_persistent_arc!(atomic_refcell::AtomicRefCell<T>, T);

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

        impl_persistent_arc!($ty, T);
    };
}

impl_persistent_field_parking_lot_mutex!(parking_lot::Mutex<T>);
impl_persistent_field_parking_lot_mutex!(parking_lot::FairMutex<T>);

macro_rules! impl_persistent_atomic {
    ($ty:ty, $inner_ty:ty) => {
        impl PersistentField<'_, $inner_ty> for $ty {
            fn set(&self, new_value: $inner_ty) {
                self.store(new_value, std::sync::atomic::Ordering::SeqCst);
            }
            fn map<F, R>(&self, f: F) -> R
            where
                F: Fn(&$inner_ty) -> R,
            {
                f(&self.load(std::sync::atomic::Ordering::SeqCst))
            }
        }

        impl_persistent_arc!($ty, $inner_ty);
    };
}

impl_persistent_atomic!(std::sync::atomic::AtomicBool, bool);
impl_persistent_atomic!(std::sync::atomic::AtomicI8, i8);
impl_persistent_atomic!(std::sync::atomic::AtomicI16, i16);
impl_persistent_atomic!(std::sync::atomic::AtomicI32, i32);
impl_persistent_atomic!(std::sync::atomic::AtomicI64, i64);
impl_persistent_atomic!(std::sync::atomic::AtomicIsize, isize);
impl_persistent_atomic!(std::sync::atomic::AtomicU8, u8);
impl_persistent_atomic!(std::sync::atomic::AtomicU16, u16);
impl_persistent_atomic!(std::sync::atomic::AtomicU32, u32);
impl_persistent_atomic!(std::sync::atomic::AtomicU64, u64);
impl_persistent_atomic!(std::sync::atomic::AtomicUsize, usize);
impl_persistent_atomic!(atomic_float::AtomicF32, f32);
impl_persistent_atomic!(atomic_float::AtomicF64, f64);

impl<'a, T> PersistentField<'a, T> for crossbeam::atomic::AtomicCell<T>
where
    T: serde::Serialize + serde::Deserialize<'a> + Copy + Send,
{
    fn set(&self, new_value: T) {
        self.store(new_value);
    }
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&T) -> R,
    {
        f(&self.load())
    }
}
impl_persistent_arc!(crossbeam::atomic::AtomicCell<T>,
                     T: serde::Serialize + serde::Deserialize<'a> + Copy + Send);

/// Can be used with the `#[serde(with = "nih_plug::params::internals::serialize_atomic_cell")]`
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
