// Puberty Simulator: the next generation in voice change simulation technology
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
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::f32;
use std::sync::Arc;

const MIN_WINDOW_ORDER: usize = 6;
#[allow(dead_code)]
const MIN_WINDOW_SIZE: usize = 1 << MIN_WINDOW_ORDER; // 64
const DEFAULT_WINDOW_ORDER: usize = 10;
#[allow(dead_code)]
const DEFAULT_WINDOW_SIZE: usize = 1 << DEFAULT_WINDOW_ORDER; // 1024
const MAX_WINDOW_ORDER: usize = 15;
const MAX_WINDOW_SIZE: usize = 1 << MAX_WINDOW_ORDER; // 32768

const MIN_OVERLAP_ORDER: usize = 2;
#[allow(dead_code)]
const MIN_OVERLAP_TIMES: usize = 1 << MIN_OVERLAP_ORDER; // 4
const DEFAULT_OVERLAP_ORDER: usize = 3;
#[allow(dead_code)]
const DEFAULT_OVERLAP_TIMES: usize = 1 << DEFAULT_OVERLAP_ORDER; // 8
const MAX_OVERLAP_ORDER: usize = 5;
#[allow(dead_code)]
const MAX_OVERLAP_TIMES: usize = 1 << MAX_OVERLAP_ORDER; // 32

struct PubertySimulator {
    params: Arc<PubertySimulatorParams>,

    /// An adapter that performs most of the overlap-add algorithm for us.
    stft: util::StftHelper,
    /// Contains a Hann window function of the current window length, passed to the overlap-add
    /// helper. Allocated with a `MAX_WINDOW_SIZE` initial capacity.
    window_function: Vec<f32>,

    /// The algorithms for the FFT and IFFT operations, for each supported order so we can switch
    /// between them without replanning or allocations. Initialized during `initialize()`.
    plan_for_order: Option<[Plan; MAX_WINDOW_ORDER - MIN_WINDOW_ORDER + 1]>,
    /// The output of our real->complex FFT.
    complex_fft_buffer: Vec<Complex32>,
}

/// A plan for a specific window size, all of which will be precomputed during initilaization.
struct Plan {
    /// The algorithm for the FFT operation.
    r2c_plan: Arc<dyn RealToComplex<f32>>,
    /// The algorithm for the IFFT operation.
    c2r_plan: Arc<dyn ComplexToReal<f32>>,
}

#[derive(Params)]
struct PubertySimulatorParams {
    /// The pitch change in octaves.
    #[id = "pitch"]
    pitch_octaves: FloatParam,

    /// The size of the FFT window as a power of two (to prevent invalid inputs).
    #[id = "wndsz"]
    window_size_order: IntParam,
    /// The amount of overlap to use in the overlap-add algorithm as a power of two (again to
    /// prevent invalid inputs).
    #[id = "ovrlap"]
    overlap_times_order: IntParam,

    /// The type of broken pitch shifting to apply.
    #[id = "mode"]
    mode: EnumParam<PitchShiftingMode>,
}

#[derive(Enum, Debug, PartialEq)]
enum PitchShiftingMode {
    /// Directly linearly interpolate sine and cosine waves from different bins. This obviously
    /// sounds very bad, but it also sounds kind of hilarious.
    #[id = "interpolated-rectangular"]
    #[name = "Very broken"]
    InterpolateRectangular,
    /// The same as `InterpolateRectangular`, but interpolating the polar forms instead. This sounds
    /// slightly better, which actually ends up making it sound a lot worse.
    #[id = "interpolated-polar"]
    #[name = "Also very broken"]
    InterpolatePolar,
}

impl Default for PubertySimulator {
    fn default() -> Self {
        Self {
            params: Arc::new(PubertySimulatorParams::default()),

            stft: util::StftHelper::new(2, MAX_WINDOW_SIZE, 0),
            window_function: Vec::with_capacity(MAX_WINDOW_SIZE),

            plan_for_order: None,
            complex_fft_buffer: Vec::with_capacity(MAX_WINDOW_SIZE / 2 + 1),
        }
    }
}

impl Default for PubertySimulatorParams {
    fn default() -> Self {
        let power_of_two_val2str = formatters::v2s_i32_power_of_two();
        let power_of_two_str2val = formatters::s2v_i32_power_of_two();

        Self {
            pitch_octaves: FloatParam::new(
                "Pitch",
                -1.0,
                FloatRange::SymmetricalSkewed {
                    min: -5.0,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-2.0),
                    center: 0.0,
                },
            )
            // This doesn't need smoothing to prevent zippers because we're already going
            // overlap-add, but sounds kind of slick
            .with_smoother(SmoothingStyle::Linear(100.0))
            .with_unit(" Octaves")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            window_size_order: IntParam::new(
                "Window Size",
                DEFAULT_WINDOW_ORDER as i32,
                IntRange::Linear {
                    min: MIN_WINDOW_ORDER as i32,
                    max: MAX_WINDOW_ORDER as i32,
                },
            )
            .with_value_to_string(power_of_two_val2str.clone())
            .with_string_to_value(power_of_two_str2val.clone()),
            overlap_times_order: IntParam::new(
                "Window Overlap",
                DEFAULT_OVERLAP_ORDER as i32,
                IntRange::Linear {
                    min: MIN_OVERLAP_ORDER as i32,
                    max: MAX_OVERLAP_ORDER as i32,
                },
            )
            .with_value_to_string(power_of_two_val2str)
            .with_string_to_value(power_of_two_str2val),
            mode: EnumParam::new("Mode", PitchShiftingMode::InterpolateRectangular),
        }
    }
}

impl Plugin for PubertySimulator {
    const NAME: &'static str = "Puberty Simulator";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // We'll only do stereo for simplicity's sake
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        // Planning with RustFFT is very fast, but it will still allocate we we'll plan all of the
        // FFTs we might need in advance
        if self.plan_for_order.is_none() {
            let mut planner = RealFftPlanner::new();
            let plan_for_order: Vec<Plan> = (MIN_WINDOW_ORDER..=MAX_WINDOW_ORDER)
                .map(|order| Plan {
                    r2c_plan: planner.plan_fft_forward(1 << order),
                    c2r_plan: planner.plan_fft_inverse(1 << order),
                })
                .collect();
            self.plan_for_order = Some(
                plan_for_order
                    .try_into()
                    .unwrap_or_else(|_| panic!("Mismatched plan orders")),
            );
        }

        // Normally we'd also initialize the STFT helper for the correct channel count here, but we
        // only do stereo so that's not necessary
        let window_size = self.window_size();
        if self.window_function.len() != window_size {
            self.resize_for_window(window_size);

            context.set_latency_samples(self.stft.latency_samples());
        }

        true
    }

    fn reset(&mut self) {
        // This zeroes out the buffers
        self.stft.set_block_size(self.window_size());
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Compensate for the window function, the overlap, and the extra gain introduced by the
        // IDFT operation
        let window_size = self.window_size();
        let overlap_times = self.overlap_times();
        let sample_rate = context.transport().sample_rate;
        // The overlap gain compensation is based on a squared Hann window, which will sum perfectly
        // at four times overlap or higher. We'll apply a regular Hann window before the analysis
        // and after the synthesis.
        let gain_compensation: f32 =
            ((overlap_times as f32 / 4.0) * 1.5).recip() / window_size as f32;

        // If the window size has changed since the last process call, reset the buffers and chance
        // our latency. All of these buffers already have enough capacity
        if self.window_function.len() != window_size {
            self.resize_for_window(window_size);

            context.set_latency_samples(self.stft.latency_samples());
        }

        // These plans have already been made during initialization we can switch between versions
        // without reallocating
        let fft_plan = &mut self.plan_for_order.as_mut().unwrap()
            [self.params.window_size_order.value() as usize - MIN_WINDOW_ORDER];

        let mut smoothed_pitch_value = 0.0;
        self.stft
            .process_overlap_add(buffer, overlap_times, |channel_idx, real_fft_buffer| {
                // This loop runs whenever there's a block ready, so we can't easily do any post- or
                // pre-processing without muddying up the interface. But if this is channel 0, then
                // we're dealing with a new block. We'll use this for our parameter smoothing.
                if channel_idx == 0 {
                    smoothed_pitch_value = self
                        .params
                        .pitch_octaves
                        .smoothed
                        .next_step((window_size / overlap_times) as u32);
                }
                // Negated because pitching down should cause us to take values from higher frequency bins
                let frequency_multiplier = 2.0f32.powf(-smoothed_pitch_value);

                // We'll window the input with a Hann function to avoid spectral leakage
                util::window::multiply_with_window(real_fft_buffer, &self.window_function);

                // RustFFT doesn't actually need a scratch buffer here, so we'll pass an empty
                // buffer instead
                fft_plan
                    .r2c_plan
                    .process_with_scratch(real_fft_buffer, &mut self.complex_fft_buffer, &mut [])
                    .unwrap();

                // TODO: Move this to helper functions. These functions capture a lot of variables
                //       here so that might require some work. And branch preductors are probably
                //       good enough to be able to put the match inside of the `process_bin`
                //       function, but it seems preferable to have it outside of the loop.
                let num_bins = self.complex_fft_buffer.len();
                match self.params.mode.value() {
                    PitchShiftingMode::InterpolateRectangular => {
                        // This simply interpolates the sine and cosine waves composing the complex
                        // sinusoids from the frequency bins to neighbouring frequency bins scaled
                        // by the octave pitch multiplies. The iteration order dependson the pitch
                        // shifting direction since we're doing it in place.
                        let mut process_bin = |bin_idx| {
                            let frequency = bin_idx as f32 / window_size as f32 * sample_rate;
                            let target_frequency = frequency * frequency_multiplier;

                            // Simple linear interpolation
                            let target_bin = target_frequency / sample_rate * window_size as f32;
                            let target_bin_floor = target_bin.floor() as usize;
                            let target_bin_ceil = target_bin.ceil() as usize;
                            let target_floor_t = target_bin % 1.0;
                            let target_ceil_t = 1.0 - target_floor_t;
                            let target_floor = self
                                .complex_fft_buffer
                                .get(target_bin_floor)
                                .copied()
                                .unwrap_or_default();
                            let target_ceil = self
                                .complex_fft_buffer
                                .get(target_bin_ceil)
                                .copied()
                                .unwrap_or_default();

                            self.complex_fft_buffer[bin_idx] = (target_floor * target_floor_t
                                + target_ceil * target_ceil_t)
                                * 3.0 // Random extra gain, not sure
                                * gain_compensation;
                        };

                        if frequency_multiplier >= 1.0 {
                            for bin_idx in 0..num_bins {
                                process_bin(bin_idx);
                            }
                        } else {
                            for bin_idx in (0..num_bins).rev() {
                                process_bin(bin_idx);
                            }
                        }
                    }
                    PitchShiftingMode::InterpolatePolar => {
                        // Same as the above, but interpolating in the polar form instead. While
                        // this does sound more correct it doesn't sound nearly as hilarious, and it
                        // just sounds bad at this point. But maybe there's some use for this.
                        let mut process_bin = |bin_idx| {
                            let frequency = bin_idx as f32 / window_size as f32 * sample_rate;
                            let target_frequency = frequency * frequency_multiplier;

                            // Simple linear interpolation
                            let target_bin = target_frequency / sample_rate * window_size as f32;
                            let target_bin_floor = target_bin.floor() as usize;
                            let target_bin_ceil = target_bin.ceil() as usize;
                            let target_floor_t = target_bin % 1.0;
                            let target_ceil_t = 1.0 - target_floor_t;
                            let target_floor = self
                                .complex_fft_buffer
                                .get(target_bin_floor)
                                .copied()
                                .unwrap_or_default();
                            let target_ceil = self
                                .complex_fft_buffer
                                .get(target_bin_ceil)
                                .copied()
                                .unwrap_or_default();

                            let target_floor_magnitude = target_floor.norm();
                            let target_floor_phase = target_floor.arg();
                            let target_ceil_magnitude = target_ceil.norm();
                            let target_ceil_phase = target_ceil.arg();

                            self.complex_fft_buffer[bin_idx] = Complex32::from_polar(
                                (target_floor_magnitude * target_floor_t)
                                    + (target_ceil_magnitude * target_ceil_t),
                                (target_floor_phase * target_floor_t)
                                    + (target_ceil_phase * target_ceil_t),
                            ) * 3.0 // Random extra gain, not sure
                                * gain_compensation;
                        };

                        if frequency_multiplier >= 1.0 {
                            for bin_idx in 0..num_bins {
                                process_bin(bin_idx);
                            }
                        } else {
                            for bin_idx in (0..num_bins).rev() {
                                process_bin(bin_idx);
                            }
                        }
                    }
                }

                // Make sure the imaginary components on the first and last bin are zero
                self.complex_fft_buffer[0].im = 0.0;
                self.complex_fft_buffer[num_bins - 1].im = 0.0;

                // Inverse FFT back into the scratch buffer. This will be added to a ring buffer
                // which gets written back to the host at a one block delay.
                fft_plan
                    .c2r_plan
                    .process_with_scratch(&mut self.complex_fft_buffer, real_fft_buffer, &mut [])
                    .unwrap();

                // Apply the window function once more to reduce time domain aliasing. The gain
                // compensation compensates for the squared Hann window that would be applied if we
                // didn't do any processing at all.
                util::window::multiply_with_window(real_fft_buffer, &self.window_function);
            });

        ProcessStatus::Normal
    }
}

impl PubertySimulator {
    fn window_size(&self) -> usize {
        1 << self.params.window_size_order.value() as usize
    }

    fn overlap_times(&self) -> usize {
        1 << self.params.overlap_times_order.value() as usize
    }

    /// `window_size` should not exceed `MAX_WINDOW_SIZE` or this will allocate.
    fn resize_for_window(&mut self, window_size: usize) {
        // The FFT algorithms for this window size have already been planned
        self.stft.set_block_size(window_size);
        self.window_function.resize(window_size, 0.0);
        self.complex_fft_buffer
            .resize(window_size / 2 + 1, Complex32::default());
        util::window::hann_in_place(&mut self.window_function);
    }
}

impl ClapPlugin for PubertySimulator {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.puberty-simulator";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Simulates a pitched down cracking voice");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Glitch,
        ClapFeature::PitchShifter,
    ];
}

impl Vst3Plugin for PubertySimulator {
    const VST3_CLASS_ID: [u8; 16] = *b"PubertySim..RvdH";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::PitchShift,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(PubertySimulator);
nih_export_vst3!(PubertySimulator);
