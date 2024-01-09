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

use nih_plug::buffer::ChannelSamples;
use nih_plug::debug::*;
use std::simd::f32x2;

use self::biquad::{Biquad, BiquadCoefficients, NEUTRAL_Q};
use crate::NUM_BANDS;

pub mod biquad;

#[derive(Debug)]
pub struct IirCrossover {
    /// The kind of crossover to use. `.update_filters()` must be called after changing this.
    mode: IirCrossoverType,

    /// The crossovers. Depending on the number of bands argument passed to `.process()` one to four
    /// of these may be used.
    crossovers: [Crossover; NUM_BANDS - 1],
    /// Used to compensate the earlier bands for the phase shift introduced in the higher bands.
    all_passes: AllPassCascade,
}

/// The type of IIR crossover to use.
#[derive(Debug, Clone, Copy)]
pub enum IirCrossoverType {
    /// Clean crossover with 24 dB/octave slopes and one period of delay in the power band. Stacks
    /// two Butterworth-style (i.e. $q = \frac{\sqrt{2}}{2}$) filters per crossover.
    LinkwitzRiley24,
}

/// A single crossover using multiple biquads in series to get steeper slopes. This can do both the
/// low-pass and the high-pass parts of the crossover.
#[derive(Debug, Clone, Default)]
struct Crossover {
    /// Filters for the low-pass section of the crossover. Not all filters may be used dependign on
    /// the crossover type.
    lp_filters: [Biquad<f32x2>; 2],
    /// Filters for the high-pass section of the crossover. Not all filters may be used dependign on
    /// the crossover type.
    hp_filters: [Biquad<f32x2>; 2],
}

/// The crossover is super simple and feeds the low-passed result to the next band output while
/// using the high-passed version as the input for the next band. Because the higher bands will thus
/// have had more filters applied to them, the lower bands need to have their phase response
/// adjusted to match the higher bands. So for the LR24 crossovers, low-passed band `n` will get a
/// second order all-pass for the frequencies corresponding to crossovers `n + 1..NUM_CROSSOVERS`
/// applied to it.
#[derive(Debug, Default)]
struct AllPassCascade {
    /// The aforementioned all-pass filters. This is indexed by `[crossover_idx][0..num_bands -
    /// crossover_index - 1]`. Ergo, if there are three crossovers, then the low-pass section from
    /// the first crossover needs to have `[0][0]` and `[0][1]` applied to it. The last band doesn't
    /// need any compensation, hence the `NUM_BANDS - 2`. The outer array is equal to the number of
    /// crossovers. It will never contain any filters, but this makes the code a bit nicer by
    /// needing an explicit check for this.
    ap_filters: [[Biquad<f32x2>; NUM_BANDS - 2]; NUM_BANDS - 1],

    /// The number of activate bands. Only coefficients for used bands are computed in `ap_filters`.
    num_bands: usize,
}

impl IirCrossover {
    /// Create a new multiband crossover processor. All filters will be configured to pass audio
    /// through as is. `.update()` needs to be called first to set up the filters, and `.reset()`
    /// can be called whenever the filter state must be cleared.
    pub fn new(mode: IirCrossoverType) -> Self {
        Self {
            mode,
            crossovers: Default::default(),
            all_passes: Default::default(),
        }
    }

    /// Split the signal into bands using the crossovers previously configured through `.update()`.
    /// The split bands will be written to `band_outputs`. `main_io` is not written to, and should
    /// be cleared separately.
    pub fn process(
        &mut self,
        num_bands: usize,
        main_io: &ChannelSamples,
        mut band_outputs: [ChannelSamples; NUM_BANDS],
    ) {
        nih_debug_assert!(num_bands >= 2);
        nih_debug_assert!(num_bands <= NUM_BANDS);
        // Required for the SIMD, so we'll just do a hard assert or the unchecked conversions will
        // be unsound
        assert!(main_io.len() == 2);

        let mut samples: f32x2 = unsafe { main_io.to_simd_unchecked() };
        match self.mode {
            IirCrossoverType::LinkwitzRiley24 => {
                for (crossover_idx, (crossover, band_channel_samples)) in self
                    .crossovers
                    .iter_mut()
                    .zip(band_outputs.iter_mut())
                    .take(num_bands - 1)
                    .enumerate()
                {
                    let (lp_samples, hp_samples) = crossover.process_lr24(samples);

                    // The low-pass result needs to have the same phase shift applied to it that
                    // higher bands would get
                    let lp_samples = self.all_passes.compensate_lr24(lp_samples, crossover_idx);

                    unsafe { band_channel_samples.from_simd_unchecked(lp_samples) };
                    samples = hp_samples;
                }

                // And the final high-passed result should be written to the last band
                unsafe { band_outputs[num_bands - 1].from_simd_unchecked(samples) };
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
        // NOTE: Currently we don't actually need to make sure that the frequencies are monotonic

        match self.mode {
            IirCrossoverType::LinkwitzRiley24 => {
                for (crossover, frequency) in self
                    .crossovers
                    .iter_mut()
                    .zip(frequencies)
                    .take(num_bands - 1)
                {
                    let lp_coefs = BiquadCoefficients::lowpass(sample_rate, frequency, NEUTRAL_Q);
                    let hp_coefs = BiquadCoefficients::highpass(sample_rate, frequency, NEUTRAL_Q);
                    crossover.update_coefficients(lp_coefs, hp_coefs);
                }
            }
        }

        self.all_passes
            .update_coefficients(sample_rate, num_bands, &frequencies);
    }

    /// Reset the internal filter state for all crossovers.
    pub fn reset(&mut self) {
        for crossover in &mut self.crossovers {
            crossover.reset();
        }

        self.all_passes.reset();
    }
}

impl Crossover {
    /// Process left and right audio samples through two low-pass and two high-pass filter stages.
    /// The resulting tuple contains the low-passed and the high-passed samples. Used for the
    /// Linkwitz-Riley 24 dB/octave crossover.
    pub fn process_lr24(&mut self, samples: f32x2) -> (f32x2, f32x2) {
        let mut low_passed = samples;
        for filter in &mut self.lp_filters[..2] {
            low_passed = filter.process(low_passed)
        }
        let mut high_passed = samples;
        for filter in &mut self.hp_filters[..2] {
            high_passed = filter.process(high_passed)
        }

        (low_passed, high_passed)
    }

    /// Update the coefficients for all filters in the crossover.
    pub fn update_coefficients(
        &mut self,
        lp_coefs: BiquadCoefficients<f32x2>,
        hp_coefs: BiquadCoefficients<f32x2>,
    ) {
        for filter in &mut self.lp_filters {
            filter.coefficients = lp_coefs;
        }
        for filter in &mut self.hp_filters {
            filter.coefficients = hp_coefs;
        }
    }

    /// Reset the internal filter state.
    pub fn reset(&mut self) {
        for filter in &mut self.lp_filters {
            filter.reset();
        }
        for filter in &mut self.hp_filters {
            filter.reset();
        }
    }
}

impl AllPassCascade {
    /// Compensate lower bands for the additional phase shift introduced in higher bands when using
    /// LR24 filters to split those bands.
    pub fn compensate_lr24(&mut self, lp_samples: f32x2, band_idx: usize) -> f32x2 {
        // The all-pass filters are set up based on the crossover that produced the low-passed
        // samples
        let crossover_idx = band_idx;

        // The idea here is that if `band_idx == 0`, and `self.num_bands == 3`, then there are two
        // crossovers, and `lp_samples` only needs to be filtered by `self.ap_filters[0][0]`. If
        // `self.num_bands` were 4 then it would additionally also be filtered by
        // `self.ap_filters[0][1]`.
        let mut compensated = lp_samples;
        for filter in &mut self.ap_filters[crossover_idx][..self.num_bands - band_idx - 2] {
            compensated = filter.process(compensated)
        }

        compensated
    }

    /// Update the coefficients for all filters in the cascade. For every active band, this adds up
    /// to `num_bands - band_idx - 1` filters. The filter state of course cannot be shared between
    /// bands, but the coefficients along the matrix's diagonals are identical.
    pub fn update_coefficients(
        &mut self,
        sample_rate: f32,
        num_bands: usize,
        frequencies: &[f32; NUM_BANDS - 1],
    ) {
        self.num_bands = num_bands;

        // All output bands go through the first filter, so we don't compensate for that. `band_idx`
        // starts at 1
        for (crossover_idx, crossover_frequency) in
            frequencies.iter().enumerate().take(num_bands - 1).skip(1)
        {
            let ap_coefs =
                BiquadCoefficients::allpass(sample_rate, *crossover_frequency, NEUTRAL_Q);

            // This sets the coefficients in a diagonal pattern. If `crossover_idx == 2`, then this
            // will set the coefficients for these filters:
            // ```
            // [_, x, ...] // Crossover 1 filters
            // [x, ...]    // Crossover 2 filters
            // ...
            // ```
            for target_crossover_idx in 0..crossover_idx {
                self.ap_filters[target_crossover_idx][crossover_idx - target_crossover_idx - 1]
                    .coefficients = ap_coefs;
            }
        }
    }

    /// Reset the internal filter state.
    pub fn reset(&mut self) {
        for filters in &mut self.ap_filters {
            for filter in filters.iter_mut() {
                filter.reset();
            }
        }
    }
}
