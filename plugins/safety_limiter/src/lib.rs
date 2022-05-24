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

/// After reaching the threshold, it will take this many milliseconds under that threshold to start
/// fading back to the normal signal. Peaking above the threshold again during this time resets
/// this. The fadeout doesn't start immediately since that would add some nasty distortion when most
/// but not all samples pass the threshold.
const MORSE_FADEOUT_START_MS: f32 = 500.0;
/// The Morse fadeout ends after this many milliseconds.
const MORSE_FADEOUT_END_MS: f32 = MORSE_FADEOUT_START_MS + 1500.0;
/// The frequency of the sine wave used for the SOS signal.
const MORSE_FREQUENCY: f32 = 420.0;

struct SafetyLimiter {
    params: Arc<SafetyLimiterParams>,

    buffer_config: BufferConfig,

    /// `MORSE_FADEOUT_START_MS` translated into samples.
    morse_fadeout_samples_start: u32,
    /// `MORSE_FADEOUT_END_MS` translated into samples.
    morse_fadeout_samples_end: u32,
    /// The number of samples into the fadeout.
    morse_fadeout_samples_current: u32,

    /// The phase of the Morse code sine oscillator. This runs from zero to `2 * pi` for
    /// efficiency's sake.
    osc_phase_tau: f32,
    /// The phase increment for every sample. This can be precomputed since the frequency is fixed.
    osc_phase_tau_dt: f32,
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

            buffer_config: BufferConfig {
                sample_rate: 1.0,
                min_buffer_size: None,
                max_buffer_size: 0,
                process_mode: ProcessMode::Realtime,
            },

            morse_fadeout_samples_start: 0,
            morse_fadeout_samples_end: 0,
            morse_fadeout_samples_current: 0,

            osc_phase_tau: 0.0,
            osc_phase_tau_dt: 0.0,
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

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl ProcessContext,
    ) -> bool {
        self.buffer_config = *buffer_config;
        self.morse_fadeout_samples_start =
            (MORSE_FADEOUT_START_MS / 1000.0 * buffer_config.sample_rate).round() as u32;
        self.morse_fadeout_samples_end =
            (MORSE_FADEOUT_END_MS / 1000.0 * buffer_config.sample_rate).round() as u32;
        self.osc_phase_tau_dt = MORSE_FREQUENCY / buffer_config.sample_rate * std::f32::consts::TAU;

        true
    }

    fn reset(&mut self) {
        self.morse_fadeout_samples_current = self.morse_fadeout_samples_end;
        self.reset_morse_signal();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // Don't do anything when bouncing
        if self.buffer_config.process_mode == ProcessMode::Offline {
            return ProcessStatus::Normal;
        }

        for mut channel_samples in buffer.iter_samples() {
            let mut is_peaking = false;
            for sample in channel_samples.iter_mut() {
                is_peaking |= sample.abs() > self.params.threshold_gain.value;
            }

            // TODO: Do the morse code thing, right now this just fades to silence
            if is_peaking {
                // We'll continue playback where it was left off when this gets triggered before the
                // fadeout has finished, but otherwise the sequence should be restarted.
                if self.morse_fadeout_samples_current >= self.morse_fadeout_samples_end {
                    self.reset_morse_signal();
                }

                // This is the number of samples into the fadeout
                self.morse_fadeout_samples_current = 0;
            }

            if self.morse_fadeout_samples_current < self.morse_fadeout_samples_end {
                // This phase runs from 0 to `2 * pi` as an optimization, so we can use it directly.
                // And the sine wave is scaled down to the threshold minus 12 dB
                let sine_wave =
                    self.osc_phase_tau.sin() * (self.params.threshold_gain.value * 0.25);
                self.osc_phase_tau += self.osc_phase_tau_dt;
                if self.osc_phase_tau >= std::f32::consts::TAU {
                    self.osc_phase_tau -= std::f32::consts::TAU;
                }

                let original_t = if self.morse_fadeout_samples_current
                    < self.morse_fadeout_samples_start
                {
                    0.0
                } else {
                    (self.morse_fadeout_samples_current - self.morse_fadeout_samples_start) as f32
                        / (self.morse_fadeout_samples_end - self.morse_fadeout_samples_start) as f32
                };
                let morse_t = 1.0 - original_t;
                for sample in channel_samples {
                    *sample = (sine_wave * morse_t) + (*sample * original_t);
                }

                self.morse_fadeout_samples_current += 1;
            }
        }

        ProcessStatus::Normal
    }
}

impl SafetyLimiter {
    /// Reset the SOS signal to the start.
    fn reset_morse_signal(&mut self) {
        self.osc_phase_tau = 0.0;
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
