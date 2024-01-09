// Safety limiter: ear protection for the 21st century
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
use nih_plug::util::permit_alloc;
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

/// The four second SOS morse code sequence. Each element here represents an edge where the signal
/// is either turned on or off. The first element of each tuple is the time in milliseconds into the
/// sequence, while the second element is the new gate status at that time point. The last element
/// acts as a delay before wrapping around, and it is equivalent to the 0 position in the next cycle
/// (hence why it is set to true).
const MORSE_SEQ_EDGES_MS: [(u32, bool); 19] = [
    // S, 3*100 ms + 2*100ms spacing
    (0, true),
    (100, false),
    (200, true),
    (300, false),
    (400, true),
    // 500 ms silence
    (500, false),
    //
    // O, 3*200 ms + 2*100ms spacing
    (1000, true),
    (1200, false),
    (1400, true),
    (1600, false),
    (1800, true),
    // 500 ms silence
    (2000, false),
    //
    // S, 3*100 ms + 2*100ms spacing
    (2500, true),
    (2600, false),
    (2700, true),
    (2800, false),
    (2900, true),
    // 1000 ms silence
    (3000, false),
    // Acts as a delay at the end before the sequence loops. This sample 4000 behaves like an alias
    // for sample 0 in the next cycle.
    (4000, true),
];

struct SafetyLimiter {
    params: Arc<SafetyLimiterParams>,

    buffer_config: BufferConfig,

    /// `MORSE_FADEOUT_START_MS` translated into samples.
    morse_fadeout_samples_start: u32,
    /// `MORSE_FADEOUT_END_MS` translated into samples.
    morse_fadeout_samples_end: u32,
    /// `MORSE_SEQ_EDGES_MS` translated into samples.
    morse_seq_edges_samples: [(u32, bool); 19],

    /// The number of samples into the fadeout. This resets back to 0 whenever the signal peaks
    /// above the threshold.
    morse_fadeout_samples_current: u32,
    /// The index of the current step into `morse_seq_edges_samples`. This wraps around to zero when
    /// reaching the end of the sequence. This is only reset once the fadeout is fully finished.
    morse_seq_current_step_idx: usize,
    /// The index of the current sample in the morse code qeuence. This wraps around to zero when
    /// reaching the end of the sequence. This is only reset once the fadeout is fully finished.
    morse_seq_current_sample_idx: u32,

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
                FloatRange::Skewed {
                    min: util::db_to_gain(-24.0),
                    max: util::db_to_gain(12.0),
                    factor: FloatRange::gain_skew_factor(-24.0, 12.0),
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
            morse_seq_edges_samples: [(0, false); 19],

            morse_fadeout_samples_current: 0,
            morse_seq_current_sample_idx: 0,
            morse_seq_current_step_idx: 0,

            osc_phase_tau: 0.0,
            osc_phase_tau_dt: 0.0,
        }
    }
}

impl Plugin for SafetyLimiter {
    const NAME: &'static str = "Safety Limiter";
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
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.buffer_config = *buffer_config;
        self.morse_fadeout_samples_start =
            (MORSE_FADEOUT_START_MS / 1000.0 * buffer_config.sample_rate).round() as u32;
        self.morse_fadeout_samples_end =
            (MORSE_FADEOUT_END_MS / 1000.0 * buffer_config.sample_rate).round() as u32;
        self.osc_phase_tau_dt = MORSE_FREQUENCY / buffer_config.sample_rate * std::f32::consts::TAU;

        self.morse_seq_edges_samples = MORSE_SEQ_EDGES_MS.map(|(time_ms, gate)| {
            (
                (time_ms as f32 / 1000.0 * buffer_config.sample_rate).round() as u32,
                gate,
            )
        });

        true
    }

    fn reset(&mut self) {
        self.morse_fadeout_samples_current = self.morse_fadeout_samples_end;
        self.reset_morse_signal();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Don't do anything when bouncing
        if self.buffer_config.process_mode == ProcessMode::Offline {
            return ProcessStatus::Normal;
        }

        // We'll print this once per buffer to make it obvious something very fishy is going on
        // without tanking performance too much
        let mut buffer_contains_nan = false;
        let mut buffer_contains_inf = false;

        let &(morse_seq_len, _) = self.morse_seq_edges_samples.last().unwrap();
        for mut channel_samples in buffer.iter_samples() {
            let mut is_peaking = false;
            for sample in channel_samples.iter_mut() {
                if sample.is_finite() {
                    is_peaking |= sample.abs() > self.params.threshold_gain.value();
                } else {
                    // Infinity or NaN values need to be completely filtered out, because otherwise
                    // we'll try to mix them back into the signal later
                    *sample = 0.0;
                    is_peaking = true;

                    if sample.is_nan() {
                        buffer_contains_nan = true;
                    } else if sample.is_infinite() {
                        buffer_contains_inf = true;
                    } else {
                        unreachable!();
                    }
                }
            }

            if is_peaking {
                // We'll continue playback where it was left off when this gets triggered before the
                // fadeout has finished, but otherwise the sequence should be restarted.
                if self.morse_fadeout_samples_current >= self.morse_fadeout_samples_end {
                    self.reset_morse_signal();
                }

                // This is the number of samples into the fadeout
                self.morse_fadeout_samples_current = 0;
            }

            // Depending on the current gate status in the morse code sequence we'll either play a
            // sine wave oscillator or silence, and the original audio will be faded back in when it
            // stays under the threshold for long enough.
            if self.morse_fadeout_samples_current < self.morse_fadeout_samples_end {
                // Move to the next step when it is reached
                // NOTE: This assumes there are no two edges at the same time, becuase that would be
                //       weird
                // NOTE: Also assumes the sequence starts at 0
                let morse_seq_next_step_idx =
                    (self.morse_seq_current_step_idx + 1) % self.morse_seq_edges_samples.len();
                if self.morse_seq_current_sample_idx
                    >= self.morse_seq_edges_samples[morse_seq_next_step_idx].0
                {
                    self.morse_seq_current_step_idx = morse_seq_next_step_idx;
                }

                // And either play or don't play the sine wave depending on the current step's gate
                // values. We'll wait for the phase wraparound when deactivating the sine wave to
                // avoid clicks.
                let (_, gate) = self.morse_seq_edges_samples[self.morse_seq_current_step_idx];
                let morse_sample = if gate || self.osc_phase_tau > self.osc_phase_tau_dt {
                    // This phase runs from 0 to `2 * pi` as an optimization, so we can use it
                    // directly. And the sine wave is scaled down to the threshold minus 24 dB
                    let sine_sample =
                        self.osc_phase_tau.sin() * (self.params.threshold_gain.value() * 0.125);
                    self.osc_phase_tau += self.osc_phase_tau_dt;
                    if self.osc_phase_tau >= std::f32::consts::TAU {
                        self.osc_phase_tau -= std::f32::consts::TAU;
                    }

                    sine_sample
                } else {
                    0.0
                };

                // We'll do an equal power fade
                let original_t_squared = if self.morse_fadeout_samples_current
                    < self.morse_fadeout_samples_start
                {
                    0.0
                } else {
                    (self.morse_fadeout_samples_current - self.morse_fadeout_samples_start) as f32
                        / (self.morse_fadeout_samples_end - self.morse_fadeout_samples_start) as f32
                };
                let original_t = original_t_squared.sqrt();
                let morse_t = (1.0 - original_t_squared).sqrt();
                for sample in channel_samples {
                    *sample = (morse_sample * morse_t) + (*sample * original_t);
                }

                self.morse_fadeout_samples_current += 1;
                self.morse_seq_current_sample_idx += 1;
                if self.morse_seq_current_sample_idx >= morse_seq_len {
                    self.morse_seq_current_sample_idx -= morse_seq_len;
                    self.morse_seq_current_step_idx = 0;
                }
            }
        }

        if buffer_contains_nan {
            permit_alloc(|| nih_log!("The buffer contains NaN values"));
        }
        if buffer_contains_inf {
            permit_alloc(|| nih_log!("The buffer contains infinite values"));
        }

        ProcessStatus::Normal
    }
}

impl SafetyLimiter {
    /// Reset the SOS signal to the start.
    fn reset_morse_signal(&mut self) {
        self.osc_phase_tau = 0.0;
        self.morse_seq_current_step_idx = 0;
        self.morse_seq_current_sample_idx = 0;
    }
}

impl ClapPlugin for SafetyLimiter {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.safety-limiter";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Plays SOS in Morse code when redlining");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Mono,
        ClapFeature::Utility,
    ];
}

impl Vst3Plugin for SafetyLimiter {
    const VST3_CLASS_ID: [u8; 16] = *b"SafetyLimtrRvdH.";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Tools];
}

nih_export_clap!(SafetyLimiter);
nih_export_vst3!(SafetyLimiter);
