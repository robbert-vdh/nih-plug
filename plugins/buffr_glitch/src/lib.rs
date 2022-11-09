// Buffr Glitch: a MIDI-controlled buffer repeater
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

struct BuffrGlitch {
    params: Arc<BuffrGlitchParams>,
}

#[derive(Params)]
struct BuffrGlitchParams {}

impl Default for BuffrGlitch {
    fn default() -> Self {
        Self {
            params: Arc::new(BuffrGlitchParams::default()),
        }
    }
}

impl Default for BuffrGlitchParams {
    fn default() -> Self {
        Self {}
    }
}

impl Plugin for BuffrGlitch {
    const NAME: &'static str = "Buffr Glitch";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.1.0";

    const DEFAULT_INPUT_CHANNELS: u32 = 2;
    const DEFAULT_OUTPUT_CHANNELS: u32 = 2;

    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }
}

impl ClapPlugin for BuffrGlitch {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.buffr-glitch";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("MIDI-controller buffer repeat");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Glitch,
    ];
}

impl Vst3Plugin for BuffrGlitch {
    const VST3_CLASS_ID: [u8; 16] = *b"BuffrGlitch.RvdH";
    const VST3_CATEGORIES: &'static str = "Fx";
}

nih_export_clap!(BuffrGlitch);
nih_export_vst3!(BuffrGlitch);
