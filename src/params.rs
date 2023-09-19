//! NIH-plug can handle floating point, integer, boolean, and enum parameters. Parameters are
//! managed by creating a struct deriving the [`Params`][Params] trait containing fields
//! for those parameter types, and then returning a reference to that object from your
//! [`Plugin::params()`][crate::prelude::Plugin::params()] method. See the `Params` trait for more
//! information.

use std::collections::BTreeMap;
use std::fmt::{Debug, Display};
use std::sync::Arc;

use self::internals::ParamPtr;

// The proc-macro for deriving `Params`
pub use nih_plug_derive::Params;

// Parameter types
mod boolean;
pub mod enums;
mod float;
mod integer;

pub mod internals;
pub mod persist;
pub mod range;
pub mod smoothing;

pub use boolean::BoolParam;
pub use enums::EnumParam;
pub use float::FloatParam;
pub use integer::IntParam;

bitflags::bitflags! {
    /// Flags for controlling a parameter's behavior.
    #[repr(transparent)]
    #[derive(Default)]
    pub struct ParamFlags: u32 {
        /// When applied to a [`BoolParam`], this will cause the parameter to be linked to the
        /// host's bypass control. Only a single parameter can be marked as a bypass parameter. If
        /// you don't have a bypass parameter, then NIH-plug will add one for you. You will need to
        /// implement this yourself if your plugin introduces latency.
        const BYPASS = 1 << 0;
        /// The parameter cannot be changed from an automation lane. The parameter can however still
        /// be manually changed by the user from either the plugin's own GUI or from the host's
        /// generic UI.
        const NON_AUTOMATABLE = 1 << 1;
        /// Hides the parameter in the host's generic UI for this plugin. This also implies
        /// `NON_AUTOMATABLE`. Setting this does not prevent you from changing the parameter in the
        /// plugin's editor GUI.
        const HIDDEN = 1 << 2;
        /// Don't show this parameter when generating a generic UI for the plugin using one of
        /// NIH-plug's generic UI widgets.
        const HIDE_IN_GENERIC_UI = 1 << 3;
    }
}

// See https://rust-lang.github.io/api-guidelines/future-proofing.html for more information
mod sealed {
    /// Dummy trait to prevent [`Param`] from being implemented outside of NIH-plug. This is not
    /// possible because of the way `ParamPtr` works, so it's best to just make it flat out impossible.
    pub trait Sealed {}
}
pub(crate) use sealed::Sealed;

/// Describes a single parameter of any type. Most parameter implementations also have a field
/// called `value` that and a field called `smoothed`. The former stores the latest unsmoothed
/// value, and the latter can be used to access the smoother. These two fields should be used in DSP
/// code to either get the parameter's current (smoothed) value. In UI code the getters from this
/// trait should be used instead.
///
/// # Sealed
///
/// This trait cannot be implemented outside of NIH-plug itself. If you want to create new
/// abstractions around parameters, consider wrapping them in a struct instead. Then use the
/// `#[nested(id_prefix = "foo")]` syntax from the [`Params`] trait to reuse that wrapper in
/// multiple places.
pub trait Param: Display + Debug + sealed::Sealed {
    /// The plain parameter type.
    type Plain: PartialEq;

    /// Get the human readable name for this parameter.
    fn name(&self) -> &str;

    /// Get the unit label for this parameter, if any.
    fn unit(&self) -> &'static str;

    /// Get this parameter's polyphonic modulation ID. If this is set for a parameter in a CLAP
    /// plugin, then polyphonic modulation will be enabled for that parameter. Polyphonic modulation
    /// is communicated to the plugin through
    /// [`NoteEvent::PolyModulation`][crate::prelude::NoteEvent::PolyModulation] and
    /// [`NoteEvent::MonoAutomation`][crate::prelude::NoteEvent::MonoAutomation] events. See the
    /// documentation on those events for more information.
    ///
    /// # Important
    ///
    /// After enabling polyphonic modulation, the plugin **must** start sending
    /// [`NoteEvent::VoiceTerminated`][crate::prelude::NoteEvent::VoiceTerminated] events to the
    /// host when a voice has fully ended. This allows the host to reuse its modulation resources.
    fn poly_modulation_id(&self) -> Option<u32>;

    /// Get the unnormalized value for this parameter.
    fn modulated_plain_value(&self) -> Self::Plain;

    /// Get the normalized `[0, 1]` value for this parameter.
    fn modulated_normalized_value(&self) -> f32;

    /// Get the unnormalized value for this parameter before any (monophonic) modulation coming from
    /// the host has been applied. If the host is not currently modulating this parameter than this
    /// will be the same as [`modulated_plain_value()`][Self::modulated_plain_value()]. This may be
    /// useful for displaying modulation differently in plugin GUIs. Right now only CLAP plugins in
    /// Bitwig Studio use modulation.
    fn unmodulated_plain_value(&self) -> Self::Plain;

    /// Get the normalized `[0, 1]` value for this parameter before any (monophonic) modulation
    /// coming from the host has been applied. If the host is not currently modulating this
    /// parameter than this will be the same as
    /// [`modulated_normalized_value()`][Self::modulated_normalized_value()]. This may be useful for
    /// displaying modulation differently in plugin GUIs. Right now only CLAP plugins in Bitwig
    /// Studio use modulation.
    fn unmodulated_normalized_value(&self) -> f32;

    /// Get the unnormalized default value for this parameter.
    fn default_plain_value(&self) -> Self::Plain;

    /// Get the normalized `[0, 1]` default value for this parameter.
    #[inline]
    fn default_normalized_value(&self) -> f32 {
        self.preview_normalized(self.default_plain_value())
    }

    /// Get the number of steps for this parameter, if it is discrete. Used for the host's generic
    /// UI.
    fn step_count(&self) -> Option<usize>;

    /// Returns the previous step from a specific value for this parameter. This can be the same as
    /// `from` if the value is at the start of its range. This is mainly used for scroll wheel
    /// interaction in plugin GUIs. When the parameter is not discrete then a step should cover one
    /// hundredth of the normalized range instead.
    ///
    /// If `finer` is true, then the step size should be decreased if the parameter is continuous.
    fn previous_step(&self, from: Self::Plain, finer: bool) -> Self::Plain;

    /// Returns the next step from a specific value for this parameter. This can be the same as
    /// `from` if the value is at the end of its range. This is mainly used for scroll wheel
    /// interaction in plugin GUIs. When the parameter is not discrete then a step should cover one
    /// hundredth of the normalized range instead.
    ///
    /// If `finer` is true, then the step size should be decreased if the parameter is continuous.
    fn next_step(&self, from: Self::Plain, finer: bool) -> Self::Plain;

    /// The same as [`previous_step()`][Self::previous_step()], but for normalized values. This is
    /// mostly useful for GUI widgets.
    fn previous_normalized_step(&self, from: f32, finer: bool) -> f32 {
        self.preview_normalized(self.previous_step(self.preview_plain(from), finer))
    }

    /// The same as [`next_step()`][Self::next_step()], but for normalized values. This is mostly
    /// useful for GUI widgets.
    fn next_normalized_step(&self, from: f32, finer: bool) -> f32 {
        self.preview_normalized(self.next_step(self.preview_plain(from), finer))
    }

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
    /// wrappers. This **does** snap to step sizes for continuous parameters (i.e. [`FloatParam`]).
    fn preview_plain(&self, normalized: f32) -> Self::Plain;

    /// Get the plain, unnormalized value for this parameter after polyphonic modulation has been
    /// applied. This is a convenience method for calling [`preview_plain()`][Self::preview_plain()]
    /// with `unmodulated_normalized_value() + normalized_offset`.
    #[inline]
    fn preview_modulated(&self, normalized_offset: f32) -> Self::Plain {
        self.preview_plain(self.unmodulated_normalized_value() + normalized_offset)
    }

    /// Flags to control the parameter's behavior. See [`ParamFlags`].
    fn flags(&self) -> ParamFlags;

    /// Internal implementation detail for implementing [`Params`][Params]. This should
    /// not be used directly.
    fn as_ptr(&self) -> internals::ParamPtr;
}

/// Contains the setters for parameters. These should not be exposed to plugins to avoid confusion.
pub(crate) trait ParamMut: Param {
    /// Set this parameter based on a plain, unnormalized value. This does not snap to step sizes
    /// for continuous parameters (i.e. [`FloatParam`]). If
    /// [`modulate_value()`][Self::modulate_value()] has previously been called with a non zero
    /// value then this offset is taken into account to form the effective value.
    ///
    /// Returns whether or not the value has changed. Any parameter callbacks are only run the value
    /// has actually changed.
    ///
    /// This does **not** update the smoother.
    fn set_plain_value(&self, plain: Self::Plain) -> bool;

    /// Set this parameter based on a normalized value. The normalized value will be snapped to the
    /// step size for continuous parameters (i.e. [`FloatParam`]). If
    /// [`modulate_value()`][Self::modulate_value()] has previously been called with a non zero
    /// value then this offset is taken into account to form the effective value.
    ///
    /// Returns whether or not the value has changed. Any parameter callbacks are only run the value
    /// has actually changed.
    ///
    /// This does **not** update the smoother.
    fn set_normalized_value(&self, normalized: f32) -> bool;

    /// Add a modulation offset to the value's unmodulated value. This value sticks until this
    /// function is called again with a 0.0 value. Out of bound values will be clamped to the
    /// parameter's range. The normalized value will be snapped to the step size for continuous
    /// parameters (i.e. [`FloatParam`]).
    ///
    /// Returns whether or not the value has changed. Any parameter callbacks are only run the value
    /// has actually changed.
    ///
    /// This does **not** update the smoother.
    fn modulate_value(&self, modulation_offset: f32) -> bool;

    /// Update the smoother state to point to the current value. Also used when initializing and
    /// restoring a plugin so everything is in sync. In that case the smoother should completely
    /// reset to the current value.
    fn update_smoother(&self, sample_rate: f32, reset: bool);
}

/// Describes a struct containing parameters and other persistent fields.
///
/// # Deriving `Params` and `#[id = "stable"]`
///
/// This trait can be derived on a struct containing [`FloatParam`] and other parameter fields by
/// adding `#[derive(Params)]`. When deriving this trait, any of those parameter fields should have
/// the `#[id = "stable"]` attribute, where `stable` is an up to 6 character long string (to avoid
/// collisions) that will be used to identify the parameter internally so you can safely move it
/// around and rename the field without breaking compatibility with old presets.
///
/// ## `#[persist = "key"]`
///
/// The struct can also contain other fields that should be persisted along with the rest of the
/// preset data. These fields should be [`PersistentField`][persist::PersistentField]s annotated
/// with the `#[persist = "key"]` attribute containing types that can be serialized and deserialized
/// with [Serde](https://serde.rs/).
///
/// ## `#[nested]`, `#[nested(group_name = "group name")]`
///
/// Finally, the `Params` object may include parameters from other objects. Setting a group name is
/// optional, but some hosts can use this information to display the parameters in a tree structure.
/// Parameter IDs and persisting keys still need to be **unique** when using nested parameter
/// structs.
///
/// Take a look at the example gain example plugin to see how this is used.
///
/// ## `#[nested(id_prefix = "foo", group_name = "Foo")]`
///
/// Adding this attribute to a `Params` sub-object works similarly to the regular `#[nested]`
/// attribute, but it also adds an ID to all parameters from the nested object. If a parameter in
/// the nested nested object normally has parameter ID `bar`, the parameter's ID will now be renamed
/// to `foo_bar`. The same thing happens with persistent field keys to support multiple copies of
/// the field. _This makes it possible to reuse the same parameter struct with different names and
/// parameter indices._
///
/// ## `#[nested(array, group_name = "Foo")]`
///
/// This can be applied to an array-like data structure and it works similar to a `nested` attribute
/// with an `id_name`, except that it will iterate over the array and create unique indices for all
/// nested parameters. If the nested parameters object has a parameter called `bar`, then that
/// parameter will belong to the group `Foo {array_index + 1}`, and it will have the renamed
/// parameter ID `bar_{array_index + 1}`. The same thing applies to persistent field keys.
///
/// # Safety
///
/// This implementation is safe when using from the wrapper because the plugin's returned `Params`
/// object lives in an `Arc`, and the wrapper also holds a reference to this `Arc`.
pub unsafe trait Params: 'static + Send + Sync {
    /// Create a mapping from unique parameter IDs to parameter pointers along with the name of the
    /// group/unit/module they are in, as a `(param_id, param_ptr, group)` triple. The order of the
    /// `Vec` determines the display order in the (host's) generic UI. The group name is either an
    /// empty string for top level parameters, or a slash/delimited `"group name 1/Group Name 2"` if
    /// this `Params` object contains nested child objects. All components of a group path must
    /// exist or you may encounter panics. The derive macro does this for every parameter field
    /// marked with `#[id = "stable"]`, and it also inlines all fields from nested child `Params`
    /// structs marked with `#[nested(...)]` while prefixing that group name before the parameter's
    /// original group name. Dereferencing the pointers stored in the values is only valid as long
    /// as this object is valid.
    ///
    /// # Note
    ///
    /// This uses `String` even though for the `Params` derive macro `&'static str` would have been
    /// fine to be able to support custom reusable Params implementations.
    fn param_map(&self) -> Vec<(String, ParamPtr, String)>;

    /// Serialize all fields marked with `#[persist = "stable_name"]` into a hash map containing
    /// JSON-representations of those fields so they can be written to the plugin's state and
    /// recalled later. This uses [`persist::serialize_field()`] under the hood.
    fn serialize_fields(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// Restore all fields marked with `#[persist = "stable_name"]` from a hashmap created by
    /// [`serialize_fields()`][Self::serialize_fields()]. All of these fields should be wrapped in a
    /// [`persist::PersistentField`] with thread safe interior mutability, like an `RwLock` or a
    /// `Mutex`. This gets called when the plugin's state is being restored. This uses
    /// [`persist::deserialize_field()`] under the hood.
    #[allow(unused_variables)]
    fn deserialize_fields(&self, serialized: &BTreeMap<String, String>) {}
}

/// This may be useful when building generic UIs using nested `Params` objects.
unsafe impl<P: Params> Params for Arc<P> {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        self.as_ref().param_map()
    }

    fn serialize_fields(&self) -> BTreeMap<String, String> {
        self.as_ref().serialize_fields()
    }

    fn deserialize_fields(&self, serialized: &BTreeMap<String, String>) {
        self.as_ref().deserialize_fields(serialized)
    }
}
