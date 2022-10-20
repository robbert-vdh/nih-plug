//! Traits and helpers for persistent fields. See the [`Params`][super::Params] trait for more
//! information.

/// Re-export for use in the [`Params`] proc-macro.
pub use serde_json::from_str as deserialize_field;
/// Re-export for use in the [`Params`] proc-macro.
pub use serde_json::to_string as serialize_field;

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
