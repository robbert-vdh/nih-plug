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

use nih_plug::prelude::*;
use std::sync::Arc;

const MIN_CROSSOVER_FREQUENCY: f32 = 40.0;
const MAX_CROSSOVER_FREQUENCY: f32 = 20_000.0;

struct Crossover {
    params: Arc<CrossoverParams>,
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

impl Default for CrossoverParams {
    fn default() -> Self {
        let crossover_range = FloatRange::Skewed {
            min: MIN_CROSSOVER_FREQUENCY,
            max: MAX_CROSSOVER_FREQUENCY,
            factor: FloatRange::skew_factor(-1.0),
        };
        let crossover_smoothing_style = SmoothingStyle::Logarithmic(100.0);
        let crossover_value_to_string = formatters::v2s_f32_hz_then_khz(0);
        let crossover_string_to_value = formatters::s2v_f32_hz_then_khz();

        Self {
            num_bands: IntParam::new("Band Count", 2, IntRange::Linear { min: 2, max: 5 }),
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
        Crossover {
            params: Arc::new(CrossoverParams::default()),
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
        _buffer_config: &BufferConfig,
        _context: &mut impl InitContext,
    ) -> bool {
        // TODO: Setup filters
        true
    }

    fn reset(&mut self) {
        // TODO: Reset filters
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // TODO: Do the splitty thing

        // The main output should be silent as the signal is already evenly split over the other
        // bands
        for channel_slice in buffer.as_slice() {
            channel_slice.fill(0.0);
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Crossover {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.crossover";
    const CLAP_DESCRIPTION: &'static str = "Cleanly split a signal into multiple bands";
    const CLAP_FEATURES: &'static [&'static str] = &["audio_effect", "stereo", "utility"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Crossover {
    const VST3_CLASS_ID: [u8; 16] = *b"CrossoverRvdH...";
    const VST3_CATEGORIES: &'static str = "Fx|Tools";
}

nih_export_clap!(Crossover);
nih_export_vst3!(Crossover);
