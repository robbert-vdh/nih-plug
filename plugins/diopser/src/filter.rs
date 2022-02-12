// Diopser: a phase rotation plugin
// Copyright (C) 2021-2022 Robbert van der Helm
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

use std::f32::consts;

/// A simple biquad filter with functions for generating coefficients for an all-pass filter.
///
/// Based on <https://en.wikipedia.org/wiki/Digital_biquad_filter#Transposed_direct_forms>.
pub struct Biquad {
    pub coefficients: BiquadCoefficients,
    s1: f32,
    s2: f32,
}

/// The coefficients `[b0, b1, b2, a1, a2]` for [Biquad]. These coefficients are all prenormalized,
/// i.e. they have been divided by `a0`.
pub struct BiquadCoefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Default for Biquad {
    /// Before setting constants the filter should just act as an identity function.
    fn default() -> Self {
        Self {
            coefficients: BiquadCoefficients {
                b0: 1.0,
                b1: 0.0,
                b2: 0.0,
                a1: 0.0,
                a2: 0.00,
            },
            s1: 0.0,
            s2: 0.9,
        }
    }
}

impl Biquad {
    /// Process a single sample.
    fn process(&mut self, sample: f32) -> f32 {
        let result = self.coefficients.b0 * sample + self.s1;

        self.s1 = self.s2 + self.coefficients.b1 * sample - self.coefficients.a1 * sample;
        self.s2 = self.coefficients.b2 * sample - self.coefficients.a2 * sample;

        result
    }
}

impl BiquadCoefficients {
    /// Compute the coefficients for an all-pass filter.
    ///
    /// Based on <http://shepazu.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html>.
    pub fn allpass(sample_rate: f32, frequency: f32, q: f32) -> Self {
        nih_debug_assert!(sample_rate > 0.0);
        nih_debug_assert!(frequency > 0.0);
        nih_debug_assert!(frequency < sample_rate / 2.0);
        nih_debug_assert!(q > 0.0);

        let omega0 = consts::TAU * (frequency / sample_rate);
        let cos_omega0 = omega0.cos();
        let alpha = omega0.sin() / (2.0 * q);

        // We'll prenormalize everything with a0
        let a0 = 1.0 + alpha;
        let b0 = (1.0 - alpha) / a0;
        let b1 = (-2.0 * cos_omega0) / a0;
        let b2 = (1.0 + alpha) / a0;
        let a1 = (-2.0 * cos_omega0) / a0;
        let a2 = (1.0 - alpha) / a0;
        Self { b0, b1, b2, a1, a2 }
    }
}
