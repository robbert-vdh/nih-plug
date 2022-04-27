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

/// The length of silence after which the signal should start fading out into silence. This is to
/// avoid outputting a constant DC signal.
const SILENCE_FADEOUT_START_MS: f32 = 1000.0;
/// The time it takes after `SILENCE_FADEOUT_START_MS` to fade from a full scale DC signal to silence.
const SILENCE_FADEOUT_END_MS: f32 = SILENCE_FADEOUT_START_MS + 1000.0;

struct LoudnessWarWinner {
    params: Arc<LoudnessWarWinnerParams>,

    /// The number of samples since the last non-zero sample. This is used to fade into silence when
    /// the input has also been silent for a while instead of outputting a constant DC signal. All
    /// channels need to be silent for a signal to be considered silent.
    num_silent_samples: u32,
    /// `SILENCE_FADEOUT_START_MS` converted to samples.
    silence_fadeout_start_samples: u32,
    /// `SILENCE_FADEOUT_END_MS` converted to samples.
    silence_fadeout_end_samples: u32,
    /// The length of the fadeout, in samples.
    silence_fadeout_length_samples: u32,
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

            num_silent_samples: 0,
            silence_fadeout_start_samples: 0,
            silence_fadeout_end_samples: 0,
            silence_fadeout_length_samples: 0,
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

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl ProcessContext,
    ) -> bool {
        self.silence_fadeout_start_samples =
            (SILENCE_FADEOUT_START_MS / 1000.0 * buffer_config.sample_rate).round() as u32;
        self.silence_fadeout_end_samples =
            (SILENCE_FADEOUT_END_MS / 1000.0 * buffer_config.sample_rate).round() as u32;
        self.silence_fadeout_length_samples =
            self.silence_fadeout_end_samples - self.silence_fadeout_start_samples;

        true
    }

    fn reset(&mut self) {
        // Start with silence, so we don't immediately output a DC signal if the plugin is inserted
        // on a silent channel
        self.num_silent_samples = self.silence_fadeout_end_samples;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        for mut channel_samples in buffer.iter_samples() {
            let output_gain = self.params.output_gain.smoothed.next();

            // TODO: Add a second parameter called "WIN HARDER" that bandpasses the signal around 5
            //       kHz
            let mut is_silent = true;
            for sample in channel_samples.iter_mut() {
                is_silent &= *sample == 0.0;
                *sample = if *sample >= 0.0 { 1.0 } else { -1.0 } * output_gain;
            }

            // To avoid outputting a constant DC signal even when there's no input we'll slowly fade
            // into silence
            if is_silent {
                self.num_silent_samples += 1;

                if self.num_silent_samples >= self.silence_fadeout_end_samples {
                    for sample in channel_samples {
                        *sample = 0.0;
                    }
                } else if self.num_silent_samples >= self.silence_fadeout_start_samples {
                    let fadeout_gain = 1.0
                        - ((self.num_silent_samples - self.silence_fadeout_start_samples) as f32
                            / self.silence_fadeout_length_samples as f32);

                    for sample in channel_samples {
                        *sample *= fadeout_gain;
                    }
                }
            } else {
                self.num_silent_samples = 0;
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
