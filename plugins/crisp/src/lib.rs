// Crisp: a distortion plugin but not quite
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
use std::pin::Pin;

struct Crisp {
    params: Pin<Box<CrispParams>>,
}

#[derive(Params)]
pub struct CrispParams {}

impl Default for Crisp {
    fn default() -> Self {
        Self {
            params: Box::pin(CrispParams::default()),
        }
    }
}

impl Default for CrispParams {
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self {}
    }
}

impl Plugin for Crisp {
    const NAME: &'static str = "Crisp";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.1.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // We'll add a SIMD version in a bit which only supports stereo
        config.num_input_channels == config.num_output_channels && config.num_input_channels == 2
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Crisp {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.crisp";
    const CLAP_DESCRIPTION: &'static str = "Adds a bright crispy top end to low bass sounds";
    const CLAP_FEATURES: &'static [&'static str] =
        &["audio_effect", "stereo", "filter", "distortion"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Crisp {
    const VST3_CLASS_ID: [u8; 16] = *b"CrispPluginRvdH.";
    const VST3_CATEGORIES: &'static str = "Fx|Filter|Distortion";
}

nih_export_clap!(Crisp);
nih_export_vst3!(Crisp);
