// Puberty Simulator: the next generation in voice change simulation technology
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

use fftw::array::AlignedVec;
use fftw::plan::{C2RPlan, C2RPlan32, R2CPlan, R2CPlan32};
use fftw::types::{c32, Flag};
use nih_plug::prelude::*;
use std::f32;
use std::pin::Pin;
use std::sync::Arc;

const MIN_WINDOW_ORDER: usize = 6;
#[allow(dead_code)]
const MIN_WINDOW_SIZE: usize = 1 << MIN_WINDOW_ORDER; // 64
const DEFAULT_WINDOW_ORDER: usize = 10;
#[allow(dead_code)]
const DEFAULT_WINDOW_SIZE: usize = 1 << DEFAULT_WINDOW_ORDER; // 1024
const MAX_WINDOW_ORDER: usize = 15;
const MAX_WINDOW_SIZE: usize = 1 << MAX_WINDOW_ORDER; // 32768

const MIN_OVERLAP_ORDER: usize = 1;
#[allow(dead_code)]
const MIN_OVERLAP_TIMES: usize = 1 << MIN_OVERLAP_ORDER; // 2
const DEFAULT_OVERLAP_ORDER: usize = 3;
#[allow(dead_code)]
const DEFAULT_OVERLAP_TIMES: usize = 1 << DEFAULT_OVERLAP_ORDER; // 4
const MAX_OVERLAP_ORDER: usize = 5;
#[allow(dead_code)]
const MAX_OVERLAP_TIMES: usize = 1 << MAX_OVERLAP_ORDER; // 32

struct PubertySimulator {
    params: Pin<Box<PubertySimulatorParams>>,

    /// An adapter that performs most of the overlap-add algorithm for us.
    stft: util::StftHelper,
    /// Contains a Hann window function of the current window length, passed to the overlap-add
    /// helper. Allocated with a `MAX_WINDOW_SIZE` initial capacity.
    window_function: Vec<f32>,

    /// The algorithms for the FFT and IFFT operations, for each supported order so we can switch
    /// between them without replanning or allocations. Initialized during `initialize()`.
    plan_for_order: Option<[Plan; MAX_WINDOW_ORDER - MIN_WINDOW_ORDER + 1]>,
    /// Scratch buffers for computing our FFT. The [`StftHelper`] already contains a buffer for the
    /// real values. This type cannot be resized, so we'll simply take a slice of it with the
    /// correct length instead.
    complex_fft_scratch_buffer: AlignedVec<c32>,
}

/// FFTW uses raw pointers which aren't Send+Sync, so we'll wrap this in a separate struct.
struct Plan {
    r2c_plan: R2CPlan32,
    c2r_plan: C2RPlan32,
}

unsafe impl Send for Plan {}
unsafe impl Sync for Plan {}

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
}

impl Default for PubertySimulator {
    fn default() -> Self {
        Self {
            params: Box::pin(PubertySimulatorParams::default()),

            stft: util::StftHelper::new(2, MAX_WINDOW_SIZE),
            window_function: Vec::with_capacity(MAX_WINDOW_SIZE),

            plan_for_order: None,
            complex_fft_scratch_buffer: AlignedVec::new(MAX_WINDOW_SIZE / 2 + 1),
        }
    }
}

impl Default for PubertySimulatorParams {
    fn default() -> Self {
        let power_of_two_val2str = Arc::new(|value| format!("{}", 1 << value));
        let power_of_two_str2val =
            Arc::new(|string: &str| string.parse().ok().map(|n: i32| (n as f32).log2() as i32));

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
            .with_value_to_string(formatters::f32_rounded(2)),

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
        }
    }
}

impl Plugin for PubertySimulator {
    const NAME: &'static str = "Puberty Simulator";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.1.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const ACCEPTS_MIDI: bool = false;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // We'll only do stereo for simplicity's sake
        config.num_input_channels == config.num_output_channels && config.num_input_channels == 2
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        _buffer_config: &BufferConfig,
        context: &mut impl ProcessContext,
    ) -> bool {
        if self.plan_for_order.is_none() {
            let plan_for_order: Vec<Plan> = (MIN_WINDOW_ORDER..=MAX_WINDOW_ORDER)
                // `Flag::MEASURE` is pretty slow above 1024 which hurts initialization time.
                // `Flag::ESTIMATE` does not seem to hurt performance much at reasonable orders, so
                // that's good enough for now. An alternative would be to replan on a worker thread,
                // but this makes switching between window sizes a bit cleaner.
                .map(|order| Plan {
                    r2c_plan: R2CPlan32::aligned(
                        &[1 << order],
                        Flag::ESTIMATE | Flag::DESTROYINPUT,
                    )
                    .unwrap(),
                    c2r_plan: C2RPlan32::aligned(
                        &[1 << order],
                        Flag::ESTIMATE | Flag::DESTROYINPUT,
                    )
                    .unwrap(),
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

    fn process(&mut self, buffer: &mut Buffer, context: &mut impl ProcessContext) -> ProcessStatus {
        // Compensate for the window function, the overlap, and the extra gain introduced by the
        // IDFT operation
        let window_size = self.window_size();
        let overlap_times = self.overlap_times();
        let sample_rate = context.transport().sample_rate;
        let gain_compensation: f32 = 1.0 / (overlap_times as f32).log2() / window_size as f32;

        // If the window size has changed since the last process call, reset the buffers and chance
        // our latency. All of these buffers already have enough capacity
        if self.window_function.len() != window_size {
            self.resize_for_window(window_size);

            context.set_latency_samples(self.stft.latency_samples());
        }

        // Since this type cannot be resized, we'll simply slice the full buffer instead
        let complex_fft_scratch_buffer =
            &mut self.complex_fft_scratch_buffer.as_slice_mut()[..window_size / 2 + 1];
        // These plans have already been made during initialization we can switch between versions
        // without reallocating
        let fft_plan = &mut self.plan_for_order.as_mut().unwrap()
            [self.params.window_size_order.value as usize - MIN_WINDOW_ORDER];

        let mut smoothed_pitch_value = 0.0;
        self.stft.process_overlap_add(
            buffer,
            &self.window_function,
            overlap_times,
            |channel_idx, real_fft_scratch_buffer| {
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

                // Forward FFT, the helper has already applied window function
                fft_plan
                    .r2c_plan
                    .r2c(real_fft_scratch_buffer, complex_fft_scratch_buffer)
                    .unwrap();

                // This simply interpolates between the complex sinusoids from the frequency bins
                // for this bin's frequency scaled by the octave pitch multiplies. The iteration
                // order dependson the pitch shifting direction since we're doing it in place.
                let num_bins = complex_fft_scratch_buffer.len();
                let mut process_bin = |bin_idx| {
                    let frequency = bin_idx as f32 / window_size as f32 * sample_rate;
                    let target_frequency = frequency * frequency_multiplier;

                    // Simple linear interpolation
                    let target_bin = target_frequency / sample_rate * window_size as f32;
                    let target_bin_low = target_bin.floor() as usize;
                    let target_bin_high = target_bin.ceil() as usize;
                    let target_low_t = target_bin % 1.0;
                    let target_high_t = 1.0 - target_low_t;
                    let target_low = complex_fft_scratch_buffer
                        .get(target_bin_low)
                        .copied()
                        .unwrap_or_default();
                    let target_high = complex_fft_scratch_buffer
                        .get(target_bin_high)
                        .copied()
                        .unwrap_or_default();

                    complex_fft_scratch_buffer[bin_idx] = (target_low * target_low_t
                        + target_high * target_high_t)
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

                // Inverse FFT back into the scratch buffer. This will be added to a ring buffer
                // which gets written back to the host at a one block delay.
                fft_plan
                    .c2r_plan
                    .c2r(complex_fft_scratch_buffer, real_fft_scratch_buffer)
                    .unwrap();
            },
        );

        ProcessStatus::Normal
    }
}

impl PubertySimulator {
    fn window_size(&self) -> usize {
        1 << self.params.window_size_order.value as usize
    }

    fn overlap_times(&self) -> usize {
        1 << self.params.overlap_times_order.value as usize
    }

    /// `window_size` should not exceed `MAX_WINDOW_SIZE` or this will allocate.
    fn resize_for_window(&mut self, window_size: usize) {
        // The FFT algorithms for this window size have already been planned
        self.stft.set_block_size(window_size);
        self.window_function.resize(window_size, 0.0);
        util::window::hann_in_place(&mut self.window_function);
    }
}

impl ClapPlugin for PubertySimulator {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.puberty-simulator";
    const CLAP_DESCRIPTION: &'static str = "Simulates a pitched down cracking voice";
    const CLAP_FEATURES: &'static [&'static str] =
        &["audio_effect", "stereo", "glitch", "pitch_shifter"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for PubertySimulator {
    const VST3_CLASS_ID: [u8; 16] = *b"PubertySim..RvdH";
    const VST3_CATEGORIES: &'static str = "Fx|Pitch Shift";
}

nih_export_clap!(PubertySimulator);
nih_export_vst3!(PubertySimulator);
