//! Diopser's parameter structs.
//!
//! This is moved to a module to avoid cluttering up `lib.rs` because we also need to expose the
//! ranges separately for some of the GUI abstractions to work.

use atomic_float::AtomicF32;
use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How many all-pass filters we can have in series at most. The filter stages parameter determines
/// how many filters are actually active.
pub const MAX_NUM_FILTERS: usize = 512;
/// The minimum step size for smoothing the filter parameters.
pub const MIN_AUTOMATION_STEP_SIZE: u32 = 1;
/// The maximum step size for smoothing the filter parameters. Updating these parameters can be
/// expensive, so updating them in larger steps can be useful.
pub const MAX_AUTOMATION_STEP_SIZE: u32 = 512;

/// The filter stages parameter's range. Also used in the safe mode utilities.
pub fn filter_stages_range() -> IntRange {
    IntRange::Linear {
        min: 0,
        max: MAX_NUM_FILTERS as i32,
    }
}

/// The filter stages parameters minimum value in safe mode.
pub const FILTER_STAGES_RESTRICTED_MIN: i32 = 0;
/// The filter stages parameters maximum value in safe mode.
pub const FILTER_STAGES_RESTRICTED_MAX: i32 = 40;

/// The filter frequency parameter's range. Also used in the `SpectrumAnalyzer` widget.
pub fn filter_frequency_range() -> FloatRange {
    FloatRange::Skewed {
        min: 5.0, // This must never reach 0
        max: 20_000.0,
        factor: FloatRange::skew_factor(-2.5),
    }
}

/// The filter frequency parameters minimum value in safe mode.
pub const FILTER_FREQUENCY_RESTRICTED_MIN: f32 = 35.0;
/// The filter frequency parameters maximum value in safe mode.
pub const FILTER_FREQUENCY_RESTRICTED_MAX: f32 = 22_000.0;

pub fn normalize_automation_precision(step_size: u32) -> f32 {
    (MAX_AUTOMATION_STEP_SIZE - step_size) as f32
        / (MAX_AUTOMATION_STEP_SIZE - MIN_AUTOMATION_STEP_SIZE) as f32
}

pub fn unnormalize_automation_precision(normalized: f32) -> u32 {
    MAX_AUTOMATION_STEP_SIZE
        - (normalized * (MAX_AUTOMATION_STEP_SIZE - MIN_AUTOMATION_STEP_SIZE) as f32).round() as u32
}

#[derive(Params)]
pub struct DiopserParams {
    /// The editor state, saved together with the parameter state so the custom scaling can be
    /// restored.
    #[persist = "editor-state"]
    pub editor_state: Arc<ViziaState>,
    /// If this option is enabled, then the filter stages parameter is limited to `[0, 40]`. This is
    /// editor-only state, and doesn't affect host automation.
    #[persist = "safe-mode"]
    pub safe_mode: Arc<AtomicBool>,

    /// This plugin really doesn't need its own bypass parameter, but it's still useful to have a
    /// dedicated one so it can be shown in the GUI. This is linked to the host's bypass if the host
    /// supports it.
    #[id = "bypass"]
    pub bypass: BoolParam,

    /// The number of all-pass filters applied in series.
    #[id = "stages"]
    pub filter_stages: IntParam,

    /// The filter's center frequqency. When this is applied, the filters are spread around this
    /// frequency.
    #[id = "cutoff"]
    pub filter_frequency: FloatParam,
    /// The Q parameter for the filters.
    #[id = "res"]
    pub filter_resonance: FloatParam,
    /// Controls a frequency spread between the filter stages in octaves. When this value is 0, the
    /// same coefficients are used for every filter. Otherwise, the earliest stage's frequency will
    /// be offset by `-filter_spread_octave_amount`, while the latest stage will be offset by
    /// `filter_spread_octave_amount`. If the filter spread style is set to linear then the negative
    /// range will cover the same frequency range as the positive range.
    #[id = "spread"]
    pub filter_spread_octaves: FloatParam,
    /// How the spread range should be distributed. The octaves mode will sound more musical while
    /// the linear mode can be useful for sound design purposes.
    #[id = "spstyl"]
    pub filter_spread_style: EnumParam<SpreadStyle>,

    /// The precision of the automation, determines the step size. This is presented to the userq as
    /// a percentage, and it's stored here as `[0, 1]` float because smaller step sizes are more
    /// precise so having this be an integer would result in odd situations.
    #[id = "autopr"]
    pub automation_precision: FloatParam,

    /// Very important.
    #[id = "ignore"]
    pub very_important: BoolParam,
}

#[derive(Enum, Debug, PartialEq)]
pub enum SpreadStyle {
    #[id = "octaves"]
    Octaves,
    #[id = "linear"]
    Linear,
}

impl DiopserParams {
    pub fn new(
        sample_rate: Arc<AtomicF32>,
        should_update_filters: Arc<AtomicBool>,
        bypass_smoother: Arc<Smoother<f32>>,
    ) -> Self {
        Self {
            editor_state: crate::editor::default_state(),
            safe_mode: Arc::new(AtomicBool::new(true)),

            bypass: BoolParam::new("Bypass", false)
                .with_callback(Arc::new(move |value| {
                    bypass_smoother.set_target(
                        sample_rate.load(Ordering::Relaxed),
                        if value { 1.0 } else { 0.0 },
                    );
                }))
                .with_value_to_string(formatters::v2s_bool_bypass())
                .with_string_to_value(formatters::s2v_bool_bypass())
                .make_bypass(),

            filter_stages: IntParam::new("Filter Stages", 0, filter_stages_range()).with_callback(
                {
                    let should_update_filters = should_update_filters.clone();
                    Arc::new(move |_| should_update_filters.store(true, Ordering::Release))
                },
            ),

            // Smoothed parameters don't need the callback as we can just look at whether the
            // smoother is still smoothing
            filter_frequency: FloatParam::new(
                "Filter Frequency",
                200.0,
                // This value is also used in the spectrum analyzer to match the spectrum analyzer
                // with this parameter which is bound to the X-Y pad's X-axis
                filter_frequency_range(),
            )
            // This needs quite a bit of smoothing to avoid artifacts
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            // This includes the unit
            .with_value_to_string(formatters::v2s_f32_hz_then_khz_with_note_name(0, true))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            filter_resonance: FloatParam::new(
                "Filter Resonance",
                // The actual default neutral Q-value would be `sqrt(2) / 2`, but this value
                // produces slightly less ringing.
                0.5,
                FloatRange::Skewed {
                    min: 0.01, // This must also never reach 0
                    max: 30.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            filter_spread_octaves: FloatParam::new(
                "Filter Spread",
                0.0,
                FloatRange::SymmetricalSkewed {
                    min: -5.0,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-1.0),
                    center: 0.0,
                },
            )
            .with_unit(" octaves")
            .with_step_size(0.01)
            .with_smoother(SmoothingStyle::Linear(100.0)),
            filter_spread_style: EnumParam::new("Filter Spread Style", SpreadStyle::Octaves)
                .with_callback(Arc::new(move |_| {
                    should_update_filters.store(true, Ordering::Release)
                })),

            very_important: BoolParam::new("Don't touch this", true)
                .with_value_to_string(Arc::new(|value| {
                    String::from(if value { "please don't" } else { "stop it" })
                }))
                .with_string_to_value(Arc::new(|string| {
                    let string = string.trim();
                    if string.eq_ignore_ascii_case("please don't") {
                        Some(true)
                    } else if string.eq_ignore_ascii_case("stop it") {
                        Some(false)
                    } else {
                        None
                    }
                }))
                .hide_in_generic_ui(),

            automation_precision: FloatParam::new(
                "Automation precision",
                normalize_automation_precision(MIN_AUTOMATION_STEP_SIZE),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}
