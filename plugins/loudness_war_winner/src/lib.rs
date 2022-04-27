// Loudness War Winner: Because negative LUFS are boring
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

struct LoudnessWarWinner {
    params: Arc<LoudnessWarWinnerParams>,
}

#[derive(Params)]
struct LoudnessWarWinnerParams {
    /// The output gain, set to -24 dB by default because oof ouchie.
    #[id = "output"]
    output_gain: FloatParam,
}

impl Default for LoudnessWarWinner {
    fn default() -> Self {
        Self {
            params: Arc::new(LoudnessWarWinnerParams::default()),
        }
    }
}

impl Default for LoudnessWarWinnerParams {
    fn default() -> Self {
        Self {
            output_gain: FloatParam::new(
                "Output Gain",
                util::db_to_gain(-24.0),
                // Because we're representing gain as decibels the range is already logarithmic
                FloatRange::Linear {
                    min: util::db_to_gain(-24.0),
                    max: util::db_to_gain(0.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(10.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

impl Plugin for LoudnessWarWinner {
    const NAME: &'static str = "Loudness War Winner";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.1.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Arc<dyn Params> {
        // The explicit cast is not needed, but Rust Analyzer gets very upset when you don't do it
        self.params.clone() as Arc<dyn Params>
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn reset(&mut self) {
        // TODO: Keep track of silence samples and reset it here to avoid a DC offset
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_samples() {
            let output_gain = self.params.output_gain.smoothed.next();

            // TODO: Slowly fade back to zero after a period of uninterrupted silence so this
            //       doesn't output a constant DC signal even when the input is silent
            // TODO: Add a second parameter called "WIN HARDER" that bandpasses the signal around 5
            //       kHz
            for sample in channel_samples {
                *sample = if *sample >= 0.0 { 1.0 } else { -1.0 } * output_gain;
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for LoudnessWarWinner {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.loudness-war-winner";
    const CLAP_DESCRIPTION: &'static str = "Win the loudness war with ease";
    const CLAP_FEATURES: &'static [&'static str] = &[
        "audio_effect",
        "stereo",
        "mono",
        "limiter",
        "distortion",
        "utility",
        "pain",
    ];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for LoudnessWarWinner {
    const VST3_CLASS_ID: [u8; 16] = *b"LoudnessWar.RvdH";
    const VST3_CATEGORIES: &'static str = "Fx|Dynamics|Distortion";
}

nih_export_clap!(LoudnessWarWinner);
nih_export_vst3!(LoudnessWarWinner);
