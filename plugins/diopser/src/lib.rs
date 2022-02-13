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

use nih_plug::{formatters, BoolParam, FloatParam, IntParam, Params, Range, SmoothingStyle};
use nih_plug::{
    Buffer, BufferConfig, BusConfig, Plugin, ProcessContext, ProcessStatus, Vst3Plugin,
};
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

// An incomplete list of unported features includes:
// - Filter spread
//
// After that the features I want to implement are:
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

    /// The filter's cutoff frequqency. When this is applied, the filters are spread around this
    /// frequency.
    #[id = "cutoff"]
    filter_frequency: FloatParam,
    /// The Q parameter for the filters.
    #[id = "res"]
    filter_resonance: FloatParam,

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
        let trigger_filter_update =
            Arc::new(move |_| should_update_filters.store(true, Ordering::Release));

        Self {
            filter_stages: IntParam::new(
                "Filter Stages",
                0,
                Range::Linear {
                    min: 0,
                    max: MAX_NUM_FILTERS as i32,
                },
            )
            .with_callback(trigger_filter_update),

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

            automation_precision: FloatParam::new(
                "Automation precision",
                normalize_automation_precision(128),
                Range::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(Arc::new(|value| format!("{:.0}", value * 100.0))),

            very_important: BoolParam::new("Don't touch this", true).with_value_to_string(
                Arc::new(|value| String::from(if value { "please don't" } else { "stop it" })),
            ),
        }
    }
}

impl Plugin for Diopser {
    const NAME: &'static str = "Diopser (WIP port)";
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
                // Because of this filter_idx outer loop we can't directly iterate over
                // `channel_samples` as the iterator would be empty after the first loop
                for (sample, filters) in channel_samples.iter_mut().zip(self.filters.iter_mut()) {
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
                || self.params.filter_resonance.smoothed.is_smoothing())
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
        let coefficients = filter::BiquadCoefficients::allpass(
            self.sample_rate,
            self.params
                .filter_frequency
                .smoothed
                .next_step(smoothing_interval),
            self.params
                .filter_resonance
                .smoothed
                .next_step(smoothing_interval),
        );
        for channel in self.filters.iter_mut() {
            for filter in channel
                .iter_mut()
                .take(self.params.filter_stages.value as usize)
            {
                filter.coefficients = coefficients;
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
