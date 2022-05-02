//! Simple boolean parameters.

use std::fmt::Display;
use std::sync::Arc;

use super::internals::ParamPtr;
use super::{Param, ParamFlags, ParamMut};

/// A simple boolean parameter.
#[repr(C, align(4))]
pub struct BoolParam {
    /// The field's current value.
    pub value: bool,
    /// The field's current value normalized to the `[0, 1]` range.
    normalized_value: f32,
    /// The field's value before any monophonic automation coming from the host has been applied.
    /// This will always be the same as `value` for VST3 plugins.
    unmodulated_value: bool,
    /// The field's value normalized to the `[0, 1]` range before any monophonic automation coming
    /// from the host has been applied. This will always be the same as `value` for VST3 plugins.
    unmodulated_normalized_value: f32,
    /// A value in `[-1, 1]` indicating the amount of modulation applied to
    /// `unmodulated_normalized_`. This needs to be stored separately since the normalied values are
    /// clamped, and this value persists after new automation events.
    modulation_offset: f32,
    /// The field's default value.
    default: bool,

    /// Flags to control the parameter's behavior. See [`ParamFlags`].
    flags: ParamFlags,
    /// Optional callback for listening to value changes. The argument passed to this function is
    /// the parameter's new value. This should not do anything expensive as it may be called
    /// multiple times in rapid succession, and it can be run from both the GUI and the audio
    /// thread.
    value_changed: Option<Arc<dyn Fn(bool) + Send + Sync>>,

    /// The parameter's human readable display name.
    name: String,
    /// Optional custom conversion function from a boolean value to a string.
    value_to_string: Option<Arc<dyn Fn(bool) -> String + Send + Sync>>,
    /// Optional custom conversion function from a string to a boolean value. If the string cannot
    /// be parsed, then this should return a `None`. If this happens while the parameter is being
    /// updated then the update will be canceled.
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<bool> + Send + Sync>>,
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

impl Param for BoolParam {
    type Plain = bool;

    fn name(&self) -> &str {
        &self.name
    }

    fn unit(&self) -> &'static str {
        ""
    }

    #[inline]
    fn plain_value(&self) -> Self::Plain {
        self.value
    }

    #[inline]
    fn normalized_value(&self) -> f32 {
        self.normalized_value
    }

    #[inline]
    fn unmodulated_plain_value(&self) -> Self::Plain {
        self.unmodulated_value
    }

    #[inline]
    fn unmodulated_normalized_value(&self) -> f32 {
        self.unmodulated_normalized_value
    }

    #[inline]
    fn default_plain_value(&self) -> Self::Plain {
        self.default
    }

    fn step_count(&self) -> Option<usize> {
        Some(1)
    }

    fn previous_step(&self, _from: Self::Plain) -> Self::Plain {
        false
    }

    fn next_step(&self, _from: Self::Plain) -> Self::Plain {
        true
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
        let string = string.trim();
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

    fn initialize_block_smoother(&mut self, _max_block_size: usize) {}

    fn flags(&self) -> ParamFlags {
        self.flags
    }

    fn as_ptr(&self) -> ParamPtr {
        ParamPtr::BoolParam(self as *const BoolParam as *mut BoolParam)
    }
}

impl ParamMut for BoolParam {
    fn set_plain_value(&mut self, plain: Self::Plain) {
        self.unmodulated_value = plain;
        self.unmodulated_normalized_value = self.preview_normalized(plain);
        if self.modulation_offset == 0.0 {
            self.value = self.unmodulated_value;
            self.normalized_value = self.unmodulated_normalized_value;
        } else {
            self.normalized_value =
                (self.unmodulated_normalized_value + self.modulation_offset).clamp(0.0, 1.0);
            self.value = self.preview_plain(self.normalized_value);
        }
        if let Some(f) = &self.value_changed {
            f(self.value);
        }
    }

    fn set_normalized_value(&mut self, normalized: f32) {
        self.unmodulated_value = self.preview_plain(normalized);
        self.unmodulated_normalized_value = normalized;
        if self.modulation_offset == 0.0 {
            self.value = self.unmodulated_value;
            self.normalized_value = self.unmodulated_normalized_value;
        } else {
            self.normalized_value =
                (self.unmodulated_normalized_value + self.modulation_offset).clamp(0.0, 1.0);
            self.value = self.preview_plain(self.normalized_value);
        }
        if let Some(f) = &self.value_changed {
            f(self.value);
        }
    }

    fn modulate_value(&mut self, modulation_offset: f32) {
        self.modulation_offset = modulation_offset;
        self.set_normalized_value(self.unmodulated_normalized_value);
    }

    fn update_smoother(&mut self, _sample_rate: f32, _init: bool) {
        // Can't really smooth a binary parameter now can you
    }
}

impl BoolParam {
    /// Build a new [`BoolParam`]. Use the other associated functions to modify the behavior of the
    /// parameter.
    pub fn new(name: impl Into<String>, default: bool) -> Self {
        Self {
            value: default,
            normalized_value: if default { 1.0 } else { 0.0 },
            unmodulated_value: default,
            unmodulated_normalized_value: if default { 1.0 } else { 0.0 },
            modulation_offset: 0.0,
            default,

            flags: ParamFlags::default(),
            value_changed: None,

            name: name.into(),
            value_to_string: None,
            string_to_value: None,
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

    /// Mark this parameter as a bypass parameter. Plugin hosts can integrate this parameter into
    /// their UI. Only a single [`BoolParam`] can be a bypass parameter, and NIH-plug will add one
    /// if you don't create one yourself. You will need to implement this yourself if your plugin
    /// introduces latency.
    pub fn make_bypass(mut self) -> Self {
        self.flags.insert(ParamFlags::BYPASS);
        self
    }

    /// Mark the paramter as non-automatable. This means that the parameter cannot be automated from
    /// the host. Setting this flag also prevents it from showing up in the host's own generic UI
    /// for this plugin. The parameter can still be changed from the plugin's editor GUI.
    pub fn non_automatable(mut self) -> Self {
        self.flags.insert(ParamFlags::NON_AUTOMATABLE);
        self
    }

    /// Don't show this parameter when generating a generic UI for the plugin using one of
    /// NIH-plug's generic UI widgets.
    pub fn hide_in_generic_ui(mut self) -> Self {
        self.flags.insert(ParamFlags::HIDE_IN_GENERIC_UI);
        self
    }
}
