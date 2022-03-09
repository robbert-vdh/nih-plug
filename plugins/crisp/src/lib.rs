// Crisp: a distortion plugin but not quite
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

#[macro_use]
extern crate nih_plug;

use nih_plug::prelude::*;
use pcg::Pcg32iState;
use std::pin::Pin;
use std::sync::Arc;

mod filter;
mod pcg;

/// The number of channels we support. Hardcoded to allow for easier SIMD-ifying in the future.
const NUM_CHANNELS: u32 = 2;

/// These seeds being fixed makes bouncing deterministic.
const INITIAL_PRNG_SEED: Pcg32iState = Pcg32iState::new(69, 420);

/// Allow 100% amount to scale the gain to a bit above 100%, to make the effect even less subtle.
const AMOUNT_GAIN_MULTIPLIER: f32 = 2.0;

/// This plugin essentially layers the sound with another copy of the signal ring modulated with
/// white (or filtered) noise. That other copy of the sound may have a low-pass filter applied to it
/// since this effect just turns into literal noise at high frequencies.
struct Crisp {
    params: Pin<Box<CrispParams>>,

    /// Needed for computing the filter coefficients.
    sample_rate: f32,

    /// A PRNG for generating noise, after that we'll implement PCG ourselves so we can easily
    /// SIMD-ify this in the future.
    prng: Pcg32iState,

    /// Resonant filters for low passing the input signal before RM'ing, to allow this to work with
    /// inputs that already contain a lot of high freuqency content.
    rm_input_lpf: [filter::Biquad<f32>; NUM_CHANNELS as usize],
    /// Resonant filters for high passing the noise signal, to make it even brighter.
    noise_hpf: [filter::Biquad<f32>; NUM_CHANNELS as usize],
}

// TODO: Add more kinds of noise
#[derive(Params)]
pub struct CrispParams {
    /// On a range of `[0, 1]`, how much of the modulated sound to mix in.
    #[id = "amount"]
    amount: FloatParam,
    /// What kind of RM to apply. The preset this was modelled after whether intentional or not only
    /// RMs the positive part of the waveform.
    #[id = "mode"]
    mode: EnumParam<Mode>,
    /// How to handle stereo signals. See [`StereoMode`].
    #[id = "stereo"]
    stereo_mode: EnumParam<StereoMode>,

    /// The cutoff frequency for the low-pass filter applied to the input before RM'ing.
    #[id = "rmlpff"]
    rm_input_lpf_freq: FloatParam,
    /// The Q frequency for the low-pass filter applied to the input before RM'ing.
    #[id = "rmlpfq"]
    rm_input_lpf_q: FloatParam,
    /// The cutoff frequency for the high-pass filter applied to the noise.
    #[id = "nzhpff"]
    noise_hpf_freq: FloatParam,
    /// The Q parameter for the high pass-filter applied to the noise.
    #[id = "nzhpfq"]
    noise_hpf_q: FloatParam,

    /// Output gain, as voltage gain. Displayed in decibels.
    #[id = "output"]
    output_gain: FloatParam,
}

/// Controls the type of modulation to apply.
#[derive(Enum, Debug, PartialEq)]
enum Mode {
    /// RM the entire waveform.
    Crispy,
    /// RM only the positive part of the waveform.
    #[name = "Even Crispier"]
    EvenCrispier,
    /// RM only the negative part of the waveform.
    #[name = "Even Crispier (alt)"]
    EvenCrispierNegated,
}

/// Controls how to handle stereo input.
#[derive(Enum, Debug, PartialEq)]
enum StereoMode {
    /// Use the same noise for both channels.
    Mono,
    /// Use a different noise source per channel.
    Stereo,
}

impl Default for Crisp {
    fn default() -> Self {
        Self {
            params: Box::pin(CrispParams::default()),

            sample_rate: 1.0,

            prng: INITIAL_PRNG_SEED,
            rm_input_lpf: [filter::Biquad::default(); NUM_CHANNELS as usize],
            noise_hpf: [filter::Biquad::default(); NUM_CHANNELS as usize],
        }
    }
}

impl Default for CrispParams {
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self {
            amount: FloatParam::new("Amount", 0.35, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(10.0))
                .with_unit("%")
                .with_value_to_string(formatters::f32_percentage(0))
                .with_string_to_value(formatters::from_f32_percentage()),

            mode: EnumParam::new("Mode", Mode::EvenCrispier),
            stereo_mode: EnumParam::new("Stereo Mode", StereoMode::Stereo),

            rm_input_lpf_freq: FloatParam::new(
                "RM LP Frequency",
                22_000.0,
                FloatRange::Skewed {
                    min: 5.0,
                    max: 22_000.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            .with_unit(" Hz")
            .with_value_to_string(Arc::new(|value| {
                if value >= 22_000.0 {
                    String::from("Disabled")
                } else {
                    format!("{:.0}", value)
                }
            }))
            .with_string_to_value(Arc::new(|string| {
                if string == "Disabled" {
                    Some(22_000.0)
                } else {
                    string.trim().trim_end_matches(" Hz").parse().ok()
                }
            })),
            rm_input_lpf_q: FloatParam::new(
                "RM LP Resonance",
                2.0f32.sqrt() / 2.0,
                FloatRange::Skewed {
                    min: 2.0f32.sqrt() / 2.0,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            .with_value_to_string(formatters::f32_rounded(2)),
            noise_hpf_freq: FloatParam::new(
                "Noise HP Frequency",
                1.0,
                FloatRange::Skewed {
                    min: 5.0,
                    max: 22_000.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            .with_unit(" Hz")
            .with_value_to_string(Arc::new(|value| {
                if value <= 5.0 {
                    String::from("Disabled")
                } else {
                    format!("{:.0}", value)
                }
            }))
            .with_string_to_value(Arc::new(|string| {
                if string == "Disabled" {
                    Some(5.0)
                } else {
                    string.trim().trim_end_matches(" Hz").parse().ok()
                }
            })),
            noise_hpf_q: FloatParam::new(
                "Noise HP Resonance",
                2.0f32.sqrt() / 2.0,
                FloatRange::Skewed {
                    min: 2.0f32.sqrt() / 2.0,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            .with_value_to_string(formatters::f32_rounded(2)),

            output_gain: FloatParam::new(
                "Output",
                1.0,
                // Because we're representing gain as decibels the range is already logarithmic
                FloatRange::Linear {
                    min: util::db_to_gain(-24.0),
                    max: util::db_to_gain(0.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(10.0))
            .with_unit(" dB")
            .with_value_to_string(Arc::new(|value| format!("{:.2}", util::gain_to_db(value))))
            .with_string_to_value(Arc::new(|string| {
                string
                    .trim()
                    .trim_end_matches(" dB")
                    .parse()
                    .ok()
                    .map(util::db_to_gain)
            })),
        }
    }
}

impl Plugin for Crisp {
    const NAME: &'static str = "Crisp";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.1.0";

    const DEFAULT_NUM_INPUTS: u32 = NUM_CHANNELS;
    const DEFAULT_NUM_OUTPUTS: u32 = NUM_CHANNELS;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // We'll add a SIMD version in a bit which only supports stereo
        config.num_input_channels == config.num_output_channels
            && config.num_input_channels == NUM_CHANNELS
    }

    fn initialize(
        &mut self,
        bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl ProcessContext,
    ) -> bool {
        nih_debug_assert_eq!(bus_config.num_input_channels, NUM_CHANNELS);
        nih_debug_assert_eq!(bus_config.num_output_channels, NUM_CHANNELS);
        self.sample_rate = buffer_config.sample_rate;

        true
    }

    fn reset(&mut self) {
        // By using the same seeds each time bouncing can be made deterministic
        self.prng = INITIAL_PRNG_SEED;

        for filter in &mut self.rm_input_lpf {
            filter.reset();
        }
        for filter in &mut self.noise_hpf {
            filter.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_mut() {
            let amount = self.params.amount.smoothed.next() * AMOUNT_GAIN_MULTIPLIER;
            let output_gain = self.params.output_gain.smoothed.next();

            // Controls the pre-RM LPF and the HPF applied to the noise signal
            self.maybe_update_filters();

            // TODO: SIMD-ize this to process both channels at once
            // TODO: Avoid branching twice here. Modern branch predictors are pretty good at this
            //       though.
            match self.params.stereo_mode.value() {
                StereoMode::Mono => {
                    let noise = self.gen_noise(0);
                    for (channel_idx, sample) in channel_samples.into_iter().enumerate() {
                        *sample += self.do_ring_mod(*sample, channel_idx, noise) * amount;
                        *sample *= output_gain;
                    }
                }
                StereoMode::Stereo => {
                    for (channel_idx, sample) in channel_samples.into_iter().enumerate() {
                        let noise = self.gen_noise(channel_idx);
                        *sample += self.do_ring_mod(*sample, channel_idx, noise) * amount;
                        *sample *= output_gain;
                    }
                }
            }
        }

        ProcessStatus::Normal
    }
}

impl Crisp {
    /// Generate a new noise sample with the high pass filter applied.
    fn gen_noise(&mut self, channel: usize) -> f32 {
        let noise = self.prng.next_f32() * 2.0 - 1.0;
        self.noise_hpf[channel].process(noise)
    }

    /// Perform the RM step depending on the mode. This applies a low pass filter to the input
    /// before RM'ing.
    fn do_ring_mod(&mut self, sample: f32, channel_idx: usize, noise: f32) -> f32 {
        let sample = self.rm_input_lpf[channel_idx].process(sample);

        // TODO: Avoid branching in the main loop, this just makes it a bit easier to prototype
        match self.params.mode.value() {
            Mode::Crispy => sample * noise,
            Mode::EvenCrispier => sample.max(0.0) * noise,
            Mode::EvenCrispierNegated => sample.max(0.0) * noise,
        }
    }

    /// Update the filter coefficients if needed. Should be called once per sample.
    fn maybe_update_filters(&mut self) {
        if self.params.rm_input_lpf_freq.smoothed.is_smoothing()
            || self.params.rm_input_lpf_q.smoothed.is_smoothing()
        {
            self.update_rm_input_lpf();
        }
        if self.params.noise_hpf_freq.smoothed.is_smoothing()
            || self.params.noise_hpf_q.smoothed.is_smoothing()
        {
            self.update_noise_hpf();
        }
    }

    /// Update the filter coefficients if needed. Should be called explicitly from `initialize()`.
    fn update_rm_input_lpf(&mut self) {
        let frequency = self.params.rm_input_lpf_freq.smoothed.next();
        let q = self.params.rm_input_lpf_q.smoothed.next();
        let coefficients = filter::BiquadCoefficients::lowpass(self.sample_rate, frequency, q);
        for filter in &mut self.rm_input_lpf {
            filter.coefficients = coefficients;
        }
    }

    /// Update the filter coefficients if needed. Should be called explicitly from `initialize()`.
    fn update_noise_hpf(&mut self) {
        let frequency = self.params.noise_hpf_freq.smoothed.next();
        let q = self.params.noise_hpf_q.smoothed.next();
        let coefficients = filter::BiquadCoefficients::highpass(self.sample_rate, frequency, q);
        for filter in &mut self.noise_hpf {
            filter.coefficients = coefficients;
        }
    }
}

impl ClapPlugin for Crisp {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.crisp";
    const CLAP_DESCRIPTION: &'static str = "Adds a bright crispy top end to low bass sounds";
    const CLAP_FEATURES: &'static [&'static str] =
        &["audio_effect", "stereo", "distortion", "filter"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Crisp {
    const VST3_CLASS_ID: [u8; 16] = *b"CrispPluginRvdH.";
    const VST3_CATEGORIES: &'static str = "Fx|Filter|Distortion";
}

nih_export_clap!(Crisp);
nih_export_vst3!(Crisp);
