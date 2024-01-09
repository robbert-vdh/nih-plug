// Diopser: a phase rotation plugin
// Copyright (C) 2021-2024 Robbert van der Helm
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
use nih_plug::util::window::multiply_with_window;
use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};
use std::f32;
use std::sync::Arc;
use triple_buffer::TripleBuffer;

pub const SPECTRUM_WINDOW_SIZE: usize = 2048;
// Don't need that much precision here
const SPECTRUM_WINDOW_OVERLAP: usize = 2;

/// The time it takes for the spectrum to go down 12 dB. The upwards step is immediate like in a
/// peak meter.
const SMOOTHING_DECAY_MS: f32 = 100.0;

/// The amplitudes of all frequency bins in a windowed FFT of Diopser's output. Also includes the DC
/// offset bin which we don't draw, just to make this a bit less confusing.
pub type Spectrum = [f32; SPECTRUM_WINDOW_SIZE / 2 + 1];
/// A receiver for a spectrum computed by [`SpectrumInput`].
pub type SpectrumOutput = triple_buffer::Output<Spectrum>;

/// Continuously compute spectrums and send them to the connected [`SpectrumOutput`].
pub struct SpectrumInput {
    /// A helper to do most of the STFT process.
    stft: util::StftHelper,
    /// The number of channels we're working on.
    num_channels: usize,

    /// The spectrum behaves like a peak meter. If the new value is higher than the previous one, it
    /// jump up immediately. Otherwise the old value is multiplied by this weight and the new value
    /// by one minus this weight.
    smoothing_decay_weight: f32,

    /// A way to send data to the corresponding [`SpectrumOutput`]. `spectrum_result_buffer` gets
    /// copied into this buffer every time a new spectrum is available.
    triple_buffer_input: triple_buffer::Input<Spectrum>,
    /// A scratch buffer to compute the resulting power amplitude spectrum.
    spectrum_result_buffer: Spectrum,

    /// The algorithm for the FFT operation used for our spectrum analyzer.
    plan: Arc<dyn RealToComplex<f32>>,
    /// A Hann window window, passed to the STFT helper. The gain compensation is already part of
    /// this window to save a multiplication step.
    compensated_window_function: Vec<f32>,
    /// The output of our real->complex FFT.
    complex_fft_buffer: Vec<Complex32>,
}

impl SpectrumInput {
    /// Create a new spectrum input and output pair. The output should be moved to the editor.
    pub fn new(num_channels: usize) -> (SpectrumInput, SpectrumOutput) {
        let (triple_buffer_input, triple_buffer_output) =
            TripleBuffer::new(&[0.0; SPECTRUM_WINDOW_SIZE / 2 + 1]).split();

        let input = Self {
            stft: util::StftHelper::new(num_channels, SPECTRUM_WINDOW_SIZE, 0),
            num_channels,

            // This is set in `initialize()` based on the sample rate
            smoothing_decay_weight: 0.0,

            triple_buffer_input,
            spectrum_result_buffer: [0.0; SPECTRUM_WINDOW_SIZE / 2 + 1],

            plan: RealFftPlanner::new().plan_fft_forward(SPECTRUM_WINDOW_SIZE),
            compensated_window_function: util::window::hann(SPECTRUM_WINDOW_SIZE)
                .into_iter()
                // Include the gain compensation in the window function to save some multiplications
                .map(|x| x / SPECTRUM_WINDOW_SIZE as f32)
                .collect(),
            complex_fft_buffer: vec![Complex32::default(); SPECTRUM_WINDOW_SIZE / 2 + 1],
        };

        (input, triple_buffer_output)
    }

    /// Update the smoothing using the specified sample rate. Called in `initialize()`.
    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        // We'll express the dacay rate in the time it takes for the moving average to drop by 12 dB
        // NOTE: The effective sample rate accounts for the STFT interval, **and** for the number of
        //       channels. We'll average both channels to mono-ish.
        let effective_sample_rate = sample_rate / SPECTRUM_WINDOW_SIZE as f32
            * SPECTRUM_WINDOW_OVERLAP as f32
            * self.num_channels as f32;
        let decay_samples = (SMOOTHING_DECAY_MS / 1000.0 * effective_sample_rate) as f64;

        self.smoothing_decay_weight = 0.25f64.powf(decay_samples.recip()) as f32
    }

    /// Compute the spectrum for a buffer and send it to the corresponding output pair.
    pub fn compute(&mut self, buffer: &Buffer) {
        self.stft.process_analyze_only(
            buffer,
            SPECTRUM_WINDOW_OVERLAP,
            |_channel_idx, real_fft_scratch_buffer| {
                multiply_with_window(real_fft_scratch_buffer, &self.compensated_window_function);

                self.plan
                    .process_with_scratch(
                        real_fft_scratch_buffer,
                        &mut self.complex_fft_buffer,
                        // We don't actually need a scratch buffer
                        &mut [],
                    )
                    .unwrap();

                // We'll use peak meter-like behavior for the spectrum analyzer to make things
                // easier to dial in. Values that are higher than the old value snap to the new
                // value immediately, lower values decay gradually. This also results in quasi-mono
                // summing since this same callback will be called for both channels. Gain
                // compensation has already been baked into the window function.
                for (bin, spectrum_result) in self
                    .complex_fft_buffer
                    .iter()
                    .zip(&mut self.spectrum_result_buffer)
                {
                    let magnitude = bin.norm();
                    if magnitude > *spectrum_result {
                        *spectrum_result = magnitude;
                    } else {
                        *spectrum_result = (*spectrum_result * self.smoothing_decay_weight)
                            + (magnitude * (1.0 - self.smoothing_decay_weight));
                    }
                }

                self.triple_buffer_input.write(self.spectrum_result_buffer);
            },
        );
    }
}
