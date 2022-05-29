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

use nih_plug::prelude::*;
use std::sync::Arc;

struct Crossover {
    params: Arc<CrossoverParams>,
}

#[derive(Params)]
struct CrossoverParams {
    // TODO:
}

impl Default for CrossoverParams {
    fn default() -> Self {
        Self {}
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
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // TODO

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
