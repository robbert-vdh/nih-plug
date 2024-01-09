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

use nih_plug::debug::*;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::f32;
use std::sync::Arc;

use self::filter::{FftFirFilter, FirCoefficients, FFT_INPUT_SIZE, FFT_SIZE};
use crate::crossover::fir::filter::FILTER_SIZE;
use crate::crossover::iir::biquad::{BiquadCoefficients, NEUTRAL_Q};
use crate::{NUM_BANDS, NUM_CHANNELS};

pub mod filter;

pub struct FirCrossover {
    /// The kind of crossover to use. `.update_filters()` must be called after changing this.
    mode: FirCrossoverType,

    /// Filters for each of the bands. Depending on the number of bands argument passed to
    /// `.process()`, two to five of these may be used. The first one always contains a low-pass
    /// filter, the last one always contains a high-pass filter, while the other bands will contain
    /// band-pass filters.
    ///
    /// These filters will be fed the FFT from the main input to produce output samples for the enxt
    /// period. Everything could be a bit nicer to read if the filter did the entire STFT process,
    /// but that would mean duplicating the input ring buffer and forward DFT up to five times.
    band_filters: Box<[FftFirFilter; NUM_BANDS]>,

    /// A ring buffer that is used to store inputs for the next FFT. Until it is time to take the
    /// next FFT, samples are copied from the inputs to this buffer, while simultaneously copying
    /// the already processed output samples from the output buffers to the output. Once
    /// `io_buffer_next_indices` wrap back around to 0, the next buffer should be produced.
    input_buffers: Box<[[f32; FFT_INPUT_SIZE]; NUM_CHANNELS as usize]>,
    /// A ring that contains the next period's outputs for each of the five bands. This is written
    /// to and read from in lockstep with `input_buffers`.
    band_output_buffers: Box<[[[f32; FFT_INPUT_SIZE]; NUM_CHANNELS as usize]; NUM_BANDS]>,
    /// The index in the inner `io_buffer` the next sample should be read from. After a sample is
    /// written to the band's output then this is incremented by one. Once
    /// `self.io_buffer_next_indices[channel_idx] == self.io_buffer.len()` then the next block
    /// should be processed.
    ///
    /// This is stored as an array since each channel is processed individually. While this should
    /// of course stay in sync, this makes it much simpler to process both channels in sequence.
    io_buffers_next_indices: [usize; NUM_CHANNELS as usize],

    /// The algorithm for the FFT operation.
    r2c_plan: Arc<dyn RealToComplex<f32>>,
    /// The algorithm for the IFFT operation.
    c2r_plan: Arc<dyn ComplexToReal<f32>>,

    /// A real buffer that may be written to in place during the FFT and IFFT operations.
    real_scratch_buffer: Box<[f32; FFT_SIZE]>,
    /// A complex buffer corresponding to `real_scratch_buffer` that may be written to in place
    /// during the FFT and IFFT operations.
    complex_scratch_buffer: Box<[Complex32; FFT_SIZE / 2 + 1]>,
}

/// The type of FIR crossover to use.
#[derive(Debug, Clone, Copy)]
pub enum FirCrossoverType {
    /// Emulates the filter slope of [`super::iir::IirCrossoverType`], but with linear-phase FIR
    /// filters instead of minimum-phase IIR filters. The exact same filters are used to design the
    /// FIR filters.
    LinkwitzRiley24LinearPhase,
}

impl FirCrossover {
    /// Create a new multiband crossover processor. All filters will be configured to pass audio
    /// through as is, albeit with a delay. `.update()` needs to be called first to set up the
    /// filters, and `.reset()` can be called whenever the filter state must be cleared.
    ///
    /// Make sure to add the latency reported by [`latency()`][Self::latency()] to the plugin's
    /// reported latency.
    pub fn new(mode: FirCrossoverType) -> Self {
        let mut fft_planner = RealFftPlanner::new();

        Self {
            mode,
            band_filters: Default::default(),

            input_buffers: Box::new([[0.0; FFT_INPUT_SIZE]; NUM_CHANNELS as usize]),
            band_output_buffers: Box::new(
                [[[0.0; FFT_INPUT_SIZE]; NUM_CHANNELS as usize]; NUM_BANDS],
            ),
            io_buffers_next_indices: [0; NUM_CHANNELS as usize],
            r2c_plan: fft_planner.plan_fft_forward(FFT_SIZE),
            c2r_plan: fft_planner.plan_fft_inverse(FFT_SIZE),
            real_scratch_buffer: Box::new([0.0; FFT_SIZE]),
            complex_scratch_buffer: Box::new([Complex32::default(); FFT_SIZE / 2 + 1]),
        }
    }

    /// Get the current latency in samples. This depends on the selected mode.
    pub fn latency(&self) -> u32 {
        // Actually, that's a lie, since we currently only do linear-phase filters with a constant
        // size
        match self.mode {
            FirCrossoverType::LinkwitzRiley24LinearPhase => {
                (FFT_INPUT_SIZE + (FILTER_SIZE / 2)) as u32
            }
        }
    }

    /// Split the signal into bands using the crossovers previously configured through `.update()`.
    /// The split bands will be written to `band_outputs`. The main output should be cleared
    /// separately. For efficiency's sake this processes an entire channel at once to minimize the
    /// number of FFT operations needed. Since this process delays the signal by `FFT_INPUT_SIZE`
    /// samples, the latency should be reported to the host.
    pub fn process(
        &mut self,
        num_bands: usize,
        main_input: &[f32],
        mut band_outputs: [&mut &mut [f32]; NUM_BANDS],
        channel_idx: usize,
    ) {
        nih_debug_assert!(main_input.len() == band_outputs[0].len());
        nih_debug_assert!(channel_idx < NUM_CHANNELS as usize);

        // We'll copy already processed output to `band_outputs` while storing input for the next
        // FFT operation. This is a modified version of what's going on in `StftHelper`.
        let mut current_sample_idx = 0;
        while current_sample_idx < main_input.len() {
            {
                // When `self.io_buffers_next_indices == FFT_SIZE`, the next block should be processed
                let io_buffers_next_indices = self.io_buffers_next_indices[channel_idx];
                let process_num_samples = (FFT_INPUT_SIZE - io_buffers_next_indices)
                    .min(main_input.len() - current_sample_idx);

                // Since we can't do this in-place (without unnecessarily duplicating a ton of data),
                // copying data from and to the ring buffers can be done with simple memcpys
                self.input_buffers[channel_idx]
                    [io_buffers_next_indices..io_buffers_next_indices + process_num_samples]
                    .copy_from_slice(
                        &main_input[current_sample_idx..current_sample_idx + process_num_samples],
                    );
                for (band_output, band_output_buffers) in band_outputs
                    .iter_mut()
                    .zip(self.band_output_buffers.iter())
                    .take(num_bands)
                {
                    band_output[current_sample_idx..current_sample_idx + process_num_samples]
                        .copy_from_slice(
                            &band_output_buffers[channel_idx][io_buffers_next_indices
                                ..io_buffers_next_indices + process_num_samples],
                        );
                }

                // This is tracked per-channel because both channels are processed individually
                self.io_buffers_next_indices[channel_idx] += process_num_samples;
                current_sample_idx += process_num_samples;
            }

            // At this point we either reached the end of the buffer (`current_sample_idx ==
            // main_input.len()`), or we filled up the `io_buffer` and we can process the next block
            if self.io_buffers_next_indices[channel_idx] == FFT_INPUT_SIZE {
                // Zero pad the input for the FFT
                self.real_scratch_buffer[..FFT_INPUT_SIZE]
                    .copy_from_slice(&self.input_buffers[channel_idx]);
                self.real_scratch_buffer[FFT_INPUT_SIZE..].fill(0.0);

                self.r2c_plan
                    .process_with_scratch(
                        &mut *self.real_scratch_buffer,
                        &mut *self.complex_scratch_buffer,
                        &mut [],
                    )
                    .unwrap();

                // The input can then be used to produce each band's output. Since realfft expects
                // to be able to modify the input, we need to make a copy of this first:
                let input_fft = *self.complex_scratch_buffer;

                for (band_output_buffers, band_filter) in self
                    .band_output_buffers
                    .iter_mut()
                    .zip(self.band_filters.iter_mut())
                    .take(num_bands)
                {
                    band_filter.process(
                        &input_fft,
                        &mut band_output_buffers[channel_idx],
                        channel_idx,
                        &*self.c2r_plan,
                        &mut self.real_scratch_buffer,
                        &mut self.complex_scratch_buffer,
                    )
                }

                self.io_buffers_next_indices[channel_idx] = 0;
            }
        }
    }

    /// Update the crossover frequencies for all filters. `num_bands` is assumed to be in `[2,
    /// NUM_BANDS]`.
    pub fn update(
        &mut self,
        sample_rate: f32,
        num_bands: usize,
        frequencies: [f32; NUM_BANDS - 1],
    ) {
        match self.mode {
            FirCrossoverType::LinkwitzRiley24LinearPhase => {
                // The goal here is to design 2-5 filters with the same frequency response
                // magnitudes as the split bands in the IIR LR24 crossover version with the same
                // center frequencies would have. The algorithm works in two stages. First, the IIR
                // low-pass filters for the 1-4 crossovers used in the equivalent IIR LR24 version
                // are computed and converted to equivalent linear-phase FIR filters using the
                // algorithm described below in `FirCoefficients`. Then these are used to build the
                // coefficients for the 2-5 bands:
                //
                // - The first band is always simply the first band's
                //   low-pass filter.
                // - The middle bands are band-pass filters. These are created by taking the next
                //   crossover's low-pass filter and subtracting the accumulated band impulse
                //   response up to that point. The accumulated band impulse response is initialized
                //   with the first band's low-pass filter, and the band-pass filter for every band
                //   after that gets added to it.
                // - The final band is a high-pass filter that's computed through spectral inversion
                //   from the accumulated band impulse response.

                // As explained above, we'll start with the low-pass band
                nih_debug_assert!(num_bands >= 2);
                let iir_coefs = BiquadCoefficients::lowpass(sample_rate, frequencies[0], NEUTRAL_Q);
                let lp_fir_coefs =
                    FirCoefficients::design_fourth_order_linear_phase_low_pass_from_biquad(
                        iir_coefs,
                    );
                self.band_filters[0].recompute_coefficients(
                    lp_fir_coefs.clone(),
                    &*self.r2c_plan,
                    &mut self.real_scratch_buffer,
                    &mut self.complex_scratch_buffer,
                );

                // For the band-pass filters and the final high-pass filter, we need to keep track
                // of the accumulated impulse response
                let mut accumulated_ir = lp_fir_coefs;
                for (split_frequency, band_filter) in frequencies
                    .iter()
                    .zip(self.band_filters.iter_mut())
                    // There are `num_bands` bands, so there are `num_bands - 1` crossovers. The
                    // last band is formed from the accumulated impulse response.
                    .take(num_bands - 1)
                    // And the first band is already taken care of
                    .skip(1)
                {
                    let iir_coefs =
                        BiquadCoefficients::lowpass(sample_rate, *split_frequency, NEUTRAL_Q);
                    let lp_fir_coefs =
                        FirCoefficients::design_fourth_order_linear_phase_low_pass_from_biquad(
                            iir_coefs,
                        );

                    // We want the band between the accumulated frequency response and the next
                    // crossover's low-pass filter
                    let mut fir_bp_coefs = lp_fir_coefs;
                    for (bp_coef, accumulated_coef) in
                        fir_bp_coefs.0.iter_mut().zip(accumulated_ir.0.iter_mut())
                    {
                        // At this poing `bp_coef` is the low-pass filter
                        *bp_coef -= *accumulated_coef;

                        // And the accumulated coefficients for the next band/for the high-pass
                        // filter should contain this band-pass filter. This becomes a bit weirder
                        // to read when it's a single loop, but essentially this is what's going on
                        // here:
                        //
                        //     fir_bp_coefs = fir_lp_coefs - accumulated_ir
                        //     accumulated_ir += fir_bp_coefs

                        *accumulated_coef += *bp_coef;
                    }

                    band_filter.recompute_coefficients(
                        fir_bp_coefs,
                        &*self.r2c_plan,
                        &mut self.real_scratch_buffer,
                        &mut self.complex_scratch_buffer,
                    );
                }

                // And finally we can do a spectral inversion of the accumulated IR to the the last
                // band's high-pass filter
                let mut fir_hp_coefs = accumulated_ir;
                for coef in fir_hp_coefs.0.iter_mut() {
                    *coef = -*coef;
                }
                fir_hp_coefs.0[FILTER_SIZE / 2] += 1.0;

                self.band_filters[num_bands - 1].recompute_coefficients(
                    fir_hp_coefs,
                    &*self.r2c_plan,
                    &mut self.real_scratch_buffer,
                    &mut self.complex_scratch_buffer,
                );
            }
        }
    }

    /// Reset the internal filter state for all crossovers.
    pub fn reset(&mut self) {
        for filter in self.band_filters.iter_mut() {
            filter.reset();
        }

        // The inputs don't need to be reset as they'll be overwritten immediately
        for band_buffers in self.band_output_buffers.iter_mut() {
            for buffer in band_buffers {
                buffer.fill(0.0);
            }
        }

        // This being 0 means that the very first period will simply output the silence form above
        // and gather input for the next FFT
        self.io_buffers_next_indices.fill(0);
    }
}
