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

const WINDOW_SIZE: usize = 1024;
const OVERLAP_TIMES: usize = 4;

struct PubertySimulator {
    params: Pin<Box<PubertySimulatorParams>>,

    /// An adapter that performs most of the overlap-add algorithm for us.
    stft: util::StftHelper,
    /// A Hann window window, passed to the overlap-add helper.
    window_function: Vec<f32>,

    /// The algorithms for the FFT and IFFT operations.
    plan: Plan,
    /// Scratch buffers for computing our FFT. The [`StftHelper`] already contains a buffer for the
    /// real values.
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
    #[id = "pitch"]
    pitch_octaves: FloatParam,
}

impl Default for PubertySimulator {
    fn default() -> Self {
        Self {
            params: Box::pin(PubertySimulatorParams::default()),

            stft: util::StftHelper::new(2, WINDOW_SIZE),
            window_function: util::window::hann(WINDOW_SIZE),

            plan: Plan {
                r2c_plan: R2CPlan32::aligned(&[WINDOW_SIZE], Flag::MEASURE).unwrap(),
                c2r_plan: C2RPlan32::aligned(&[WINDOW_SIZE], Flag::MEASURE).unwrap(),
            },
            complex_fft_scratch_buffer: AlignedVec::new(WINDOW_SIZE / 2 + 1),
        }
    }
}

impl Default for PubertySimulatorParams {
    fn default() -> Self {
        Self {
            pitch_octaves: FloatParam::new(
                "Pitch",
                -1.0,
                FloatRange::SymmetricalSkewed {
                    min: -5.0,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-1.0),
                    center: 0.0,
                },
            )
            // This doesn't need smoothing to prevent zippers because we're already going
            // overlap-add, but sounds kind of slick
            .with_smoother(SmoothingStyle::Linear(100.0))
            .with_unit(" Octaves")
            .with_value_to_string(formatters::f32_rounded(2)),
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
        // Normally we'd also initialize the STFT helper for the correct channel count here, but we
        // only do stereo so that's not necessary
        self.stft.set_block_size(WINDOW_SIZE);
        context.set_latency_samples(self.stft.latency_samples());

        true
    }

    fn process(&mut self, buffer: &mut Buffer, context: &mut impl ProcessContext) -> ProcessStatus {
        // Compensate for the window function, the overlap, and the extra gain introduced by the
        // IDFT operation
        const GAIN_COMPENSATION: f32 = 1.0 / OVERLAP_TIMES as f32 / WINDOW_SIZE as f32;

        let sample_rate = context.transport().sample_rate;

        let mut smoothed_pitch_value = 0.0;
        self.stft.process_overlap_add(
            buffer,
            &self.window_function,
            OVERLAP_TIMES,
            |channel_idx, real_fft_scratch_buffer| {
                // This loop runs whenever there's a block ready, so we can't easily do any post- or
                // pre-processing without muddying up the interface. But if this is channel 0, then
                // we're dealing with a new block. We'll use this for our parameter smoothing.
                if channel_idx == 0 {
                    smoothed_pitch_value = self
                        .params
                        .pitch_octaves
                        .smoothed
                        .next_step((WINDOW_SIZE / OVERLAP_TIMES) as u32);
                }
                // Negated because pitching down should cause us to take values from higher frequency bins
                let frequency_multiplier = 2.0f32.powf(-smoothed_pitch_value);

                // Forward FFT, the helper has already applied window function
                self.plan
                    .r2c_plan
                    .r2c(
                        real_fft_scratch_buffer,
                        &mut self.complex_fft_scratch_buffer,
                    )
                    .unwrap();

                // This simply interpolates between the complex sinusoids from the frequency bins
                // for this bin's frequency scaled by the octave pitch multiplies. The iteration
                // order dependson the pitch shifting direction since we're doing it in place.
                let num_bins = self.complex_fft_scratch_buffer.len();
                let mut process_bin = |bin_idx| {
                    let frequency = bin_idx as f32 / WINDOW_SIZE as f32 * sample_rate;
                    let target_frequency = frequency * frequency_multiplier;

                    // Simple linear interpolation
                    let target_bin = target_frequency / sample_rate * WINDOW_SIZE as f32;
                    let target_bin_low = target_bin.floor() as usize;
                    let target_bin_high = target_bin.ceil() as usize;
                    let target_low_t = target_bin % 1.0;
                    let target_high_t = 1.0 - target_low_t;
                    let target_low = self
                        .complex_fft_scratch_buffer
                        .get(target_bin_low)
                        .copied()
                        .unwrap_or_default();
                    let target_high = self
                        .complex_fft_scratch_buffer
                        .get(target_bin_high)
                        .copied()
                        .unwrap_or_default();

                    self.complex_fft_scratch_buffer[bin_idx] = (target_low * target_low_t
                        + target_high * target_high_t)
                        * 6.0 // Random extra gain, not sure
                        * GAIN_COMPENSATION;
                };

                if frequency_multiplier >= 1.0 {
                    for bin_idx in 0..num_bins {
                        process_bin(bin_idx);
                    }
                } else {
                    for bin_idx in (0..num_bins).rev() {
                        process_bin(bin_idx);
                    }
                };

                // Inverse FFT back into the scratch buffer. This will be added to a ring buffer
                // which gets written back to the host at a one block delay.
                self.plan
                    .c2r_plan
                    .c2r(
                        &mut self.complex_fft_scratch_buffer,
                        real_fft_scratch_buffer,
                    )
                    .unwrap();
            },
        );

        ProcessStatus::Normal
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
