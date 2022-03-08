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
use pcg::Pcg32iState;
use std::pin::Pin;
use std::sync::Arc;

mod pcg;

/// These seeds being fixed makes bouncing deterministic.
const INITIAL_PRNG_SEED: Pcg32iState = Pcg32iState::new(69, 420);

/// Allow 100% amount to scale the gain to a bit above 100%, to make the effect even less subtle.
const AMOUNT_GAIN_MULTIPLIER: f32 = 2.0;

/// This plugin essentially layers the sound with another copy of the signal AM'ed with white (or
/// filtered) noise. That other copy of the sound may have a low pass filter applied to it since
/// this effect just turns into literal noise at high frequencies.
struct Crisp {
    params: Pin<Box<CrispParams>>,

    /// A PRNG for generating noise, after that we'll implement PCG ourselves so we can easily
    /// SIMD-ify this in the future.
    prng: Pcg32iState,
}

// TODO: Filters
// TODO: Mono/stereo/mid-side switch
#[derive(Params)]
pub struct CrispParams {
    /// On a range of `[0, 1]`, how much of the modulated sound to mix in.
    #[id = "amount"]
    amount: FloatParam,
    /// What kind of AM to apply. The preset this was modelled after whether intentional or not only
    /// AMs the positive part of the waveform.
    #[id = "mode"]
    mode: EnumParam<Mode>,

    /// Output gain, as voltage gain. Displayed in decibels.
    #[id = "output"]
    output_gain: FloatParam,
}

/// Controls the type of modulation to apply.
#[derive(Enum, Debug, PartialEq)]
enum Mode {
    /// AM the entire waveform.
    Crispy,
    /// AM only the positive part of the waveform.
    #[name = "Even Crispier"]
    EvenCrispier,
    /// AM only the negative part of the waveform.
    #[name = "Even Crispier (alt)"]
    EvenCrispierNegated,
}

impl Default for Crisp {
    fn default() -> Self {
        Self {
            params: Box::pin(CrispParams::default()),

            prng: INITIAL_PRNG_SEED,
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
            mode: EnumParam::new("Mode", Mode::EvenCrispier),
            output_gain: FloatParam::new(
                "Output",
                1.0,
                // Because we're representing gain as decibels the range is already logarithmic
                FloatRange::Linear {
                    min: util::db_to_gain(-24.0),
                    max: util::db_to_gain(0.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(10.0))
            .with_unit(" dB")
            .with_value_to_string(Arc::new(|value| format!("{:.2}", util::gain_to_db(value))))
            .with_string_to_value(Arc::new(|string| {
                string
                    .trim()
                    .trim_end_matches(" dB")
                    .parse()
                    .ok()
                    .map(util::db_to_gain)
            })),
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
        self.prng = INITIAL_PRNG_SEED;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_mut() {
            let amount = self.params.amount.smoothed.next() * AMOUNT_GAIN_MULTIPLIER;
            let output_gain = self.params.output_gain.smoothed.next();

            // TODO: SIMD-ize this to process both channels at once
            for sample in channel_samples.into_iter() {
                let noise = self.prng.next_f32() * 2.0 - 1.0;
                // TODO: Avoid branching here later
                let am_result = match self.params.mode.value() {
                    Mode::Crispy => *sample * noise,
                    Mode::EvenCrispier => sample.max(0.0) * noise,
                    Mode::EvenCrispierNegated => sample.max(0.0) * noise,
                };

                *sample += am_result * amount;
                *sample *= output_gain;
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
