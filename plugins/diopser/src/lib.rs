// Diopser: a phase rotation plugin
// Copyright (C) 2021-2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

#![cfg_attr(feature = "simd", feature(portable_simd))]

#[cfg(not(feature = "simd"))]
compile_error!("Compiling without SIMD support is currently not supported");

use atomic_float::AtomicF32;
use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use std::simd::f32x2;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::spectrum::{SpectrumInput, SpectrumOutput};

mod editor;
mod filter;
mod spectrum;

/// How many all-pass filters we can have in series at most. The filter stages parameter determines
/// how many filters are actually active.
const MAX_NUM_FILTERS: usize = 512;
/// The minimum step size for smoothing the filter parameters.
const MIN_AUTOMATION_STEP_SIZE: u32 = 1;
/// The maximum step size for smoothing the filter parameters. Updating these parameters can be
/// expensive, so updating them in larger steps can be useful.
const MAX_AUTOMATION_STEP_SIZE: u32 = 512;

/// The maximum number of samples to iterate over at a time.
const MAX_BLOCK_SIZE: usize = 64;

/// The filter frequency parameter's range. Also used in the `SpectrumAnalyzer` widget.
pub(crate) fn filter_frequency_range() -> FloatRange {
    FloatRange::Skewed {
        min: 5.0, // This must never reach 0
        max: 20_000.0,
        factor: FloatRange::skew_factor(-2.5),
    }
}

// All features from the original Diopser have been implemented (and the spread control has been
// improved). Other features I want to implement are:
// - Briefly muting the output when changing the number of filters to get rid of the clicks
// - A proper GUI
pub struct Diopser {
    params: Arc<DiopserParams>,

    /// Needed for computing the filter coefficients. Also used to update `bypass_smoother`, hence
    /// why this needs to be an `Arc<AtomicF32>`.
    sample_rate: Arc<AtomicF32>,

    /// All of the all-pass filters, with vectorized coefficients so they can be calculated for
    /// multiple channels at once. [`DiopserParams::num_stages`] controls how many filters are
    /// actually active.
    filters: [filter::Biquad<f32x2>; MAX_NUM_FILTERS],
    /// When the bypass parameter is toggled, this smoother fades between 0.0 and 1.0. This lets us
    /// crossfade the dry and the wet signal to avoid clicks. The smoothing target is set in a
    /// callback handler on the bypass parameter.
    bypass_smoother: Arc<Smoother<f32>>,

    /// If this is set at the start of the processing cycle, then the filter coefficients should be
    /// updated. For the regular filter parameters we can look at the smoothers, but this is needed
    /// when changing the number of active filters.
    should_update_filters: Arc<AtomicBool>,
    /// If this is 1 and any of the filter parameters are still smoothing, thenn the filter
    /// coefficients should be recalculated on the next sample. After that, this gets reset to
    /// `unnormalize_automation_precision(self.params.automation_precision.value())`. This is to
    /// reduce the DSP load of automation parameters. It can also cause some fun sounding glitchy
    /// effects when the precision is low.
    next_filter_smoothing_in: i32,

    /// When the GUI is open we compute the spectrum on the audio thread and send it to the GUI.
    spectrum_input: SpectrumInput,
    /// This can be cloned and moved into the editor.
    spectrum_output: Arc<Mutex<SpectrumOutput>>,
}

#[derive(Params)]
struct DiopserParams {
    /// The editor state, saved together with the parameter state so the custom scaling can be
    /// restored.
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,
    /// If this option is enabled, then the filter stages parameter is limited to `[0, 40]`. This is
    /// editor-only state, and doesn't affect host automation.
    #[persist = "safe-mode"]
    safe_mode: Arc<AtomicBool>,

    /// This plugin really doesn't need its own bypass parameter, but it's still useful to have a
    /// dedicated one so it can be shown in the GUI. This is linked to the host's bypass if the host
    /// supports it.
    #[id = "bypass"]
    bypass: BoolParam,

    /// The number of all-pass filters applied in series.
    #[id = "stages"]
    filter_stages: IntParam,

    /// The filter's center frequqency. When this is applied, the filters are spread around this
    /// frequency.
    #[id = "cutoff"]
    filter_frequency: FloatParam,
    /// The Q parameter for the filters.
    #[id = "res"]
    filter_resonance: FloatParam,
    /// Controls a frequency spread between the filter stages in octaves. When this value is 0, the
    /// same coefficients are used for every filter. Otherwise, the earliest stage's frequency will
    /// be offset by `-filter_spread_octave_amount`, while the latest stage will be offset by
    /// `filter_spread_octave_amount`. If the filter spread style is set to linear then the negative
    /// range will cover the same frequency range as the positive range.
    #[id = "spread"]
    filter_spread_octaves: FloatParam,
    /// How the spread range should be distributed. The octaves mode will sound more musical while
    /// the linear mode can be useful for sound design purposes.
    #[id = "spstyl"]
    filter_spread_style: EnumParam<SpreadStyle>,

    /// The precision of the automation, determines the step size. This is presented to the userq as
    /// a percentage, and it's stored here as `[0, 1]` float because smaller step sizes are more
    /// precise so having this be an integer would result in odd situations.
    #[id = "autopr"]
    automation_precision: FloatParam,

    /// Very important.
    #[id = "ignore"]
    very_important: BoolParam,
}

impl Default for Diopser {
    fn default() -> Self {
        let sample_rate = Arc::new(AtomicF32::new(1.0));
        let should_update_filters = Arc::new(AtomicBool::new(false));
        let bypass_smoother = Arc::new(Smoother::new(SmoothingStyle::Linear(10.0)));

        // We only do stereo right now so this is simple
        let (spectrum_input, spectrum_output) =
            SpectrumInput::new(Self::DEFAULT_OUTPUT_CHANNELS as usize);

        Self {
            params: Arc::new(DiopserParams::new(
                sample_rate.clone(),
                should_update_filters.clone(),
                bypass_smoother.clone(),
            )),

            sample_rate,

            filters: [filter::Biquad::default(); MAX_NUM_FILTERS],
            bypass_smoother,

            should_update_filters,
            next_filter_smoothing_in: 1,

            spectrum_input,
            spectrum_output: Arc::new(Mutex::new(spectrum_output)),
        }
    }
}

impl DiopserParams {
    fn new(
        sample_rate: Arc<AtomicF32>,
        should_update_filters: Arc<AtomicBool>,
        bypass_smoother: Arc<Smoother<f32>>,
    ) -> Self {
        Self {
            editor_state: editor::default_state(),
            safe_mode: Arc::new(AtomicBool::new(true)),

            bypass: BoolParam::new("Bypass", false)
                .with_callback(Arc::new(move |value| {
                    bypass_smoother.set_target(
                        sample_rate.load(Ordering::Relaxed),
                        if value { 1.0 } else { 0.0 },
                    );
                }))
                .with_value_to_string(formatters::s2v_bool_bypass())
                .with_string_to_value(formatters::v2s_bool_bypass())
                .make_bypass(),

            filter_stages: IntParam::new(
                "Filter Stages",
                0,
                IntRange::Linear {
                    min: 0,
                    max: MAX_NUM_FILTERS as i32,
                },
            )
            .with_callback({
                let should_update_filters = should_update_filters.clone();
                Arc::new(move |_| should_update_filters.store(true, Ordering::Release))
            }),

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
                normalize_automation_precision(128),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Enum, Debug, PartialEq)]
enum SpreadStyle {
    #[id = "octaves"]
    Octaves,
    #[id = "linear"]
    Linear,
}

impl Plugin for Diopser {
    const NAME: &'static str = "Diopser";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const DEFAULT_INPUT_CHANNELS: u32 = 2;
    const DEFAULT_OUTPUT_CHANNELS: u32 = 2;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            editor::Data {
                params: self.params.clone(),

                sample_rate: self.sample_rate.clone(),
                spectrum: self.spectrum_output.clone(),
                safe_mode: self.params.safe_mode.clone(),
            },
            self.params.editor_state.clone(),
        )
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // The SIMD version only supports stereo
        config.num_input_channels == config.num_output_channels && config.num_input_channels == 2
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate
            .store(buffer_config.sample_rate, Ordering::Relaxed);

        // The spectrum is smoothed so it decays gradually
        self.spectrum_input
            .update_sample_rate(buffer_config.sample_rate);

        true
    }

    fn reset(&mut self) {
        // Initialize and/or reset the filters on the next process call
        self.should_update_filters.store(true, Ordering::Release);
        self.bypass_smoother
            .reset(if self.params.bypass.value() { 1.0 } else { 0.0 });
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Since this is an expensive operation, only update the filters when it's actually
        // necessary, and allow smoothing only every n samples using the automation precision
        // parameter
        let smoothing_interval =
            unnormalize_automation_precision(self.params.automation_precision.value());

        // The bypass parameter controls a smoother so we can crossfade between the dry and the wet
        // signals as needed
        if !self.params.bypass.value() || self.bypass_smoother.is_smoothing() {
            // We'll iterate in blocks to make the blending relatively cheap without having to
            // duplicate code or add a bunch of per-sample conditionals
            for (_, mut block) in buffer.iter_blocks(MAX_BLOCK_SIZE) {
                // We'll blend this with the dry signal as needed
                let mut dry = [f32x2::default(); MAX_BLOCK_SIZE];
                let mut wet = [f32x2::default(); MAX_BLOCK_SIZE];
                for (input_samples, (dry_samples, wet_samples)) in block
                    .iter_samples()
                    .zip(std::iter::zip(dry.iter_mut(), wet.iter_mut()))
                {
                    self.maybe_update_filters(smoothing_interval);

                    // We can compute the filters for both channels at once. The SIMD version thus now
                    // only supports steroo audio.
                    *dry_samples = unsafe { input_samples.to_simd_unchecked() };
                    *wet_samples = *dry_samples;

                    for filter in self
                        .filters
                        .iter_mut()
                        .take(self.params.filter_stages.value() as usize)
                    {
                        *wet_samples = filter.process(*wet_samples);
                    }
                }

                // If the bypass smoother is activated then the bypass switch has just been flipped to
                // either the on or the off position
                if self.bypass_smoother.is_smoothing() {
                    for (mut channel_samples, (dry_samples, wet_samples)) in block
                        .iter_samples()
                        .zip(std::iter::zip(dry.iter_mut(), wet.iter_mut()))
                    {
                        // We'll do an equal-power fade
                        let dry_t_squared = self.bypass_smoother.next();
                        let dry_t = dry_t_squared.sqrt();
                        let wet_t = (1.0 - dry_t_squared).sqrt();

                        let dry_weighted = *dry_samples * f32x2::splat(dry_t);
                        let wet_weighted = *wet_samples * f32x2::splat(wet_t);

                        unsafe { channel_samples.from_simd_unchecked(dry_weighted + wet_weighted) };
                    }
                } else if self.params.bypass.value() {
                    // If the bypass is enabled and we're no longer smoothing then the output should
                    // just be the origianl dry signal
                } else {
                    // Otherwise the signal is 100% wet
                    for (mut channel_samples, wet_samples) in block.iter_samples().zip(wet.iter()) {
                        unsafe { channel_samples.from_simd_unchecked(*wet_samples) };
                    }
                }
            }
        }

        // Compute a spectrum for the GUI if needed
        if self.params.editor_state.is_open() {
            self.spectrum_input.compute(buffer);
        }

        ProcessStatus::Normal
    }
}

impl Diopser {
    /// Check if the filters need to be updated beased on
    /// [`should_update_filters`][Self::should_update_filters] and the smoothing interval, and
    /// update them as needed.
    fn maybe_update_filters(&mut self, smoothing_interval: u32) {
        // In addition to updating the filters, we should also clear the filter's state when
        // changing a setting we can't neatly interpolate between.
        let reset_filters = self
            .should_update_filters
            .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
            .is_ok();
        let should_update_filters = reset_filters
            || ((self.params.filter_frequency.smoothed.is_smoothing()
                || self.params.filter_resonance.smoothed.is_smoothing()
                || self.params.filter_spread_octaves.smoothed.is_smoothing())
                && self.next_filter_smoothing_in <= 1);
        if should_update_filters {
            self.update_filters(smoothing_interval, reset_filters);
            self.next_filter_smoothing_in = smoothing_interval as i32;
        } else {
            self.next_filter_smoothing_in -= 1;
        }
    }

    /// Recompute the filter coefficients based on the smoothed paraetersm. We can skip forwardq in
    /// larger steps to reduce the DSP load.
    fn update_filters(&mut self, smoothing_interval: u32, reset_filters: bool) {
        if self.filters.is_empty() {
            return;
        }

        let sample_rate = self.sample_rate.load(Ordering::Relaxed);
        let frequency = self
            .params
            .filter_frequency
            .smoothed
            .next_step(smoothing_interval);
        let resonance = self
            .params
            .filter_resonance
            .smoothed
            .next_step(smoothing_interval);
        let spread_octaves = self
            .params
            .filter_spread_octaves
            .smoothed
            .next_step(smoothing_interval);
        let spread_style = self.params.filter_spread_style.value();

        // Used to calculate the linear spread. This is calculated in such a way that the range
        // never dips below 0.
        let max_octave_spread = if spread_octaves >= 0.0 {
            frequency - (frequency * 2.0f32.powf(-spread_octaves))
        } else {
            (frequency * 2.0f32.powf(spread_octaves)) - frequency
        };

        // TODO: This wrecks the DSP load at high smoothing accuracy, perhaps also use SIMD here
        const MIN_FREQUENCY: f32 = 5.0;
        let max_frequency = sample_rate / 2.05;
        for filter_idx in 0..self.params.filter_stages.value() as usize {
            // The index of the filter normalized to range [-1, 1]
            let filter_proportion =
                (filter_idx as f32 / self.params.filter_stages.value() as f32) * 2.0 - 1.0;

            // The spread parameter adds an offset to the frequency depending on the number of the
            // filter
            let filter_frequency = match spread_style {
                SpreadStyle::Octaves => frequency * 2.0f32.powf(spread_octaves * filter_proportion),
                SpreadStyle::Linear => frequency + (max_octave_spread * filter_proportion),
            }
            .clamp(MIN_FREQUENCY, max_frequency);

            self.filters[filter_idx].coefficients =
                filter::BiquadCoefficients::allpass(sample_rate, filter_frequency, resonance);
            if reset_filters {
                self.filters[filter_idx].reset();
            }
        }
    }
}

fn normalize_automation_precision(step_size: u32) -> f32 {
    (MAX_AUTOMATION_STEP_SIZE - step_size) as f32
        / (MAX_AUTOMATION_STEP_SIZE - MIN_AUTOMATION_STEP_SIZE) as f32
}

fn unnormalize_automation_precision(normalized: f32) -> u32 {
    MAX_AUTOMATION_STEP_SIZE
        - (normalized * (MAX_AUTOMATION_STEP_SIZE - MIN_AUTOMATION_STEP_SIZE) as f32).round() as u32
}

impl ClapPlugin for Diopser {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.diopser";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A totally original phase rotation plugin");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Filter,
        ClapFeature::Utility,
    ];
}

impl Vst3Plugin for Diopser {
    const VST3_CLASS_ID: [u8; 16] = *b"DiopserPlugRvdH.";
    const VST3_CATEGORIES: &'static str = "Fx|Filter";
}

nih_export_clap!(Diopser);
nih_export_vst3!(Diopser);
