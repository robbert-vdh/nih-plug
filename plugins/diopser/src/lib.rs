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

#[macro_use]
extern crate nih_plug;

use nih_plug::{
    formatters, Buffer, BufferConfig, BusConfig, Plugin, ProcessContext, ProcessStatus, Vst3Plugin,
};
use nih_plug::{BoolParam, FloatParam, IntParam, Params, Range, SmoothingStyle};
use nih_plug::{Enum, EnumParam};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod filter;

/// How many all-pass filters we can have in series at most. The filter stages parameter determines
/// how many filters are actually active.
const MAX_NUM_FILTERS: usize = 512;
/// The minimum step size for smoothing the filter parmaeters.
const MIN_AUTOMATION_STEP_SIZE: u32 = 1;
/// The maximum step size for smoothing the filter parameters. Updating these parameters can be
/// expensive, so updating them in larger steps can be useful.
const MAX_AUTOMATION_STEP_SIZE: u32 = 512;

// All features from the original Diopser have been implemented (and the spread control has been
// improved). Other features I want to implement are:
// - Briefly muting the output when changing the number of filters to get rid of the clicks
// - A GUI
struct Diopser {
    params: Pin<Box<DiopserParams>>,

    /// Needed for computing the filter coefficients.
    sample_rate: f32,

    /// All of the all-pass filters, with one array of serial filters per channelq.
    /// [DiopserParams::num_stages] controls how many filters are actually active.
    filters: Vec<[filter::Biquad; MAX_NUM_FILTERS]>,
    /// If this is set at the start of the processing cycle, then the filter coefficients should be
    /// updated. For the regular filter parameters we can look at the smoothers, but this is needed
    /// when changing the number of active filters.
    should_update_filters: Arc<AtomicBool>,
    /// If this is 1 and any of the filter parameters are still smoothing, thenn the filter
    /// coefficients should be recalculated on the next sample. After that, this gets reset to
    /// `unnormalize_automation_precision(self.params.automation_precision.value)`. This is to
    /// reduce the DSP load of automation parameters. It can also cause some fun sounding glitchy
    /// effects when the precision is low.
    next_filter_smoothing_in: i32,
}

// TODO: Some combinations of parameters can cause really loud resonance. We should limit the
//       resonance and filter stages parameter ranges in the GUI until the user unlocks.
#[derive(Params)]
struct DiopserParams {
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
        let should_update_filters = Arc::new(AtomicBool::new(false));

        Self {
            params: Box::pin(DiopserParams::new(should_update_filters.clone())),

            sample_rate: 1.0,

            filters: Vec::new(),
            should_update_filters,
            next_filter_smoothing_in: 1,
        }
    }
}

impl DiopserParams {
    pub fn new(should_update_filters: Arc<AtomicBool>) -> Self {
        Self {
            filter_stages: IntParam::new(
                "Filter Stages",
                0,
                Range::Linear {
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
                Range::Skewed {
                    min: 5.0, // This must never reach 0
                    max: 20_000.0,
                    factor: Range::skew_factor(-2.5),
                },
            )
            // This needs quite a bit of smoothing to avoid artifacts
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::f32_rounded(0)),
            filter_resonance: FloatParam::new(
                "Filter Resonance",
                // The actual default neutral Q-value would be `sqrt(2) / 2`, but this value
                // produces slightly less ringing.
                0.5,
                Range::Skewed {
                    min: 0.01, // This must also never reach 0
                    max: 30.0,
                    factor: Range::skew_factor(-2.5),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            .with_value_to_string(formatters::f32_rounded(2)),
            filter_spread_octaves: FloatParam::new(
                "Filter Spread Octaves",
                0.0,
                Range::SymmetricalSkewed {
                    min: -5.0,
                    max: 5.0,
                    factor: Range::skew_factor(-1.0),
                    center: 0.0,
                },
            )
            .with_step_size(0.01)
            .with_smoother(SmoothingStyle::Linear(100.0)),
            filter_spread_style: EnumParam::new("Filter Spread Style", SpreadStyle::Octaves)
                .with_callback(Arc::new(move |_| {
                    should_update_filters.store(true, Ordering::Release)
                })),

            very_important: BoolParam::new("Don't touch this", true).with_value_to_string(
                Arc::new(|value| String::from(if value { "please don't" } else { "stop it" })),
            ),

            automation_precision: FloatParam::new(
                "Automation precision",
                normalize_automation_precision(128),
                Range::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(Arc::new(|value| format!("{:.0}", value * 100.0))),
        }
    }
}

#[derive(Enum, Debug)]
enum SpreadStyle {
    Octaves,
    Linear,
}

impl Plugin for Diopser {
    const NAME: &'static str = "Diopser";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.2.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This works with any symmetrical IO layout
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn initialize(
        &mut self,
        bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl ProcessContext,
    ) -> bool {
        self.filters =
            vec![[Default::default(); MAX_NUM_FILTERS]; bus_config.num_input_channels as usize];

        // Initialize the filters on the first process call
        self.sample_rate = buffer_config.sample_rate;
        self.should_update_filters.store(true, Ordering::Release);

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // Since this is an expensive operation, only update the filters when it's actually
        // necessary, and allow smoothing only every n samples using the automation precision
        // parameter
        let smoothing_interval =
            unnormalize_automation_precision(self.params.automation_precision.value);

        for mut channel_samples in buffer.iter_mut() {
            self.maybe_update_filters(smoothing_interval);

            // We get better cache locality by iterating over the filters and then over the channels
            for filter_idx in 0..self.params.filter_stages.value as usize {
                for (channel_idx, filters) in self.filters.iter_mut().enumerate() {
                    // We can also use `channel_samples.iter_mut()`, but the compiler isn't able to
                    // optmize that iterator away and it would add a ton of overhead over indexing
                    // the buffer directly
                    let sample = unsafe { channel_samples.get_unchecked_mut(channel_idx) };
                    *sample = filters[filter_idx].process(*sample);
                }
            }
        }

        ProcessStatus::Normal
    }
}

impl Diopser {
    /// Check if the filters need to be updated beased on [Self::should_update_filters] and the
    /// smoothing interval, and update them as needed.
    fn maybe_update_filters(&mut self, smoothing_interval: u32) {
        let should_update_filters = self
            .should_update_filters
            .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
            || ((self.params.filter_frequency.smoothed.is_smoothing()
                || self.params.filter_resonance.smoothed.is_smoothing()
                || self.params.filter_spread_octaves.smoothed.is_smoothing())
                && self.next_filter_smoothing_in <= 1);
        if should_update_filters {
            self.update_filters(smoothing_interval);
            self.next_filter_smoothing_in = smoothing_interval as i32;
        } else {
            self.next_filter_smoothing_in -= 1;
        }
    }

    /// Recompute the filter coefficients based on the smoothed paraetersm. We can skip forwardq in
    /// larger steps to reduce the DSP load.
    fn update_filters(&mut self, smoothing_interval: u32) {
        if self.filters.is_empty() {
            return;
        }

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

        const MIN_FREQUENCY: f32 = 5.0;
        let max_frequency = self.sample_rate / 2.05;
        for filter_idx in 0..self.params.filter_stages.value as usize {
            // The index of the filter normalized to range [-1, 1]
            let filter_proportion =
                (filter_idx as f32 / self.params.filter_stages.value as f32) * 2.0 - 1.0;

            // The spread parameter adds an offset to the frequency depending on the number of the
            // filter
            let filter_frequency = match spread_style {
                SpreadStyle::Octaves => frequency * 2.0f32.powf(spread_octaves * filter_proportion),
                SpreadStyle::Linear => frequency * 2.0f32.powf(spread_octaves) * filter_proportion,
            }
            .clamp(MIN_FREQUENCY, max_frequency);

            let coefficients =
                filter::BiquadCoefficients::allpass(self.sample_rate, filter_frequency, resonance);
            for channel in self.filters.iter_mut() {
                channel[filter_idx].coefficients = coefficients;
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

impl Vst3Plugin for Diopser {
    const VST3_CLASS_ID: [u8; 16] = *b"DiopserPlugRvdH.";
    const VST3_CATEGORIES: &'static str = "Fx|Filter";
}

nih_export_vst3!(Diopser);
