// Safety limiter: ear protection for the 21st century
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

struct SafetyLimiter {
    params: Arc<SafetyLimiterParams>,
}

#[derive(Params)]
struct SafetyLimiterParams {
    /// The level at which to start engaging the safety limiter. Stored as a gain ratio instead of
    /// decibels.
    #[id = "threshold"]
    threshold_gain: FloatParam,
}

impl Default for SafetyLimiterParams {
    fn default() -> Self {
        Self {
            threshold_gain: FloatParam::new(
                "Threshold",
                util::db_to_gain(0.00),
                // This parameter mostly exists to allow small peaks through, so no need to go below
                // 0 dBFS
                FloatRange::Linear {
                    min: util::db_to_gain(0.0),
                    max: util::db_to_gain(12.0),
                },
            )
            // And smoothing is not necessary here since we'll disable automation
            .non_automatable()
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

impl Default for SafetyLimiter {
    fn default() -> Self {
        SafetyLimiter {
            params: Arc::new(SafetyLimiterParams::default()),
        }
    }
}

impl Plugin for SafetyLimiter {
    const NAME: &'static str = "Safety Limiter";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.1.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        config.num_input_channels == config.num_output_channels
    }

    fn reset(&mut self) {
        // TODO: Reset counters
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // TODO: Dew it

        ProcessStatus::Normal
    }
}

impl ClapPlugin for SafetyLimiter {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.safety-limiter";
    const CLAP_DESCRIPTION: &'static str = "Plays SOS in Morse code when redlining";
    const CLAP_FEATURES: &'static [&'static str] = &["audio_effect", "stereo", "utility"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for SafetyLimiter {
    const VST3_CLASS_ID: [u8; 16] = *b"SafetyLimtrRvdH.";
    const VST3_CATEGORIES: &'static str = "Fx|Tools";
}

nih_export_clap!(SafetyLimiter);
nih_export_vst3!(SafetyLimiter);
