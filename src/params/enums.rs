//! Enum parameters. `enum` is a keyword, so `enums` it is.

use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::sync::Arc;

use super::internals::ParamPtr;
use super::range::IntRange;
use super::{IntParam, Param, ParamFlags, ParamMut};

// Re-export the derive macro
pub use nih_plug_derive::Enum;

/// An enum usable with `EnumParam`. This trait can be derived. Variants are identified either by a
/// stable _id_ (see below), or if those are not set then they are identifier by their **declaration
/// order**. If you don't provide IDs then you can freely rename the variant names, but reordering
/// them will break compatibility with existing presets. The variant's name is used as the display
/// name by default. If you want to override this, for instance, because it needs to contain spaces,
/// then you can use the `#[name = "..."]` attribute:
///
/// ```ignore
/// #[derive(Enum)]
/// enum Foo {
///     Bar,
///     Baz,
///     #[name = "Contains Spaces"]
///     ContainsSpaces,
/// }
/// ```
///
/// IDs can be added by adding the `#[id = "..."]` attribute to each variant:
///
/// ```ignore
/// #[derive(Enum)]
/// enum Foo {
///     #[id = "bar"],
///     Bar,
///     #[id = "baz"],
///     Baz,
///     #[id = "contains-spaces"],
///     #[name = "Contains Spaces"]
///     ContainsSpaces,
/// }
/// ```
///
/// You can safely move from not using IDs to using IDs without breaking patches, but you cannot go
/// back to not using IDs after that.
pub trait Enum {
    /// The human readable names for the variants. These are displayed in the GUI or parameter list,
    /// and also used for parsing text back to a parameter value. The length of this slice
    /// determines how many variants there are.
    fn variants() -> &'static [&'static str];

    /// Optional identifiers for each variant. This makes it possible to reorder variants while
    /// maintaining save compatibility (automation will still break of course). The length of this
    /// slice needs to be equal to [`variants()`][Self::variants()].
    fn ids() -> Option<&'static [&'static str]>;

    /// Get the variant index (which may not be the same as the discriminator) corresponding to the
    /// active variant. The index needs to correspond to the name in
    /// [`variants()`][Self::variants()].
    fn to_index(self) -> usize;

    /// Get the variant corresponding to the variant with the same index in
    /// [`variants()`][Self::variants()]. This must always return a value. If the index is out of
    /// range, return the first variant.
    fn from_index(index: usize) -> Self;
}

/// An [`IntParam`]-backed categorical parameter that allows convenient conversion to and from a
/// simple enum. This enum must derive the re-exported [Enum] trait. Check the trait's documentation
/// for more information on how this works.
pub struct EnumParam<T: Enum + PartialEq> {
    /// A type-erased version of this parameter so the wrapper can do its thing without needing to
    /// know about `T`.
    inner: EnumParamInner,

    /// `T` is only used on the plugin side to convert back to an enum variant. Internally
    /// everything works through the variants field on [`EnumParamInner`].
    _marker: PhantomData<T>,
}

/// The type-erased internals for [`EnumParam`] so that the wrapper can interact with it. Acts like
/// an [`IntParam`] but with different conversions from strings to values.
pub struct EnumParamInner {
    /// The integer parameter backing this enum parameter.
    pub(crate) inner: IntParam,
    /// The human readable variant names, obtained from [Enum::variants()].
    variants: &'static [&'static str],
    /// Stable identifiers for the enum variants, obtained from [Enum::ids()]. These are optional,
    /// but if they are set (they're either not set for any variant, or set for all variants) then
    /// these identifiers are used when saving enum parameter values to the state. Otherwise the
    /// index is used.
    ids: Option<&'static [&'static str]>,
}

impl<T: Enum + PartialEq> Display for EnumParam<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.inner, f)
    }
}

impl Display for EnumParamInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.variants[self.inner.modulated_plain_value() as usize]
        )
    }
}

impl<T: Enum + PartialEq> Debug for EnumParam<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.inner, f)
    }
}

impl Debug for EnumParamInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // This uses the above `Display` instance to show the value
        if self.inner.modulated_plain_value() != self.inner.unmodulated_plain_value() {
            write!(f, "{}: {} (modulated)", &self.name(), &self)
        } else {
            write!(f, "{}: {}", &self.name(), &self)
        }
    }
}

// `Params` can not be implemented outside of NIH-plug itself because `ParamPtr` is also closed
impl<T: Enum + PartialEq> super::Sealed for EnumParam<T> {}

impl<T: Enum + PartialEq> Param for EnumParam<T> {
    type Plain = T;

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn unit(&self) -> &'static str {
        self.inner.unit()
    }

    fn poly_modulation_id(&self) -> Option<u32> {
        self.inner.poly_modulation_id()
    }

    #[inline]
    fn modulated_plain_value(&self) -> Self::Plain {
        T::from_index(self.inner.modulated_plain_value() as usize)
    }

    #[inline]
    fn modulated_normalized_value(&self) -> f32 {
        self.inner.modulated_normalized_value()
    }

    #[inline]
    fn unmodulated_plain_value(&self) -> Self::Plain {
        T::from_index(self.inner.unmodulated_plain_value() as usize)
    }

    #[inline]
    fn unmodulated_normalized_value(&self) -> f32 {
        self.inner.unmodulated_normalized_value()
    }

    #[inline]
    fn default_plain_value(&self) -> Self::Plain {
        T::from_index(self.inner.default_plain_value() as usize)
    }

    fn step_count(&self) -> Option<usize> {
        self.inner.step_count()
    }

    fn previous_step(&self, from: Self::Plain, finer: bool) -> Self::Plain {
        T::from_index(self.inner.previous_step(T::to_index(from) as i32, finer) as usize)
    }

    fn next_step(&self, from: Self::Plain, finer: bool) -> Self::Plain {
        T::from_index(self.inner.next_step(T::to_index(from) as i32, finer) as usize)
    }

    fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String {
        self.inner
            .normalized_value_to_string(normalized, include_unit)
    }

    fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
        self.inner.string_to_normalized_value(string)
    }

    #[inline]
    fn preview_normalized(&self, plain: Self::Plain) -> f32 {
        self.inner.preview_normalized(T::to_index(plain) as i32)
    }

    #[inline]
    fn preview_plain(&self, normalized: f32) -> Self::Plain {
        T::from_index(self.inner.preview_plain(normalized) as usize)
    }

    fn flags(&self) -> ParamFlags {
        self.inner.flags()
    }

    fn as_ptr(&self) -> ParamPtr {
        self.inner.as_ptr()
    }
}

// `Params` can not be implemented outside of NIH-plug itself because `ParamPtr` is also closed
impl super::Sealed for EnumParamInner {}

impl Param for EnumParamInner {
    type Plain = i32;

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn unit(&self) -> &'static str {
        ""
    }

    fn poly_modulation_id(&self) -> Option<u32> {
        self.inner.poly_modulation_id()
    }

    #[inline]
    fn modulated_plain_value(&self) -> Self::Plain {
        self.inner.modulated_plain_value()
    }

    #[inline]
    fn modulated_normalized_value(&self) -> f32 {
        self.inner.modulated_normalized_value()
    }

    #[inline]
    fn default_plain_value(&self) -> Self::Plain {
        self.inner.default_plain_value()
    }

    #[inline]
    fn unmodulated_plain_value(&self) -> Self::Plain {
        self.inner.unmodulated_plain_value()
    }

    #[inline]
    fn unmodulated_normalized_value(&self) -> f32 {
        self.inner.unmodulated_normalized_value()
    }

    fn step_count(&self) -> Option<usize> {
        Some(self.len() - 1)
    }

    fn previous_step(&self, from: Self::Plain, finer: bool) -> Self::Plain {
        self.inner.previous_step(from, finer)
    }

    fn next_step(&self, from: Self::Plain, finer: bool) -> Self::Plain {
        self.inner.next_step(from, finer)
    }

    fn normalized_value_to_string(&self, normalized: f32, _include_unit: bool) -> String {
        let index = self.preview_plain(normalized);
        self.variants[index as usize].to_string()
    }

    fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
        let string = string.trim();
        self.variants
            .iter()
            .position(|variant| variant == &string)
            .map(|idx| self.preview_normalized(idx as i32))
    }

    #[inline]
    fn preview_normalized(&self, plain: Self::Plain) -> f32 {
        self.inner.preview_normalized(plain)
    }

    #[inline]
    fn preview_plain(&self, normalized: f32) -> Self::Plain {
        self.inner.preview_plain(normalized)
    }

    fn flags(&self) -> ParamFlags {
        self.inner.flags()
    }

    fn as_ptr(&self) -> ParamPtr {
        ParamPtr::EnumParam(self as *const EnumParamInner as *mut EnumParamInner)
    }
}

impl<T: Enum + PartialEq> ParamMut for EnumParam<T> {
    fn set_plain_value(&self, plain: Self::Plain) -> bool {
        self.inner.set_plain_value(T::to_index(plain) as i32)
    }

    fn set_normalized_value(&self, normalized: f32) -> bool {
        self.inner.set_normalized_value(normalized)
    }

    fn modulate_value(&self, modulation_offset: f32) -> bool {
        self.inner.modulate_value(modulation_offset)
    }

    fn update_smoother(&self, sample_rate: f32, reset: bool) {
        self.inner.update_smoother(sample_rate, reset)
    }
}

impl ParamMut for EnumParamInner {
    fn set_plain_value(&self, plain: Self::Plain) -> bool {
        self.inner.set_plain_value(plain)
    }

    fn set_normalized_value(&self, normalized: f32) -> bool {
        self.inner.set_normalized_value(normalized)
    }

    fn modulate_value(&self, modulation_offset: f32) -> bool {
        self.inner.modulate_value(modulation_offset)
    }

    fn update_smoother(&self, sample_rate: f32, reset: bool) {
        self.inner.update_smoother(sample_rate, reset)
    }
}

impl<T: Enum + PartialEq + 'static> EnumParam<T> {
    /// Build a new [Self]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: impl Into<String>, default: T) -> Self {
        let variants = T::variants();
        let ids = T::ids();

        Self {
            inner: EnumParamInner {
                inner: IntParam::new(
                    name,
                    T::to_index(default) as i32,
                    IntRange::Linear {
                        min: 0,
                        max: variants.len() as i32 - 1,
                    },
                ),
                variants,
                ids,
            },
            _marker: PhantomData,
        }
    }

    /// Get the active enum variant.
    #[inline]
    pub fn value(&self) -> T {
        self.modulated_plain_value()
    }

    /// Enable polyphonic modulation for this parameter. The ID is used to uniquely identify this
    /// parameter in [`NoteEvent::PolyModulation`][crate::prelude::NoteEvent::PolyModulation]
    /// events, and must thus be unique between _all_ polyphonically modulatable parameters. See the
    /// event's documentation on how to use polyphonic modulation. Also consider configuring the
    /// [`ClapPlugin::CLAP_POLY_MODULATION_CONFIG`][crate::prelude::ClapPlugin::CLAP_POLY_MODULATION_CONFIG]
    /// constant when enabling this.
    ///
    /// # Important
    ///
    /// After enabling polyphonic modulation, the plugin **must** start sending
    /// [`NoteEvent::VoiceTerminated`][crate::prelude::NoteEvent::VoiceTerminated] events to the
    /// host when a voice has fully ended. This allows the host to reuse its modulation resources.
    pub fn with_poly_modulation_id(mut self, id: u32) -> Self {
        self.inner.inner = self.inner.inner.with_poly_modulation_id(id);
        self
    }

    /// Run a callback whenever this parameter's value changes. The argument passed to this function
    /// is the parameter's new value. This should not do anything expensive as it may be called
    /// multiple times in rapid succession, and it can be run from both the GUI and the audio
    /// thread.
    pub fn with_callback(mut self, callback: Arc<dyn Fn(T) + Send + Sync>) -> Self {
        self.inner.inner = self.inner.inner.with_callback(Arc::new(move |value| {
            callback(T::from_index(value as usize))
        }));
        self
    }

    /// Mark the parameter as non-automatable. This means that the parameter cannot be changed from
    /// an automation lane. The parameter can however still be manually changed by the user from
    /// either the plugin's own GUI or from the host's generic UI.
    pub fn non_automatable(mut self) -> Self {
        self.inner.inner = self.inner.inner.non_automatable();
        self
    }

    /// Hide the parameter in the host's generic UI for this plugin. This also implies
    /// `NON_AUTOMATABLE`. Setting this does not prevent you from changing the parameter in the
    /// plugin's editor GUI.
    pub fn hide(mut self) -> Self {
        self.inner.inner = self.inner.inner.hide();
        self
    }

    /// Don't show this parameter when generating a generic UI for the plugin using one of
    /// NIH-plug's generic UI widgets.
    pub fn hide_in_generic_ui(mut self) -> Self {
        self.inner.inner = self.inner.inner.hide_in_generic_ui();
        self
    }
}

impl EnumParamInner {
    /// Get the number of variants for this enum.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.variants.len()
    }

    /// Get the stable ID for the parameter's current value according to
    /// [`unmodulated_plain_value()`][Param::unmodulated_plain_value()]. Returns `None` if this enum
    /// parameter doesn't have any stable IDs.
    pub fn unmodulated_plain_id(&self) -> Option<&'static str> {
        let ids = &self.ids?;

        // The `Enum` trait is supposed to make sure this contains enough values
        Some(ids[self.unmodulated_plain_value() as usize])
    }

    /// Set the parameter based on a serialized stable string identifier. Return whether the ID was
    /// known and the parameter was set.
    pub fn set_from_id(&self, id: &str) -> bool {
        match self
            .ids
            .and_then(|ids| ids.iter().position(|candidate| *candidate == id))
        {
            Some(index) => {
                self.set_plain_value(index as i32);
                true
            }
            None => false,
        }
    }
}
