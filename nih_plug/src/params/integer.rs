//! Stepped integer parameters.

use atomic_float::AtomicF32;
use std::fmt::{Debug, Display};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use super::internals::ParamPtr;
use super::range::IntRange;
use super::smoothing::{Smoother, SmoothingStyle};
use super::{Param, ParamFlags, ParamMut};

/// A discrete integer parameter that's stored unnormalized. The range is used for the normalization
/// process.
pub struct IntParam {
    /// The field's current plain value, after monophonic modulation has been applied.
    value: AtomicI32,
    /// The field's current value normalized to the `[0, 1]` range.
    normalized_value: AtomicF32,
    /// The field's plain, unnormalized value before any monophonic automation coming from the host
    /// has been applied. This will always be the same as `value` for VST3 plugins.
    unmodulated_value: AtomicI32,
    /// The field's value normalized to the `[0, 1]` range before any monophonic automation coming
    /// from the host has been applied. This will always be the same as `value` for VST3 plugins.
    unmodulated_normalized_value: AtomicF32,
    /// A value in `[-1, 1]` indicating the amount of modulation applied to
    /// `unmodulated_normalized_`. This needs to be stored separately since the normalized values are
    /// clamped, and this value persists after new automation events.
    modulation_offset: AtomicF32,
    /// The field's default plain, unnormalized value.
    default: i32,
    /// An optional smoother that will automatically interpolate between the new automation values
    /// set by the host.
    pub smoothed: Smoother<i32>,

    /// Flags to control the parameter's behavior. See [`ParamFlags`].
    flags: ParamFlags,
    /// Optional callback for listening to value changes. The argument passed to this function is
    /// the parameter's new **plain** value. This should not do anything expensive as it may be
    /// called multiple times in rapid succession.
    ///
    /// To use this, you'll probably want to store an `Arc<Atomic*>` alongside the parameter in the
    /// parameters struct, move a clone of that `Arc` into this closure, and then modify that.
    ///
    /// TODO: We probably also want to pass the old value to this function.
    value_changed: Option<Arc<dyn Fn(i32) + Send + Sync>>,

    /// The distribution of the parameter's values.
    range: IntRange,
    /// The parameter's human readable display name.
    name: String,
    /// The parameter value's unit, added after `value_to_string` if that is set. NIH-plug will not
    /// automatically add a space before the unit.
    unit: &'static str,
    /// If this parameter has been marked as polyphonically modulatable, then this will be a unique
    /// integer identifying the parameter. Because this value is determined by the plugin itself,
    /// the plugin can easily map
    /// [`NoteEvent::PolyModulation`][crate::prelude::NoteEvent::PolyModulation] events to the
    /// correct parameter by pattern matching on a constant.
    poly_modulation_id: Option<u32>,
    /// Optional custom conversion function from a plain **unnormalized** value to a string.
    value_to_string: Option<Arc<dyn Fn(i32) -> String + Send + Sync>>,
    /// Optional custom conversion function from a string to a plain **unnormalized** value. If the
    /// string cannot be parsed, then this should return a `None`. If this happens while the
    /// parameter is being updated then the update will be canceled.
    ///
    /// The input string may or may not contain the unit, so you will need to be able to handle
    /// that.
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>>,
}

impl Display for IntParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.value_to_string {
            Some(func) => write!(f, "{}{}", func(self.value()), self.unit),
            _ => write!(f, "{}{}", self.value(), self.unit),
        }
    }
}

impl Debug for IntParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // This uses the above `Display` instance to show the value
        if self.modulated_plain_value() != self.unmodulated_plain_value() {
            write!(f, "{}: {} (modulated)", &self.name, &self)
        } else {
            write!(f, "{}: {}", &self.name, &self)
        }
    }
}

// `Params` can not be implemented outside of NIH-plug itself because `ParamPtr` is also closed
impl super::Sealed for IntParam {}

impl Param for IntParam {
    type Plain = i32;

    fn name(&self) -> &str {
        &self.name
    }

    fn unit(&self) -> &'static str {
        self.unit
    }

    fn poly_modulation_id(&self) -> Option<u32> {
        self.poly_modulation_id
    }

    #[inline]
    fn modulated_plain_value(&self) -> Self::Plain {
        self.value.load(Ordering::Relaxed)
    }

    #[inline]
    fn modulated_normalized_value(&self) -> f32 {
        self.normalized_value.load(Ordering::Relaxed)
    }

    #[inline]
    fn unmodulated_plain_value(&self) -> Self::Plain {
        self.unmodulated_value.load(Ordering::Relaxed)
    }

    #[inline]
    fn unmodulated_normalized_value(&self) -> f32 {
        self.unmodulated_normalized_value.load(Ordering::Relaxed)
    }

    #[inline]
    fn default_plain_value(&self) -> Self::Plain {
        self.default
    }

    fn step_count(&self) -> Option<usize> {
        Some(self.range.step_count())
    }

    fn previous_step(&self, from: Self::Plain, _finer: bool) -> Self::Plain {
        self.range.previous_step(from)
    }

    fn next_step(&self, from: Self::Plain, _finer: bool) -> Self::Plain {
        self.range.next_step(from)
    }

    fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String {
        let value = self.preview_plain(normalized);
        match (&self.value_to_string, include_unit) {
            (Some(f), true) => format!("{}{}", f(value), self.unit),
            (Some(f), false) => f(value),
            (None, true) => format!("{}{}", value, self.unit),
            (None, false) => format!("{value}"),
        }
    }

    fn string_to_normalized_value(&self, string: &str) -> Option<f32> {
        let value = match &self.string_to_value {
            Some(f) => f(string.trim()),
            // In the CLAP wrapper the unit will be included, so make sure to handle that
            None => string.trim().trim_end_matches(self.unit).parse().ok(),
        }?;

        Some(self.preview_normalized(value))
    }

    #[inline]
    fn preview_normalized(&self, plain: Self::Plain) -> f32 {
        self.range.normalize(plain)
    }

    #[inline]
    fn preview_plain(&self, normalized: f32) -> Self::Plain {
        self.range.unnormalize(normalized)
    }

    fn flags(&self) -> ParamFlags {
        self.flags
    }

    fn as_ptr(&self) -> ParamPtr {
        ParamPtr::IntParam(self as *const _ as *mut _)
    }
}

impl ParamMut for IntParam {
    fn set_plain_value(&self, plain: Self::Plain) -> bool {
        let unmodulated_value = plain;
        let unmodulated_normalized_value = self.preview_normalized(plain);

        let modulation_offset = self.modulation_offset.load(Ordering::Relaxed);
        let (value, normalized_value) = if modulation_offset == 0.0 {
            (unmodulated_value, unmodulated_normalized_value)
        } else {
            let normalized_value =
                (unmodulated_normalized_value + modulation_offset).clamp(0.0, 1.0);

            (self.preview_plain(normalized_value), normalized_value)
        };

        // REAPER spams automation events with the same value. This prevents callbacks from firing
        // multiple times. This can be problematic when they're used to trigger expensive
        // computations when a parameter changes.
        let old_value = self.value.swap(value, Ordering::Relaxed);
        if value != old_value {
            self.normalized_value
                .store(normalized_value, Ordering::Relaxed);
            self.unmodulated_value
                .store(unmodulated_value, Ordering::Relaxed);
            self.unmodulated_normalized_value
                .store(unmodulated_normalized_value, Ordering::Relaxed);
            if let Some(f) = &self.value_changed {
                f(value);
            }

            true
        } else {
            false
        }
    }

    fn set_normalized_value(&self, normalized: f32) -> bool {
        // NOTE: The double conversion here is to make sure the state is reproducible. State is
        //       saved and restored using plain values, and the new normalized value will be
        //       different from `normalized`. This is not necessary for the modulation as these
        //       values are never shown to the host.
        self.set_plain_value(self.preview_plain(normalized))
    }

    fn modulate_value(&self, modulation_offset: f32) -> bool {
        self.modulation_offset
            .store(modulation_offset, Ordering::Relaxed);

        // TODO: This renormalizes this value, which is not necessary
        self.set_plain_value(self.unmodulated_plain_value())
    }

    fn update_smoother(&self, sample_rate: f32, reset: bool) {
        if reset {
            self.smoothed.reset(self.modulated_plain_value());
        } else {
            self.smoothed
                .set_target(sample_rate, self.modulated_plain_value());
        }
    }
}

impl IntParam {
    /// Build a new [`IntParam`]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: impl Into<String>, default: i32, range: IntRange) -> Self {
        range.assert_validity();

        Self {
            value: AtomicI32::new(default),
            normalized_value: AtomicF32::new(range.normalize(default)),
            unmodulated_value: AtomicI32::new(default),
            unmodulated_normalized_value: AtomicF32::new(range.normalize(default)),
            modulation_offset: AtomicF32::new(0.0),
            default,
            smoothed: Smoother::none(),

            flags: ParamFlags::default(),
            value_changed: None,

            range,
            name: name.into(),
            unit: "",
            poly_modulation_id: None,
            value_to_string: None,
            string_to_value: None,
        }
    }

    /// The field's current plain value, after monophonic modulation has been applied. Equivalent to
    /// calling `param.plain_value()`.
    #[inline]
    pub fn value(&self) -> i32 {
        self.modulated_plain_value()
    }

    /// The range of valid plain values for this parameter.
    #[inline]
    pub fn range(&self) -> IntRange {
        self.range
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
        self.poly_modulation_id = Some(id);
        self
    }

    /// Set up a smoother that can gradually interpolate changes made to this parameter, preventing
    /// clicks and zipper noises.
    pub fn with_smoother(mut self, style: SmoothingStyle) -> Self {
        // Logarithmic smoothing will cause problems if the range goes through zero since then you
        // end up multiplying by zero
        let goes_through_zero = match (&style, &self.range) {
            (SmoothingStyle::Logarithmic(_), IntRange::Linear { min, max }) => {
                *min == 0 || *max == 0 || min.signum() != max.signum()
            }
            _ => false,
        };
        nih_debug_assert!(
            !goes_through_zero,
            "Logarithmic smoothing does not work with ranges that go through zero"
        );

        self.smoothed = Smoother::new(style);
        self
    }

    /// Run a callback whenever this parameter's value changes. The argument passed to this function
    /// is the parameter's new value. This should not do anything expensive as it may be called
    /// multiple times in rapid succession, and it can be run from both the GUI and the audio
    /// thread.
    pub fn with_callback(mut self, callback: Arc<dyn Fn(i32) + Send + Sync>) -> Self {
        self.value_changed = Some(callback);
        self
    }

    /// Display a unit when rendering this parameter to a string. Appended after the
    /// [`value_to_string`][Self::with_value_to_string()] function if that is also set. NIH-plug
    /// will not automatically add a space before the unit.
    pub fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    /// Use a custom conversion function to convert the plain, unnormalized value to a
    /// string.
    pub fn with_value_to_string(
        mut self,
        callback: Arc<dyn Fn(i32) -> String + Send + Sync>,
    ) -> Self {
        self.value_to_string = Some(callback);
        self
    }

    // `with_step_size` is only implemented for the f32 version

    /// Use a custom conversion function to convert from a string to a plain, unnormalized
    /// value. If the string cannot be parsed, then this should return a `None`. If this
    /// happens while the parameter is being updated then the update will be canceled.
    ///
    /// The input string may or may not contain the unit, so you will need to be able to handle
    /// that.
    pub fn with_string_to_value(
        mut self,
        callback: Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>,
    ) -> Self {
        self.string_to_value = Some(callback);
        self
    }

    /// Mark the parameter as non-automatable. This means that the parameter cannot be changed from
    /// an automation lane. The parameter can however still be manually changed by the user from
    /// either the plugin's own GUI or from the host's generic UI.
    pub fn non_automatable(mut self) -> Self {
        self.flags.insert(ParamFlags::NON_AUTOMATABLE);
        self
    }

    /// Hide the parameter in the host's generic UI for this plugin. This also implies
    /// `NON_AUTOMATABLE`. Setting this does not prevent you from changing the parameter in the
    /// plugin's editor GUI.
    pub fn hide(mut self) -> Self {
        self.flags.insert(ParamFlags::HIDDEN);
        self
    }

    /// Don't show this parameter when generating a generic UI for the plugin using one of
    /// NIH-plug's generic UI widgets.
    pub fn hide_in_generic_ui(mut self) -> Self {
        self.flags.insert(ParamFlags::HIDE_IN_GENERIC_UI);
        self
    }
}
