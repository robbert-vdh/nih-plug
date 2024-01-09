// Soft Vacuum: Airwindows Hard Vacuum port with oversampling
// Copyright (C) 2023-2024 Robbert van der Helm
// Copyright (c) 2018 Chris Johnson
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

use std::f32::consts::{FRAC_PI_2, PI};

use nih_plug::nih_debug_assert;

/// For some reason this constant is used quite a few times in the Hard Vacuum implementation. I'm
/// pretty sure it's a typo.
const ALMOST_FRAC_PI_2: f32 = 1.557_079_7;

/// Single-channel port of the Hard Vacuum algorithm from
/// <https://github.com/airwindows/airwindows/blob/283343b9e90c28fdb583f27e198f882f268b051b/plugins/LinuxVST/src/HardVacuum/HardVacuumProc.cpp>.
#[derive(Debug, Default)]
pub struct HardVacuum {
    last_sample: f32,
}

/// Parameters for the [`HardVacuum`] algorithm. This is a struct to make it easier to reuse the
/// same values for multiple channels.
pub struct Params {
    /// The 'drive' parameter, should be in the range `[0, 2]`. Controls both the drive and how many
    /// distortion stages are applied.
    pub drive: f32,
    /// The 'warmth' parameter, should be in the range `[0, 1]`.
    pub warmth: f32,
    /// The 'aura' parameter, should be in the range `[0, pi]`.
    pub aura: f32,
}

impl HardVacuum {
    /// Reset the processor's state. In this case this only resets the discrete derivative
    /// calculation. Doesn't make a huge difference but it's still useful to make the effect
    /// deterministic.
    pub fn reset(&mut self) {
        self.last_sample = 0.0;
    }

    /// Process a sample for a single channel. Because this maintains per-channel internal state,
    /// you should use different [`HardVacuum`] objects for each channel when processing
    /// multichannel audio.
    ///
    /// Output scaling and dry/wet mixing should be done externally.
    #[allow(unused)]
    pub fn process(&mut self, input: f32, params: &Params) -> f32 {
        let slew = self.compute_slew(input);

        self.process_with_slew(input, params, slew)
    }

    /// Compute only the slew value. Used together with `process_with_slew()` to compute the slews,
    /// upsample that, and then process the upsampled signal using those upsampled slews so the
    /// oversampled version ends up sounding more similar to the original algorithm.
    pub fn compute_slew(&mut self, input: f32) -> f32 {
        // AW: skew will be direction/angle
        let skew = input - self.last_sample;
        self.last_sample = input;

        skew
    }

    /// The same as `process()`, but with an externally computed slew value (`input - last_value`).
    /// This is useful for the oversampled version of this algorithm as we can upsample the slew
    /// signal separately.
    pub fn process_with_slew(&self, input: f32, params: &Params, slew: f32) -> f32 {
        // We'll skip a couple unnecessary things here like the dithering and the manual denormal
        // evasion
        nih_debug_assert!((0.0..=2.0).contains(&params.drive));
        nih_debug_assert!((0.0..=1.0).contains(&params.warmth));
        nih_debug_assert!((0.0..=PI).contains(&params.aura));

        // These two values are derived from the warmth parameter in an ...interesting way
        let scaled_warmth = params.warmth / FRAC_PI_2;
        let inverse_warmth = 1.0 - params.warmth;

        // AW: We're doing all this here so skew isn't incremented by each stage
        let skew = {
            // AW: skew will be direction/angle
            let skew = slew;
            // AW: for skew we want it to go to zero effect again, so we use full range of the sine
            let bridge_rectifier = skew.abs().min(PI).sin();

            // AW: skew is now sined and clamped and then re-amplified again
            // AW @ the `* 1.557` part: cools off sparkliness and crossover distortion
            // NOTE: The 1.55707 is presumably a typo in the original plugin. `pi/2` is 1.5707...,
            //       and this one has an additional 5 in there.
            skew.signum() * bridge_rectifier * params.aura * input * ALMOST_FRAC_PI_2
        };

        // AW: WE MAKE LOUD NOISE! RAWWWK!
        let mut remaining_distortion_stages = if params.drive > 1.0 {
            params.drive * params.drive
        } else {
            params.drive
        };

        // AW: crank up the gain on this so we can make it sing
        let mut output = input;
        while remaining_distortion_stages > 0.0 {
            // AW: full crank stages followed by the proportional one whee. 1 at full warmth to
            //     1.5570etc at no warmth
            let drive = if remaining_distortion_stages > 1.0 {
                ALMOST_FRAC_PI_2
            } else {
                remaining_distortion_stages * (1.0 + ((ALMOST_FRAC_PI_2 - 1.0) * inverse_warmth))
            };

            // AW: set up things so we can do repeated iterations, assuming that wet is always going
            //     to be 0-1 as in the previous plug.
            let bridge_rectifier = (output.abs() + skew).min(FRAC_PI_2).sin();
            // AW: the distortion section.
            let bridge_rectifier = bridge_rectifier.mul_add(drive, skew).min(FRAC_PI_2).sin();
            output = if output > 0.0 {
                let positive = drive - scaled_warmth;
                (output * (1.0 - positive + skew)) + (bridge_rectifier * (positive + skew))
            } else {
                let negative = drive + scaled_warmth;
                (output * (1.0 - negative + skew)) - (bridge_rectifier * (negative + skew))
            };

            remaining_distortion_stages -= 1.0;
        }

        output
    }
}
