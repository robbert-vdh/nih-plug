// Crossover: clean crossovers as a multi-out plugin
// Copyright (C) 2022 Robbert van der Helm
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

use crossover::iir::{IirCrossover, IirCrossoverType};
use nih_plug::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod biquad;
mod crossover;

/// The number of bands. Not used directly here, but this avoids hardcoding some constants in the
/// crossover implementations.
pub const NUM_BANDS: usize = 5;

const MIN_CROSSOVER_FREQUENCY: f32 = 40.0;
const MAX_CROSSOVER_FREQUENCY: f32 = 20_000.0;

struct Crossover {
    params: Arc<CrossoverParams>,

    buffer_config: BufferConfig,

    /// Provides the LR24 crossover.
    iir_crossover: IirCrossover,
    /// Set when the number of bands has changed and the filters must be updated.
    should_update_filters: Arc<AtomicBool>,
}

// TODO: Add multiple crossover types. Haven't added the control for that yet because the current
//       type (LR24) would become the second one in the list, and EnumParams are keyed by index so
//       then we'd have an LR12 doing nothing instead. Aside form those two LR48 and some linear
//       phase crossovers would also be nice
#[derive(Params)]
struct CrossoverParams {
    /// The number of bands between 2 and 5
    #[id = "bandcnt"]
    pub num_bands: IntParam,

    // We'll only provide frequency controls, as gain, panning, solo, mute etc. is all already
    // provided by Bitwig's UI
    #[id = "xov1fq"]
    pub crossover_1_freq: FloatParam,
    #[id = "xov2fq"]
    pub crossover_2_freq: FloatParam,
    #[id = "xov3fq"]
    pub crossover_3_freq: FloatParam,
    #[id = "xov4fq"]
    pub crossover_4_freq: FloatParam,
}

impl CrossoverParams {
    fn new(should_update_filters: Arc<AtomicBool>) -> Self {
        let crossover_range = FloatRange::Skewed {
            min: MIN_CROSSOVER_FREQUENCY,
            max: MAX_CROSSOVER_FREQUENCY,
            factor: FloatRange::skew_factor(-1.0),
        };
        let crossover_smoothing_style = SmoothingStyle::Logarithmic(100.0);
        let crossover_value_to_string = formatters::v2s_f32_hz_then_khz(0);
        let crossover_string_to_value = formatters::s2v_f32_hz_then_khz();

        Self {
            num_bands: IntParam::new(
                "Band Count",
                2,
                IntRange::Linear {
                    min: 2,
                    max: NUM_BANDS as i32,
                },
            )
            .with_callback(Arc::new(move |_| {
                should_update_filters.store(true, Ordering::Relaxed)
            })),
            // TODO: More sensible default frequencies
            crossover_1_freq: FloatParam::new("Crossover 1", 200.0, crossover_range)
                .with_smoother(crossover_smoothing_style)
                .with_value_to_string(crossover_value_to_string.clone())
                .with_string_to_value(crossover_string_to_value.clone()),
            crossover_2_freq: FloatParam::new("Crossover 2", 1000.0, crossover_range)
                .with_smoother(crossover_smoothing_style)
                .with_value_to_string(crossover_value_to_string.clone())
                .with_string_to_value(crossover_string_to_value.clone()),
            crossover_3_freq: FloatParam::new("Crossover 3", 5000.0, crossover_range)
                .with_smoother(crossover_smoothing_style)
                .with_value_to_string(crossover_value_to_string.clone())
                .with_string_to_value(crossover_string_to_value.clone()),
            crossover_4_freq: FloatParam::new("Crossover 4", 10000.0, crossover_range)
                .with_smoother(crossover_smoothing_style)
                .with_value_to_string(crossover_value_to_string.clone())
                .with_string_to_value(crossover_string_to_value.clone()),
        }
    }
}

impl Default for Crossover {
    fn default() -> Self {
        let should_update_filters = Arc::new(AtomicBool::new(false));

        Crossover {
            params: Arc::new(CrossoverParams::new(should_update_filters.clone())),

            buffer_config: BufferConfig {
                sample_rate: 1.0,
                min_buffer_size: None,
                max_buffer_size: 0,
                process_mode: ProcessMode::Realtime,
            },

            iir_crossover: IirCrossover::new(IirCrossoverType::LinkwitzRiley24),
            should_update_filters,
        }
    }
}

impl Plugin for Crossover {
    const NAME: &'static str = "Crossover";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.1.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const DEFAULT_AUX_OUTPUTS: Option<AuxiliaryIOConfig> = Some(AuxiliaryIOConfig {
        // Two to five of these busses will be used at a time
        num_busses: 5,
        num_channels: 2,
    });

    const PORT_NAMES: PortNames = PortNames {
        main_input: None,
        // We won't output any sound here
        main_output: Some("The Void"),
        aux_inputs: None,
        aux_outputs: Some(&["Band 1", "Band 2", "Band 3", "Band 4", "Band 5"]),
    };

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // Only do stereo
        config.num_input_channels == 2
            && config.num_output_channels == 2
            && config.aux_output_busses.num_channels == 2
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext,
    ) -> bool {
        self.buffer_config = *buffer_config;

        // Make sure the filter states match the current parameters
        self.update_filters();

        true
    }

    fn reset(&mut self) {
        self.iir_crossover.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        let aux_outputs = &mut aux.outputs;
        let (band_1_buffer, aux_outputs) = aux_outputs.split_first_mut().unwrap();
        let (band_2_buffer, aux_outputs) = aux_outputs.split_first_mut().unwrap();
        let (band_3_buffer, aux_outputs) = aux_outputs.split_first_mut().unwrap();
        let (band_4_buffer, aux_outputs) = aux_outputs.split_first_mut().unwrap();
        let (band_5_buffer, _) = aux_outputs.split_first_mut().unwrap();

        // Snoclists for days
        for (
            (
                (
                    ((main_channel_samples, band_1_channel_samples), band_2_channel_samples),
                    band_3_channel_samples,
                ),
                band_4_channel_samples,
            ),
            band_5_channel_samples,
        ) in buffer
            .iter_samples()
            .zip(band_1_buffer.iter_samples())
            .zip(band_2_buffer.iter_samples())
            .zip(band_3_buffer.iter_samples())
            .zip(band_4_buffer.iter_samples())
            .zip(band_5_buffer.iter_samples())
        {
            // We can avoid a lot of hardcoding and conditionals by restoring the original array structure
            let bands = [
                band_1_channel_samples,
                band_2_channel_samples,
                band_3_channel_samples,
                band_4_channel_samples,
                band_5_channel_samples,
            ];

            // Only update the filters when needed
            self.maybe_update_filters();

            self.iir_crossover.process(
                self.params.num_bands.value as usize,
                &main_channel_samples,
                bands,
            );

            // The main output should be silent as the signal is already evenly split over the other
            // bands
            for sample in main_channel_samples {
                *sample = 0.0;
            }
        }

        ProcessStatus::Normal
    }
}

impl Crossover {
    /// Update the filter coefficients for the crossovers, but only if it's needed.
    fn maybe_update_filters(&mut self) {
        if self
            .should_update_filters
            .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
            || self.params.crossover_1_freq.smoothed.is_smoothing()
            || self.params.crossover_2_freq.smoothed.is_smoothing()
            || self.params.crossover_3_freq.smoothed.is_smoothing()
            || self.params.crossover_4_freq.smoothed.is_smoothing()
        {
            self.update_filters();
        }
    }

    /// Update the filter coefficients for the crossovers.
    fn update_filters(&mut self) {
        // This function will take care of non-monotonic crossover frequencies for us, e.g.
        // crossover 2 being lower than crossover 1
        self.iir_crossover.update(
            self.buffer_config.sample_rate,
            self.params.num_bands.value as usize,
            [
                self.params.crossover_1_freq.smoothed.next(),
                self.params.crossover_2_freq.smoothed.next(),
                self.params.crossover_3_freq.smoothed.next(),
                self.params.crossover_4_freq.smoothed.next(),
            ],
        )
    }
}

impl ClapPlugin for Crossover {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.crossover";
    const CLAP_DESCRIPTION: &'static str = "Cleanly split a signal into multiple bands";
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Utility,
    ];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Crossover {
    const VST3_CLASS_ID: [u8; 16] = *b"CrossoverRvdH...";
    const VST3_CATEGORIES: &'static str = "Fx|Tools";
}

nih_export_clap!(Crossover);
nih_export_vst3!(Crossover);
