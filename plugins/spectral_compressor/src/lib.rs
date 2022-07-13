// Spectral Compressor: an FFT based compressor
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

use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use std::sync::Arc;

mod editor;

struct SpectralCompressor {
    params: Arc<SpectralCompressorParams>,
    editor_state: Arc<ViziaState>,
}

#[derive(Params, Default)]
struct SpectralCompressorParams {}

impl Default for SpectralCompressor {
    fn default() -> Self {
        Self {
            params: Arc::new(SpectralCompressorParams::default()),
            editor_state: editor::default_state(),
        }
    }
}

impl Plugin for SpectralCompressor {
    const NAME: &'static str = "Spectral Compressor";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.2.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&self) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.editor_state.clone())
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // We can support any channel layout
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // TODO: Do the thing
        ProcessStatus::Normal
    }
}

impl ClapPlugin for SpectralCompressor {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.spectral-compressor";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Turn things into pink noise on demand");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::PhaseVocoder,
        ClapFeature::Compressor,
        ClapFeature::Custom("spectral"),
        ClapFeature::Custom("sosig"),
    ];
}

impl Vst3Plugin for SpectralCompressor {
    const VST3_CLASS_ID: [u8; 16] = *b"SpectrlComprRvdH";
    const VST3_CATEGORIES: &'static str = "Fx|Dynamics|Spectral";
}

nih_export_clap!(SpectralCompressor);
nih_export_vst3!(SpectralCompressor);
