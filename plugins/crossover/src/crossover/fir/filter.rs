// Crossover: clean crossovers as a multi-out plugin
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

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealToComplex};
use std::f32;

use crate::crossover::iir::biquad::{Biquad, BiquadCoefficients};
use crate::NUM_CHANNELS;

/// We're doing FFT convolution here since otherwise there's no way to get decent low-frequency
/// accuracy while still having acceptable performance. The input going into the STFT will be
/// smaller since it will be padding with zeroes to compensate for the otherwise overlapping tail
/// caused by the convolution.
pub const FFT_SIZE: usize = 4096;
/// The input chunk size the FFT convolution is processing. This is also part of the latency, with
/// the total latency being `FFT_INPUT_SIZE + (FILTER_SIZE / 2)` samples. By having this be exactly
/// half of FFT_SIZE, we can make the overlap-add part of the FFT convolution a lot simpler for
/// ourselves. (check the `StftHelper` struct in NIH-plug itself for an examples that can handle
/// arbitrary padding)
pub const FFT_INPUT_SIZE: usize = FFT_SIZE / 2;
/// The size of the FIR filter window, or the number of taps. Convoling `FFT_INPUT_SIZE` samples
/// with this filter should fit exactly in `FFT_SIZE`, and it should be an odd number.
pub const FILTER_SIZE: usize = FFT_SIZE - FFT_INPUT_SIZE + 1;

/// A single FIR filter that may be configured in any way. In this plugin this will be a
/// linear-phase low-pass, band-pass, or high-pass filter. Implemented using FFT convolution. `git
/// blame` this for a version that uses direct convolution.
///
/// `N_INPUT` is the size of the input that will be processed. The size of the FFT window becomes
/// `N_INPUT * 2`. That makes handling the overlap easy, as each IDFT after multiplying the padded
/// input and the padded impulse response FFTs will result one `N_INPUT` period of output that can
/// be taken as is, followed by one `N_INPUT` period of samples that need to be added to the next
/// period's outputs as part of the overlap-add process.
#[derive(Debug, Clone)]
pub struct FftFirFilter {
    /// An `N_INPUT + 1` sized IIR. Padded, ran through the DFT, and then normalized by dividing by
    /// `FFT_SIZE`.
    padded_ir_fft: [Complex32; FFT_SIZE / 2 + 1],

    /// The padding from the previous IDFT operation that needs to be added to the next output
    /// buffer. After the IDFT process there will be an `FFT_SIZE` real scratch buffer containing
    /// the output. At that point the first `FFT_INPUT_SIZE` samples of those will be copied to
    /// `output_buffers` in the FIR crossover, `unapplied_padding_buffer` will be added to that
    /// output buffer, and then finally the last `FFT_INPUT_SIZE` samples of the scratch buffer are
    /// copied to `unapplied_padding_buffer`. This thus makes sure the tail gets delayed by another
    /// period so that everything matches up.
    unapplied_padding_buffers: [[f32; FFT_INPUT_SIZE]; NUM_CHANNELS as usize],
}

/// Coefficients for a (linear-phase) FIR filter. This struct includes ways to design the filter.
/// `T` is the sample type and `N` is the number of taps/coefficients and should be odd for linear-phase filters.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct FirCoefficients<const N: usize>(pub [f32; N]);

impl Default for FftFirFilter {
    fn default() -> Self {
        Self {
            // Would be nicer to initialize this to an impulse response that actually had the
            // correct position wrt the usual linear-phase latency, but this is fine since it should
            // never be used anyways
            padded_ir_fft: [Complex32::new(1.0 / FFT_SIZE as f32, 0.0); FFT_SIZE / 2 + 1],
            unapplied_padding_buffers: [[0.0; FFT_INPUT_SIZE]; NUM_CHANNELS as usize],
        }
    }
}

impl<const N: usize> Default for FirCoefficients<N> {
    fn default() -> Self {
        // Initialize this to a delay with the same amount of latency as we'd introduce with our
        // linear-phase filters
        let mut coefficients = [0.0; N];
        coefficients[N / 2] = 1.0;

        Self(coefficients)
    }
}

impl FftFirFilter {
    /// Filter `FFT_INPUT_SIZE` samples padded to `FFT_SIZE` through this filter, and write the
    /// outputs to `output_samples` (belonging to channel `channel_idx`), at an `FFT_INPUT_SIZE`
    /// delay. This is a bit weird and probably difficult to follow because as an optimization the
    /// DFT is taken only once, and then the IDFT is taken once for every filtered band. This
    /// function is thus called inside of the overlap-add loop to avoid duplicate work.
    pub fn process(
        &mut self,
        input_fft: &[Complex32; FFT_SIZE / 2 + 1],
        output_samples: &mut [f32; FFT_INPUT_SIZE],
        output_channel_idx: usize,
        c2r_plan: &dyn ComplexToReal<f32>,
        real_scratch_buffer: &mut [f32; FFT_SIZE],
        complex_scratch_buffer: &mut [Complex32; FFT_SIZE / 2 + 1],
    ) {
        // The padded input FFT has already been taken, so we only need to copy it to the scratch
        // buffer (the input cannot change as the next band might need it as well).
        complex_scratch_buffer.copy_from_slice(input_fft);

        // The FFT of the impulse response has already been normalized, so we just need to
        // multiply the two buffers
        for (output_bin, ir_bin) in complex_scratch_buffer
            .iter_mut()
            .zip(self.padded_ir_fft.iter())
        {
            *output_bin *= ir_bin;
        }
        c2r_plan
            .process_with_scratch(complex_scratch_buffer, real_scratch_buffer, &mut [])
            .unwrap();

        // At this point the first `FFT_INPUT_SIZE` elements in `real_scratch_buffer`
        // contain the output for the next period, while the last `FFT_INPUT_SIZE` elements
        // contain output that needs to be added to the period after that. Since previous
        // period also produced similar delayed output, we'll need to copy that to the
        // results as well.
        output_samples.copy_from_slice(&real_scratch_buffer[..FFT_INPUT_SIZE]);
        for (output_sample, padding_sample) in output_samples
            .iter_mut()
            .zip(self.unapplied_padding_buffers[output_channel_idx].iter())
        {
            *output_sample += *padding_sample;
        }
        self.unapplied_padding_buffers[output_channel_idx]
            .copy_from_slice(&real_scratch_buffer[FFT_INPUT_SIZE..]);
    }

    /// Set the filter's coefficients based on raw FIR filter coefficients. These will be padded,
    /// ran through the DFT, and normalized.
    pub fn recompute_coefficients(
        &mut self,
        coefficients: FirCoefficients<FILTER_SIZE>,
        r2c_plan: &dyn RealToComplex<f32>,
        real_scratch_buffer: &mut [f32; FFT_SIZE],
        complex_scratch_buffer: &mut [Complex32; FFT_SIZE / 2 + 1],
    ) {
        // This needs to be padded with zeroes
        real_scratch_buffer[..FILTER_SIZE].copy_from_slice(&coefficients.0);
        real_scratch_buffer[FILTER_SIZE..].fill(0.0);

        r2c_plan
            .process_with_scratch(real_scratch_buffer, complex_scratch_buffer, &mut [])
            .unwrap();

        // The resulting buffer needs to be normalized and written to `self.padded_ir_fft`. That way
        // we don't need to do anything but multiplying and writing the results back when
        // processing.
        let normalization_factor = 1.0 / FFT_SIZE as f32;
        for (filter_bin, target_bin) in complex_scratch_buffer
            .iter()
            .zip(self.padded_ir_fft.iter_mut())
        {
            *target_bin = *filter_bin * normalization_factor;
        }
    }

    /// Reset the internal filter state.
    pub fn reset(&mut self) {
        for buffer in &mut self.unapplied_padding_buffers {
            buffer.fill(0.0);
        }
    }
}

impl<const N: usize> FirCoefficients<N> {
    /// A somewhat crude but very functional and relatively fast way create linear phase FIR
    /// **low-pass** filter that matches the frequency response of a fourth order biquad low-pass
    /// filter. As in, this matches the frequency response magnitudes of applying those biquads to a
    /// signal twice. This only works for low-pass filters, as the function normalizes the result to
    /// hae unity gain at the DC bin. The algorithm works as follows:
    ///
    /// - An impulse function (so all zeroes except for the first element) of length `FILTER_LEN / 2
    ///   + 1` is filtered with the biquad.
    /// - The biquad's state is reset, and the impulse response is filtered in the opposite
    ///   direction.
    /// - At this point the bidirectionally filtered impulse response contains the **right** half of
    ///   a truncated linear phase FIR kernel.
    ///
    /// Since the FIR filter will be a symmetrical version of this impulse response, we can optimize
    /// the post-processing work slightly by windowing and normalizing this bidirectionally filtered
    /// impulse response instead.
    ///
    /// - A half Blackman window is applied to the impulse response. Since this is the right half,
    ///   this starts at unity gain for the first sample and then tapers off towards the right.
    /// - The impulse response is then normalized such that the final linear-phase FIR kernel has a
    ///   sum of 1.0. Since it will be symmetrical around the IRs first sample, the would-be final
    ///   sum can be computed as `ir.sum() * 2 - ir[0]`.
    ///
    /// Lastly the linear phase FIR filter simply needs to be constructed from this right half:
    ///
    /// - This bidirectionally filtered impulse response is then reversed, and placed at the start
    ///   of the `FILTER_LEN` size FIR coefficient array.
    /// - The non-reversed bidirectionally filtered impulse response is copied to the second half of
    ///   the coefficients. (one of the copies doesn't need to include the centermost coefficient)
    ///
    /// The corresponding high-pass filter can be computed through spectral inversion.
    pub fn design_fourth_order_linear_phase_low_pass_from_biquad(
        biquad_coefs: BiquadCoefficients<f32>,
    ) -> Self {
        // Rust doesn't allow you to define this as a constant
        let center_idx = N / 2;

        // We'll start with an impulse (at exactly half of this odd sized buffer)...
        let mut impulse_response = [0.0; N];
        impulse_response[center_idx] = 1.0;

        // ...and filter that in both directions
        let mut biquad = Biquad::default();
        biquad.coefficients = biquad_coefs;
        for sample in impulse_response.iter_mut().skip(center_idx - 1) {
            *sample = biquad.process(*sample);
        }

        biquad.reset();
        for sample in impulse_response.iter_mut().skip(center_idx - 1).rev() {
            *sample = biquad.process(*sample);
        }

        // Now the right half of `impulse_response` contains a truncated right half of the
        // linear-phase FIR filter. We can apply the window function here, and then fianlly
        // normalize it so that the the final FIR filter kernel sums to 1.

        // Adopted from `nih_plug::util::window`. We only end up applying the right half of the
        // window, starting at the top of the window.
        let blackman_scale_1 = (2.0 * f32::consts::PI) / (N - 1) as f32;
        let blackman_scale_2 = blackman_scale_1 * 2.0;
        for (sample_idx, sample) in impulse_response.iter_mut().enumerate().skip(center_idx - 1) {
            let cos_1 = (blackman_scale_1 * sample_idx as f32).cos();
            let cos_2 = (blackman_scale_2 * sample_idx as f32).cos();
            *sample *= 0.42 - (0.5 * cos_1) + (0.08 * cos_2);
        }

        // Since this final filter will be symmetrical around `impulse_response[CENTER_IDX]`, we
        // can simply normalize based on that fact:
        let would_be_impulse_response_sum = (impulse_response.iter().skip(center_idx).sum::<f32>()
            * 2.0)
            - impulse_response[center_idx];
        let would_be_impulse_response_recip = would_be_impulse_response_sum.recip();
        for sample in &mut impulse_response {
            *sample *= would_be_impulse_response_recip;
        }

        // And finally we can simply copy the right half of the filter kernel to the left half
        // around the `CENTER_IDX`.
        for source_idx in center_idx + 1..N {
            let target_idx = center_idx - (source_idx - center_idx);
            impulse_response[target_idx] = impulse_response[source_idx];
        }

        Self(impulse_response)
    }
}
