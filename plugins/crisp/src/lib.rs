// Crisp: a distortion plugin but not quite
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
use nih_plug_vizia::ViziaState;
use pcg::Pcg32iState;
use std::sync::Arc;

mod editor;
mod filter;
mod pcg;

/// The number of channels we support. Hardcoded to allow for easier SIMD-ifying in the future.
const NUM_CHANNELS: u32 = 2;
/// The number of samples to iterate over at a time.
const MAX_BLOCK_SIZE: usize = 64;

/// These seeds being fixed makes bouncing deterministic.
const INITIAL_PRNG_SEED: Pcg32iState = Pcg32iState::new(69, 420);

/// Allow 100% amount to scale the gain to a bit above 100%, to make the effect even less subtle.
const AMOUNT_GAIN_MULTIPLIER: f32 = 2.0;
const MIN_FILTER_FREQUENCY: f32 = 5.0;
const MAX_FILTER_FREQUENCY: f32 = 22_000.0;

/// This plugin essentially layers the sound with another copy of the signal ring modulated with
/// white (or filtered) noise. That other copy of the sound may have a low-pass filter applied to it
/// since this effect just turns into literal noise at high frequencies.
pub struct Crisp {
    params: Arc<CrispParams>,

    /// Needed for computing the filter coefficients.
    sample_rate: f32,

    /// A PRNG for generating noise, after that we'll implement PCG ourselves so we can easily
    /// SIMD-ify this in the future.
    prng: Pcg32iState,

    /// Resonant filters for low passing the input signal before RM'ing, to allow this to work with
    /// inputs that already contain a lot of high freuqency content.
    rm_input_lpf: [filter::Biquad<f32>; NUM_CHANNELS as usize],
    /// Resonant filters for high- and then low- passing the noise signal, to make it even brighter.
    noise_hpf: [filter::Biquad<f32>; NUM_CHANNELS as usize],
    noise_lpf: [filter::Biquad<f32>; NUM_CHANNELS as usize],
}

#[derive(Params)]
struct CrispParams {
    /// The editor state, saved together with the parameter state so the custom scaling can be
    /// restored.
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,

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
    /// The cutoff frequency for the low-pass filter applied to the noise.
    #[id = "nzlpff"]
    noise_lpf_freq: FloatParam,
    /// The Q parameter for the low pass-filter applied to the noise.
    #[id = "nzlpfq"]
    noise_lpf_q: FloatParam,

    /// Output gain, as voltage gain. Displayed in decibels.
    #[id = "output"]
    output_gain: FloatParam,
    /// If set, only output the RM'ed signal. Can be useful for further processing.
    #[id = "wtonly"]
    wet_only: BoolParam,
}

/// Controls the type of modulation to apply.
#[derive(Enum, Debug, PartialEq)]
enum Mode {
    /// RM the entire waveform.
    #[id = "soggy"]
    Soggy,
    /// RM only the positive part of the waveform.
    #[id = "crispy"]
    Crispy,
    /// RM only the negative part of the waveform.
    #[id = "crispy-negated"]
    #[name = "Crispy (alt)"]
    CrispyNegated,
}

/// Controls how to handle stereo input.
#[derive(Enum, Debug, PartialEq)]
enum StereoMode {
    /// Use the same noise for both channels.
    #[id = "mono"]
    Mono,
    /// Use a different noise source per channel.
    #[id = "stereo"]
    Stereo,
}

impl Default for Crisp {
    fn default() -> Self {
        Self {
            params: Arc::new(CrispParams::default()),

            sample_rate: 1.0,

            prng: INITIAL_PRNG_SEED,
            rm_input_lpf: [filter::Biquad::default(); NUM_CHANNELS as usize],
            noise_hpf: [filter::Biquad::default(); NUM_CHANNELS as usize],
            noise_lpf: [filter::Biquad::default(); NUM_CHANNELS as usize],
        }
    }
}

impl Default for CrispParams {
    fn default() -> Self {
        let f32_hz_then_khz = formatters::v2s_f32_hz_then_khz(0);
        let from_f32_hz_then_khz = formatters::s2v_f32_hz_then_khz();

        Self {
            editor_state: editor::default_state(),

            amount: FloatParam::new("Amount", 0.35, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(10.0))
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            mode: EnumParam::new("Mode", Mode::Crispy),
            stereo_mode: EnumParam::new("Stereo Mode", StereoMode::Stereo),

            rm_input_lpf_freq: FloatParam::new(
                "RM LP Frequency",
                MAX_FILTER_FREQUENCY,
                FloatRange::Skewed {
                    min: MIN_FILTER_FREQUENCY,
                    max: MAX_FILTER_FREQUENCY,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            // The unit is baked into the value so we can show the disabled string
            .with_value_to_string(Arc::new(|value| {
                if value >= MAX_FILTER_FREQUENCY {
                    String::from("Disabled")
                } else {
                    format!("{value:.0} Hz")
                }
            }))
            .with_string_to_value(Arc::new(|string| {
                if string == "Disabled" {
                    Some(MAX_FILTER_FREQUENCY)
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
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            noise_hpf_freq: FloatParam::new(
                "Noise HP Frequency",
                MIN_FILTER_FREQUENCY,
                FloatRange::Skewed {
                    min: MIN_FILTER_FREQUENCY,
                    max: MAX_FILTER_FREQUENCY,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            // The unit is baked into the value so we can show the disabled string
            .with_value_to_string({
                let f32_hz_then_khz = f32_hz_then_khz.clone();
                Arc::new(move |value| {
                    if value <= MIN_FILTER_FREQUENCY {
                        String::from("Disabled")
                    } else {
                        f32_hz_then_khz(value)
                    }
                })
            })
            .with_string_to_value({
                let from_f32_hz_then_khz = from_f32_hz_then_khz.clone();
                Arc::new(move |string| {
                    if string == "Disabled" {
                        Some(MIN_FILTER_FREQUENCY)
                    } else {
                        from_f32_hz_then_khz(string)
                    }
                })
            }),
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
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            noise_lpf_freq: FloatParam::new(
                "Noise LP Frequency",
                MAX_FILTER_FREQUENCY,
                FloatRange::Skewed {
                    min: MIN_FILTER_FREQUENCY,
                    max: MAX_FILTER_FREQUENCY,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            // The unit is baked into the value so we can show the disabled string
            .with_value_to_string(Arc::new(move |value| {
                if value >= MAX_FILTER_FREQUENCY {
                    String::from("Disabled")
                } else {
                    f32_hz_then_khz(value)
                }
            }))
            .with_string_to_value(Arc::new(move |string| {
                if string == "Disabled" {
                    Some(MAX_FILTER_FREQUENCY)
                } else {
                    from_f32_hz_then_khz(string)
                }
            })),
            noise_lpf_q: FloatParam::new(
                "Noise LP Resonance",
                2.0f32.sqrt() / 2.0,
                FloatRange::Skewed {
                    min: 2.0f32.sqrt() / 2.0,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(100.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

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
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            wet_only: BoolParam::new("Wet Only", false),
        }
    }
}

impl Plugin for Crisp {
    const NAME: &'static str = "Crisp";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // We'll add a SIMD version in a bit which only supports stereo
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(NUM_CHANNELS),
        main_output_channels: NonZeroU32::new(NUM_CHANNELS),
        ..AudioIOLayout::const_default()
    }];

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.params.editor_state.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;

        // The filter coefficients need to be reinitialized when loading a patch
        self.update_rm_input_lpf();
        self.update_noise_hpf();
        self.update_noise_lpf();

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
        for filter in &mut self.noise_lpf {
            filter.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for (_, mut block) in buffer.iter_blocks(MAX_BLOCK_SIZE) {
            let mut rm_outputs = [[0.0; NUM_CHANNELS as usize]; MAX_BLOCK_SIZE];

            // Reduce per-sample branching a bit by iterating over smaller blocks and only then
            // deciding what to do with the output. This version branches only once per sample (in
            // `do_ring_mod()`) which can be trivially optimized to a masked min/max later.
            // TODO: SIMD-ize this to process both channels at once
            match self.params.stereo_mode.value() {
                StereoMode::Mono => {
                    for (channel_samples, rm_outputs) in block.iter_samples().zip(&mut rm_outputs) {
                        let amount = self.params.amount.smoothed.next() * AMOUNT_GAIN_MULTIPLIER;

                        // Controls the pre-RM LPF and the HPF applied to the noise signal
                        self.maybe_update_filters();

                        let noise = self.gen_noise(0);
                        for (channel_idx, (sample, rm_output)) in
                            channel_samples.into_iter().zip(rm_outputs).enumerate()
                        {
                            *rm_output = self.do_ring_mod(*sample, channel_idx, noise) * amount;
                        }
                    }
                }
                StereoMode::Stereo => {
                    for (channel_samples, rm_outputs) in block.iter_samples().zip(&mut rm_outputs) {
                        let amount = self.params.amount.smoothed.next() * AMOUNT_GAIN_MULTIPLIER;
                        self.maybe_update_filters();
                        for (channel_idx, (sample, rm_output)) in
                            channel_samples.into_iter().zip(rm_outputs).enumerate()
                        {
                            let noise = self.gen_noise(channel_idx);
                            *rm_output = self.do_ring_mod(*sample, channel_idx, noise) * amount;
                        }
                    }
                }
            }

            if self.params.wet_only.value() {
                for (channel_samples, rm_outputs) in block.iter_samples().zip(&mut rm_outputs) {
                    let output_gain = self.params.output_gain.smoothed.next();
                    for (sample, rm_output) in channel_samples.into_iter().zip(rm_outputs) {
                        *sample = *rm_output * output_gain;
                    }
                }
            } else {
                for (channel_samples, rm_outputs) in block.iter_samples().zip(&mut rm_outputs) {
                    let output_gain = self.params.output_gain.smoothed.next();
                    for (sample, rm_output) in channel_samples.into_iter().zip(rm_outputs) {
                        *sample = (*sample + *rm_output) * output_gain;
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
        let high_passed = self.noise_hpf[channel].process(noise);
        self.noise_lpf[channel].process(high_passed)
    }

    /// Perform the RM step depending on the mode. This applies a low pass filter to the input
    /// before RM'ing.
    fn do_ring_mod(&mut self, sample: f32, channel_idx: usize, noise: f32) -> f32 {
        let sample = self.rm_input_lpf[channel_idx].process(sample);

        // TODO: Avoid branching in the main loop, this just makes it a bit easier to prototype
        match self.params.mode.value() {
            Mode::Soggy => sample * noise,
            Mode::Crispy => sample.max(0.0) * noise,
            Mode::CrispyNegated => sample.max(0.0) * noise,
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
        if self.params.noise_lpf_freq.smoothed.is_smoothing()
            || self.params.noise_lpf_q.smoothed.is_smoothing()
        {
            self.update_noise_lpf();
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

    /// Update the filter coefficients if needed. Should be called explicitly from `initialize()`.
    fn update_noise_lpf(&mut self) {
        let frequency = self.params.noise_lpf_freq.smoothed.next();
        let q = self.params.noise_lpf_q.smoothed.next();
        let coefficients = filter::BiquadCoefficients::lowpass(self.sample_rate, frequency, q);
        for filter in &mut self.noise_lpf {
            filter.coefficients = coefficients;
        }
    }
}

impl ClapPlugin for Crisp {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.crisp";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Adds a bright crispy top end to low bass sounds");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Distortion,
    ];
}

impl Vst3Plugin for Crisp {
    const VST3_CLASS_ID: [u8; 16] = *b"CrispPluginRvdH.";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Filter,
        Vst3SubCategory::Distortion,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(Crisp);
nih_export_vst3!(Crisp);
