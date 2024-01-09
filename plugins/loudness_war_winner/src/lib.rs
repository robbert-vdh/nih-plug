// Loudness War Winner: Because negative LUFS are boring
// Copyright (C) 2022-2024 Robbert van der Helm
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

mod filter;

/// The length of silence after which the signal should start fading out into silence. This is to
/// avoid outputting a constant DC signal.
const SILENCE_FADEOUT_START_MS: f32 = 1000.0;
/// The time it takes after `SILENCE_FADEOUT_START_MS` to fade from a full scale DC signal to silence.
const SILENCE_FADEOUT_END_MS: f32 = SILENCE_FADEOUT_START_MS + 1000.0;

/// The center frequency for our optional bandpass filter, in Hertz.
const BP_FREQUENCY: f32 = 5500.0;

struct LoudnessWarWinner {
    params: Arc<LoudnessWarWinnerParams>,

    sample_rate: f32,
    /// To win even harder we'll band-pass the signal around 5.5 kHz when the `WIN HARDER` parameter
    /// is enabled. And we'll cascade four of these filters while we're at it.
    bp_filters: Vec<[filter::Biquad<f32>; 4]>,

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

    /// When non-zero, this engages a bandpass filter around 5.5 kHz to help with the LUFS
    /// K-Weighting. This is a fraction in `[0, 1]`. [`LoudnessWarWinner::update_bp_filters()`]
    /// calculates the filter's Q value basedo n this.
    #[id = "powah"]
    win_harder_factor: FloatParam,
}

impl Default for LoudnessWarWinner {
    fn default() -> Self {
        Self {
            params: Arc::new(LoudnessWarWinnerParams::default()),

            sample_rate: 1.0,
            bp_filters: Vec::new(),

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
            win_harder_factor: FloatParam::new(
                "WIN HARDER",
                0.0,
                // This ramps up hard, so we'll make sure the 'usable' (for a lack of a better word)
                // value range is larger
                FloatRange::Skewed {
                    min: 0.0,
                    max: 1.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

impl Plugin for LoudnessWarWinner {
    const NAME: &'static str = "Loudness War Winner";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;

        let num_output_channels = audio_io_layout
            .main_output_channels
            .expect("Plugin does not have a main output")
            .get() as usize;
        self.bp_filters
            .resize(num_output_channels, [filter::Biquad::default(); 4]);
        self.update_bp_filters();

        self.silence_fadeout_start_samples =
            (SILENCE_FADEOUT_START_MS / 1000.0 * buffer_config.sample_rate).round() as u32;
        self.silence_fadeout_end_samples =
            (SILENCE_FADEOUT_END_MS / 1000.0 * buffer_config.sample_rate).round() as u32;
        self.silence_fadeout_length_samples =
            self.silence_fadeout_end_samples - self.silence_fadeout_start_samples;

        true
    }

    fn reset(&mut self) {
        for filters in &mut self.bp_filters {
            for filter in filters {
                filter.reset();
            }
        }

        // Start with silence, so we don't immediately output a DC signal if the plugin is inserted
        // on a silent channel
        self.num_silent_samples = self.silence_fadeout_end_samples;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for mut channel_samples in buffer.iter_samples() {
            let output_gain = self.params.output_gain.smoothed.next();

            // When the `WIN_HARDER` parameter is engaged, we'll band-pass the signal around 5 kHz
            if self.params.win_harder_factor.smoothed.is_smoothing() {
                self.update_bp_filters();
            }
            let apply_bp_filters = self.params.win_harder_factor.smoothed.previous_value() > 0.0;

            let mut is_silent = true;
            for (sample, bp_filters) in channel_samples.iter_mut().zip(&mut self.bp_filters) {
                is_silent &= *sample == 0.0;

                // For better performance we can move this conditional to an outer loop, but right
                // now it shouldn't be too bad
                if apply_bp_filters {
                    for filter in bp_filters {
                        *sample = filter.process(*sample);
                    }
                }

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

impl LoudnessWarWinner {
    /// Update the band-pass filters. This should only be called during processing if
    /// `self.params.win_harder_factor.smoothed.is_smoothing()`.
    fn update_bp_filters(&mut self) {
        let q = 0.00001 + (self.params.win_harder_factor.smoothed.next() * 30.0);

        let biquad_coefficients =
            filter::BiquadCoefficients::bandpass(self.sample_rate, BP_FREQUENCY, q);
        for filters in &mut self.bp_filters {
            for filter in filters {
                filter.coefficients = biquad_coefficients;
            }
        }
    }
}

impl ClapPlugin for LoudnessWarWinner {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.loudness-war-winner";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Win the loudness war with ease");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Mono,
        ClapFeature::Limiter,
        ClapFeature::Distortion,
        ClapFeature::Utility,
        ClapFeature::Custom("nih:pain"),
    ];
}

impl Vst3Plugin for LoudnessWarWinner {
    const VST3_CLASS_ID: [u8; 16] = *b"LoudnessWar.RvdH";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Dynamics,
        Vst3SubCategory::Distortion,
        Vst3SubCategory::Custom("Pain"),
    ];
}

nih_export_clap!(LoudnessWarWinner);
nih_export_vst3!(LoudnessWarWinner);
