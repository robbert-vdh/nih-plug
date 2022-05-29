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
use std::f32::consts;
use std::ops::{Add, Mul, Sub};
use std::simd::f32x2;

use crate::NUM_BANDS;

#[derive(Debug)]
pub struct IirCrossover {
    /// The kind of crossover to use. `.update_filters()` must be called after changing this.
    mode: IirCrossoverType,

    /// The crossovers. Depending on the number of bands argument passed to `.process()` one to four
    /// of these may be used.
    crossovers: [Crossover; NUM_BANDS - 1],
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

impl IirCrossover {
    /// Create a new multiband crossover processor. All filters will be configured to pass audio
    /// through as it. `.update()` needs to be called first to set up the filters, and `.reset()`
    /// can be called whenever the filter state must be cleared.
    pub fn new(mode: IirCrossoverType) -> Self {
        Self {
            mode,
            crossovers: Default::default(),
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
                for (crossover, band_channel_samples) in self
                    .crossovers
                    .iter_mut()
                    .zip(band_outputs.iter_mut())
                    .take(num_bands as usize - 1)
                {
                    let (lp_samples, hp_samples) = crossover.process_lr24(samples);

                    unsafe { band_channel_samples.from_simd_unchecked(lp_samples) };
                    samples = hp_samples;
                }

                // And the final high-passed result should be written to the last band
                unsafe { band_outputs[num_bands - 1].from_simd_unchecked(samples) };
            }
        }
    }

    /// Update the crossover frequencies for all filters. If the frequencies are not monotonic then
    /// this function will ensure that they are.
    pub fn update(&mut self, sample_rate: f32, mut frequencies: [f32; NUM_BANDS - 1]) {
        // Make sure the frequencies are monotonic
        for frequency_idx in 1..NUM_BANDS - 1 {
            if frequencies[frequency_idx] < frequencies[frequency_idx - 1] {
                frequencies[frequency_idx] = frequencies[frequency_idx - 1];
            }
        }

        match self.mode {
            IirCrossoverType::LinkwitzRiley24 => {
                const Q: f32 = std::f32::consts::FRAC_1_SQRT_2;
                for (crossover, frequency) in self.crossovers.iter_mut().zip(frequencies) {
                    let lp_coefs = BiquadCoefficients::lowpass(sample_rate, frequency, Q);
                    let hp_coefs = BiquadCoefficients::highpass(sample_rate, frequency, Q);
                    crossover.update_coefficients(lp_coefs, hp_coefs);
                }
            }
        }
    }

    /// Reset the internal filter state for all crossovers.
    pub fn reset(&mut self) {
        for crossover in &mut self.crossovers {
            crossover.reset();
        }
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

/// A simple biquad filter with functions for generating coefficients for second order low-pass and
/// high-pass filters. Since these filters have 3 dB of attenuation at the center frequency, we'll
/// two of them in series to get 6 dB of attenutation at the crossover point for the LR24
/// crossovers.
///
/// Based on <https://en.wikipedia.org/wiki/Digital_biquad_filter#Transposed_direct_forms>.
///
/// The type parameter T  should be either an `f32` or a SIMD type.
#[derive(Clone, Copy, Debug)]
pub struct Biquad<T> {
    pub coefficients: BiquadCoefficients<T>,
    s1: T,
    s2: T,
}

/// The coefficients `[b0, b1, b2, a1, a2]` for [`Biquad`]. These coefficients are all
/// prenormalized, i.e. they have been divided by `a0`.
///
/// The type parameter T  should be either an `f32` or a SIMD type.
#[derive(Clone, Copy, Debug)]
pub struct BiquadCoefficients<T> {
    b0: T,
    b1: T,
    b2: T,
    a1: T,
    a2: T,
}

/// Either an `f32` or some SIMD vector type of `f32`s that can be used with our biquads.
pub trait SimdType:
    Mul<Output = Self> + Sub<Output = Self> + Add<Output = Self> + Copy + Sized
{
    fn from_f32(value: f32) -> Self;
}

impl<T: SimdType> Default for Biquad<T> {
    /// Before setting constants the filter should just act as an identity function.
    fn default() -> Self {
        Self {
            coefficients: BiquadCoefficients::identity(),
            s1: T::from_f32(0.0),
            s2: T::from_f32(0.0),
        }
    }
}

impl<T: SimdType> Biquad<T> {
    /// Process a single sample.
    pub fn process(&mut self, sample: T) -> T {
        let result = self.coefficients.b0 * sample + self.s1;

        self.s1 = self.coefficients.b1 * sample - self.coefficients.a1 * result + self.s2;
        self.s2 = self.coefficients.b2 * sample - self.coefficients.a2 * result;

        result
    }

    /// Reset the state to zero, useful after making making large, non-interpolatable changes to the
    /// filter coefficients.
    pub fn reset(&mut self) {
        self.s1 = T::from_f32(0.0);
        self.s2 = T::from_f32(0.0);
    }
}

impl<T: SimdType> BiquadCoefficients<T> {
    /// Convert scalar coefficients into the correct vector type.
    pub fn from_f32s(scalar: BiquadCoefficients<f32>) -> Self {
        Self {
            b0: T::from_f32(scalar.b0),
            b1: T::from_f32(scalar.b1),
            b2: T::from_f32(scalar.b2),
            a1: T::from_f32(scalar.a1),
            a2: T::from_f32(scalar.a2),
        }
    }

    /// Filter coefficients that would cause the sound to be passed through as is.
    pub fn identity() -> Self {
        Self::from_f32s(BiquadCoefficients {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        })
    }

    /// Compute the coefficients for a low-pass filter.
    ///
    /// Based on <http://shepazu.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html>.
    pub fn lowpass(sample_rate: f32, frequency: f32, q: f32) -> Self {
        nih_debug_assert!(sample_rate > 0.0);
        nih_debug_assert!(frequency > 0.0);
        nih_debug_assert!(frequency < sample_rate / 2.0);
        nih_debug_assert!(q > 0.0);

        let omega0 = consts::TAU * (frequency / sample_rate);
        let cos_omega0 = omega0.cos();
        let alpha = omega0.sin() / (2.0 * q);

        // We'll prenormalize everything with a0
        let a0 = 1.0 + alpha;
        let b0 = ((1.0 - cos_omega0) / 2.0) / a0;
        let b1 = (1.0 - cos_omega0) / a0;
        let b2 = ((1.0 - cos_omega0) / 2.0) / a0;
        let a1 = (-2.0 * cos_omega0) / a0;
        let a2 = (1.0 - alpha) / a0;

        Self::from_f32s(BiquadCoefficients { b0, b1, b2, a1, a2 })
    }

    /// Compute the coefficients for a high-pass filter.
    ///
    /// Based on <http://shepazu.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html>.
    pub fn highpass(sample_rate: f32, frequency: f32, q: f32) -> Self {
        nih_debug_assert!(sample_rate > 0.0);
        nih_debug_assert!(frequency > 0.0);
        nih_debug_assert!(frequency < sample_rate / 2.0);
        nih_debug_assert!(q > 0.0);

        let omega0 = consts::TAU * (frequency / sample_rate);
        let cos_omega0 = omega0.cos();
        let alpha = omega0.sin() / (2.0 * q);

        // We'll prenormalize everything with a0
        let a0 = 1.0 + alpha;
        let b0 = ((1.0 + cos_omega0) / 2.0) / a0;
        let b1 = -(1.0 + cos_omega0) / a0;
        let b2 = ((1.0 + cos_omega0) / 2.0) / a0;
        let a1 = (-2.0 * cos_omega0) / a0;
        let a2 = (1.0 - alpha) / a0;

        Self::from_f32s(BiquadCoefficients { b0, b1, b2, a1, a2 })
    }
}

impl SimdType for f32 {
    #[inline(always)]
    fn from_f32(value: f32) -> Self {
        value
    }
}

impl SimdType for f32x2 {
    #[inline(always)]
    fn from_f32(value: f32) -> Self {
        f32x2::splat(value)
    }
}
