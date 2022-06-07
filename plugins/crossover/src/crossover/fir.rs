// Crossover: clean crossovers as a multi-out plugin
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

use nih_plug::buffer::ChannelSamples;
use nih_plug::debug::*;
use std::f32;
use std::simd::f32x2;

use self::filter::{FirCoefficients, FirFilter};
use crate::crossover::iir::biquad::{BiquadCoefficients, NEUTRAL_Q};
use crate::NUM_BANDS;

pub mod filter;

// TODO: Move this to FFT convolution so we can increase the filter size and improve low latency performance

/// The size of the FIR filter window, or the number of taps. The low frequency performance is
/// greatly limited by this.
const FILTER_SIZE: usize = 121;
/// The size of the FIR filter's ring buffer. This is `FILTER_SIZE` rounded up to the next power of
/// two.
const RING_BUFFER_SIZE: usize = FILTER_SIZE.next_power_of_two();

#[derive(Debug)]
pub struct FirCrossover {
    /// The kind of crossover to use. `.update_filters()` must be called after changing this.
    mode: FirCrossoverType,

    /// Filters for each of the bands. Depending on the number of bands argument passed to
    /// `.process()` two to five of these may be used. The first one always contains a low-pass
    /// filter, the last one always contains a high-pass filter, while the other bands will contain
    /// band-pass filters.
    band_filters: [FirFilter; NUM_BANDS],
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
        Self {
            mode,
            band_filters: Default::default(),
        }
    }

    /// Get the current latency in samples. This depends on the selected mode.
    pub fn latency(&self) -> u32 {
        // Actually, that's a lie, since we currently only do linear-phase filters with a constant
        // size
        match self.mode {
            FirCrossoverType::LinkwitzRiley24LinearPhase => (FILTER_SIZE / 2) as u32,
        }
    }

    /// Split the signal into bands using the crossovers previously configured through `.update()`.
    /// The split bands will be written to `band_outputs`. `main_io` is not written to, and should
    /// be cleared separately.
    pub fn process(
        &mut self,
        num_bands: usize,
        main_io: &ChannelSamples,
        band_outputs: [ChannelSamples; NUM_BANDS],
    ) {
        nih_debug_assert!(num_bands >= 2);
        nih_debug_assert!(num_bands <= NUM_BANDS);
        // Required for the SIMD, so we'll just do a hard assert or the unchecked conversions will
        // be unsound
        assert!(main_io.len() == 2);

        let samples: f32x2 = unsafe { main_io.to_simd_unchecked() };
        match self.mode {
            FirCrossoverType::LinkwitzRiley24LinearPhase => {
                // TODO: Everything is structured to be fast to compute for the IIR filters. Instead
                //       of doing two channels at the same time, it would probably be faster to use
                //       SIMD for the actual convolution so we can do 4 or 8 multiply-adds at the
                //       same time. Or perhaps a better way to spend the time, use FFT convolution
                //       for this.
                for (filter, mut output) in self
                    .band_filters
                    .iter_mut()
                    .zip(band_outputs)
                    .take(num_bands)
                {
                    let filtered_samples = filter.process(samples);

                    unsafe { output.from_simd_unchecked(filtered_samples) };
                }
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
                self.band_filters[0].coefficients = lp_fir_coefs;

                // For the band-pass filters and the final high-pass filter, we need to keep track
                // of the accumulated impulse response
                let mut accumulated_ir = self.band_filters[0].coefficients.clone();
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

                    band_filter.coefficients = fir_bp_coefs;
                }

                // And finally we can do a spectral inversion of the accumulated IR to the the last
                // band's high-pass filter
                let mut fir_hp_coefs = accumulated_ir;
                for coef in fir_hp_coefs.0.iter_mut() {
                    *coef = -*coef;
                }
                fir_hp_coefs.0[FILTER_SIZE / 2] += 1.0;

                self.band_filters[num_bands - 1].coefficients = fir_hp_coefs;
            }
        }
    }

    /// Reset the internal filter state for all crossovers.
    pub fn reset(&mut self) {
        for filter in &mut self.band_filters {
            filter.reset();
        }
    }
}
