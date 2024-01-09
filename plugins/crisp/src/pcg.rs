// Crisp: a distortion plugin but not quite
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

//! A minimal implementation of `pcg32i` PRNG from the PCG library. Implemented separately instead
//! of using the rand crate implementation so we can adapt this for SIMD use.
//!
//! <https://github.com/imneme/pcg-c/blob/master/include/pcg_variants.h>
//! <https://www.pcg-random.org/using-pcg-c.html>

const PCG_DEFAULT_MULTIPLIER_32: u32 = 747796405;

/// The `pcg32i` PRNG from PCG.
#[derive(Copy, Clone)]
pub struct Pcg32iState {
    state: u32,
    inc: u32,
}

impl Pcg32iState {
    /// Initialize the PRNG, aka `*_srandom()`.
    ///
    /// <https://github.com/imneme/pcg-c/blob/83252d9c23df9c82ecb42210afed61a7b42402d7/include/pcg_variants.h#L757-L765>
    pub const fn new(state: u32, sequence: u32) -> Self {
        let mut rng = Self {
            state: 0,
            inc: (sequence << 1) | 1,
        };

        // https://github.com/imneme/pcg-c/blob/83252d9c23df9c82ecb42210afed61a7b42402d7/include/pcg_variants.h#L540-L543,
        // inlined so we can make this a const function
        rng.state = rng
            .state
            .wrapping_mul(PCG_DEFAULT_MULTIPLIER_32)
            .wrapping_add(rng.inc);
        rng.state += state;
        rng.state = rng
            .state
            .wrapping_mul(PCG_DEFAULT_MULTIPLIER_32)
            .wrapping_add(rng.inc);

        rng
    }

    /// Generate a new uniformly distirubted `u32` covering all possible values.
    ///
    /// <https://github.com/imneme/pcg-c/blob/83252d9c23df9c82ecb42210afed61a7b42402d7/include/pcg_variants.h#L1711-L1717>
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old_state = self.state;
        self.state = self
            .state
            .wrapping_mul(PCG_DEFAULT_MULTIPLIER_32)
            .wrapping_add(self.inc);

        let word = ((old_state >> ((old_state >> 28) + 4)) ^ old_state).wrapping_mul(277803737);
        (word >> 22) ^ word
    }

    /// Generate a new `f32` value in the open `(0, 1)` range.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        const FLOAT_SIZE: u32 = std::mem::size_of::<f32>() as u32 * 8;

        // Implementation from https://docs.rs/rand/0.8.4/rand/distributions/struct.Open01.html
        let value = self.next_u32();
        let fraction = value >> (FLOAT_SIZE - f32::MANTISSA_DIGITS - 1);

        let exponent_bits: u32 = ((f32::MAX_EXP - 1) as u32) << (f32::MANTISSA_DIGITS - 1);
        f32::from_bits(fraction | exponent_bits) - (1.0 - f32::EPSILON / 2.0)
    }
}
