//! Utilities for Diopser's safe-mode mechanism.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nih_plug::prelude::Param;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::ParamEvent;

use crate::params::{self, DiopserParams};

/// Restricts the ranges of several parameters when enabled. This makes it more difficult to
/// generate load resonances with Diopser's default settings.
#[derive(Clone)]
pub struct SafeModeClamper {
    /// Whether the safe mode toggle has been enabled.
    enabled: Arc<AtomicBool>,
    /// The rest of the parameters struct. Used to restrict the parameter ranges when safe mode gets
    /// enabled.
    params: Arc<DiopserParams>,

    /// The minimum value for the filter stages parameter when safe mode is enabled, normalized as a
    /// `[0, 1]` value of the original full range.
    filter_stages_restricted_normalized_min: f32,
    /// The maximum value for the filter stages parameter when safe mode is enabled, normalized as a
    /// `[0, 1]` value of the original full range.
    filter_stages_restricted_normalized_max: f32,

    /// The minimum value for the filter frequency parameter when safe mode is enabled, normalized
    /// as a `[0, 1]` value of the original full range.
    filter_frequency_restricted_normalized_min: f32,
    /// The maximum value for the filter frequency parameter when safe mode is enabled, normalized
    /// as a `[0, 1]` value of the original full range.
    filter_frequency_restricted_normalized_max: f32,
}

impl SafeModeClamper {
    pub fn new(params: Arc<DiopserParams>) -> Self {
        let filter_stages_range = params::filter_stages_range();
        let filter_frequency_range = params::filter_frequency_range();

        Self {
            enabled: params.safe_mode.clone(),
            params,

            filter_stages_restricted_normalized_min: filter_stages_range
                .normalize(params::FILTER_STAGES_RESTRICTED_MIN),
            filter_stages_restricted_normalized_max: filter_stages_range
                .normalize(params::FILTER_STAGES_RESTRICTED_MAX),

            filter_frequency_restricted_normalized_min: filter_frequency_range
                .normalize(params::FILTER_FREQUENCY_RESTRICTED_MIN),
            filter_frequency_restricted_normalized_max: filter_frequency_range
                .normalize(params::FILTER_FREQUENCY_RESTRICTED_MAX),
        }
    }

    /// Return the current status of the safe mode swtich.
    pub fn status(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Enable or disable safe mode. Enabling safe mode immediately clamps the parameters to their
    /// new restricted ranges.
    pub fn toggle(&self, cx: &mut EventContext) {
        if !self.enabled.fetch_xor(true, Ordering::Relaxed) {
            self.restrict_range(cx);
        }
    }

    /// Enable safe mode. Enabling safe mode immediately clamps the parameters to their new
    /// restricted ranges.
    pub fn enable(&self, cx: &mut EventContext) {
        if !self.enabled.swap(true, Ordering::Relaxed) {
            self.restrict_range(cx);
        }
    }

    /// Disable safe mode.
    pub fn disable(&self) {
        // Disablign safe mode never needs to modify any parameters
        self.enabled.store(false, Ordering::Relaxed);
    }

    /// Depending on whether the safe mode is enabled or not this either returns `t`
    /// as is, or the range gets translated to the restricted range when safe mode is enabled. This
    /// is used for displaying the value. When handling events the range should be expanded again to
    /// the origianl values.
    pub fn filter_stages_renormalize_display(&self, t: f32) -> f32 {
        if self.status() {
            let renormalized = (t - self.filter_stages_restricted_normalized_min)
                / (self.filter_stages_restricted_normalized_max
                    - self.filter_stages_restricted_normalized_min);

            // This clamping may be necessary when safe mode is enabled but the effects from
            // `restrict_range()` have not been processed yet
            renormalized.clamp(0.0, 1.0)
        } else {
            t
        }
    }

    /// Depending on whether the safe mode is enabled or not this either returns `t`
    /// as is, or the restricted range gets translated back to the original range when safe mode is
    /// enabled.
    pub fn filter_stages_renormalize_event(&self, t: f32) -> f32 {
        if self.status() {
            // This is the opposite of `filter_stages_renormalize_display`
            t * (self.filter_stages_restricted_normalized_max
                - self.filter_stages_restricted_normalized_min)
                + self.filter_stages_restricted_normalized_min
        } else {
            t
        }
    }

    /// The same as
    /// [`filter_stages_renormalize_display()`][Self::filter_stages_renormalize_display()], but for
    /// filter freqnecy.
    pub fn filter_frequency_renormalize_display(&self, t: f32) -> f32 {
        if self.status() {
            let renormalized = (t - self.filter_frequency_restricted_normalized_min)
                / (self.filter_frequency_restricted_normalized_max
                    - self.filter_frequency_restricted_normalized_min);

            // This clamping may be necessary when safe mode is enabled but the effects from
            // `restrict_range()` have not been processed yet
            renormalized.clamp(0.0, 1.0)
        } else {
            t
        }
    }

    /// The same as [`filter_stages_renormalize_event()`][Self::filter_stages_renormalize_event()],
    /// but for filter freqnecy.
    pub fn filter_frequency_renormalize_event(&self, t: f32) -> f32 {
        if self.status() {
            t * (self.filter_frequency_restricted_normalized_max
                - self.filter_frequency_restricted_normalized_min)
                + self.filter_frequency_restricted_normalized_min
        } else {
            t
        }
    }

    /// CLamp the parameter values to the restricted range when enabling safe mode. This assumes
    /// there's no active automation gesture for these parameters.
    fn restrict_range(&self, cx: &mut EventContext) {
        let filter_stages = self.params.filter_stages.unmodulated_plain_value();
        let clamped_filter_stages = filter_stages.clamp(
            params::FILTER_STAGES_RESTRICTED_MIN,
            params::FILTER_STAGES_RESTRICTED_MAX,
        );
        if clamped_filter_stages != filter_stages {
            cx.emit(ParamEvent::BeginSetParameter(&self.params.filter_stages).upcast());
            cx.emit(
                ParamEvent::SetParameter(&self.params.filter_stages, clamped_filter_stages)
                    .upcast(),
            );
            cx.emit(ParamEvent::EndSetParameter(&self.params.filter_stages).upcast());
        }

        let filter_frequency = self.params.filter_frequency.unmodulated_plain_value();
        let clamped_filter_frequency = filter_frequency.clamp(
            params::FILTER_FREQUENCY_RESTRICTED_MIN,
            params::FILTER_FREQUENCY_RESTRICTED_MAX,
        );
        if clamped_filter_frequency != filter_frequency {
            cx.emit(ParamEvent::BeginSetParameter(&self.params.filter_frequency).upcast());
            cx.emit(
                ParamEvent::SetParameter(&self.params.filter_frequency, clamped_filter_frequency)
                    .upcast(),
            );
            cx.emit(ParamEvent::EndSetParameter(&self.params.filter_frequency).upcast());
        }
    }
}
