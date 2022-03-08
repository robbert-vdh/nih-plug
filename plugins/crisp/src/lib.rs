// Crisp: a distortion plugin but not quite
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

use nih_plug::prelude::*;
use std::pin::Pin;

/// Hardcoded to make SIMD-ifying this a bit easier in the future
const NUM_CHANNELS: usize = 2;

/// These seeds being fixed makes bouncing deterministic.
const INITIAL_PRNG_SEEDS: [u32; 2] = [69, 420];

/// This plugin essentially layers the sound with another copy of the signal AM'ed with white (or
/// filtered) noise. That other copy of the sound may have a low pass filter applied to it since
/// this effect just turns into literal noise at high frequencies.
struct Crisp {
    params: Pin<Box<CrispParams>>,

    /// The OS RNG is only used for the initial seeds, after that we'll implement PCG ourselves so
    /// we can easily SIMD-ify this in the future.
    prng_seeds: [u32; NUM_CHANNELS],
}

// TODO: Filters
// TODO: Mono/stereo/mid-side switch
// TODO: Output gain
#[derive(Params)]
pub struct CrispParams {
    /// On a range of `[0, 1]`, how much of the modulated sound to mix in.
    #[id = "amount"]
    amount: FloatParam,
}

impl Default for Crisp {
    fn default() -> Self {
        Self {
            params: Box::pin(CrispParams::default()),

            prng_seeds: INITIAL_PRNG_SEEDS,
        }
    }
}

impl Default for CrispParams {
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self {
            amount: FloatParam::new("Amount", 0.2, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(10.0))
                .with_unit("%")
                .with_value_to_string(formatters::f32_percentage(0))
                .with_string_to_value(formatters::from_f32_percentage()),
        }
    }
}

impl Plugin for Crisp {
    const NAME: &'static str = "Crisp";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.1.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // We'll add a SIMD version in a bit which only supports stereo
        config.num_input_channels == config.num_output_channels && config.num_input_channels == 2
    }

    fn reset(&mut self) {
        // By using the same seeds each time bouncing can be made deterministic
        self.prng_seeds = INITIAL_PRNG_SEEDS;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_mut() {
            let amount = self.params.amount.smoothed.next();

            // TODO: SIMD-ize this to process both channels at once
            for (channel_idx, sample) in channel_samples.into_iter().enumerate() {
                // TODO: Calculate some uniformly (or Gaussian?) distributed white noise in the
                //       range of `[-1, 1]`, add that scaled by `amount` to `sample`.
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Crisp {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.crisp";
    const CLAP_DESCRIPTION: &'static str = "Adds a bright crispy top end to low bass sounds";
    const CLAP_FEATURES: &'static [&'static str] =
        &["audio_effect", "stereo", "filter", "distortion"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Crisp {
    const VST3_CLASS_ID: [u8; 16] = *b"CrispPluginRvdH.";
    const VST3_CATEGORIES: &'static str = "Fx|Filter|Distortion";
}

nih_export_clap!(Crisp);
nih_export_vst3!(Crisp);
