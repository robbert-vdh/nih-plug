// Spectral Compressor: an FFT based compressor
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

#[derive(Params)]
pub struct CompressorBankParams {
    // TODO: Target curve options
    /// The downwards compression threshold relative to the target curve.
    #[id = "thresh_down_off"]
    downwards_threshold_offset_db: FloatParam,
    /// The upwards compression threshold relative to the target curve.
    #[id = "thresh_up_off"]
    upwards_threshold_offset_db: FloatParam,

    /// A `[0, 1]` scaling factor that causes the compressors for the higher registers to have lower
    /// ratios than the compressors for the lower registers. The scaling is applied logarithmically
    /// rather than linearly over the compressors.
    ///
    /// TODO: Decide on whether or not this should only apply on upwards ratios, or if we may need
    ///       separate controls for both
    #[id = "ratio_hi_freq_rolloff"]
    high_freq_ratio_rolloff: FloatParam,
    /// The downwards compression ratio. At 1.0 the downwards compressor is disengaged.
    #[id = "ratio_down"]
    downwards_ratio: FloatParam,
    /// The upwards compression ratio. At 1.0 the upwards compressor is disengaged.
    #[id = "ratio_up"]
    upwards_ratio: FloatParam,

    /// The downwards compression knee width, in decibels.
    #[id = "knee_down_off"]
    downwards_knee_width_db: FloatParam,
    /// The upwards compression knee width, in decibels.
    #[id = "knee_up_off"]
    upwards_knee_width_db: FloatParam,

    /// The compressor's attack time in milliseconds. Controls both upwards and downwards
    /// compression.
    #[id = "attack"]
    compressor_attack_ms: FloatParam,
    /// The compressor's release time in milliseconds. Controls both upwards and downwards
    /// compression.
    #[id = "release"]
    compressor_release_ms: FloatParam,
}

impl Default for CompressorBankParams {
    fn default() -> Self {
        Self {
            // TODO: Set nicer default values for these things
            // As explained above, these offsets are relative to the target curve
            downwards_threshold_offset_db: FloatParam::new(
                "Downwards Offset",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
            upwards_threshold_offset_db: FloatParam::new(
                "Upwards Offset",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),

            high_freq_ratio_rolloff: FloatParam::new(
                "High-freq Ratio Rolloff",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            downwards_ratio: FloatParam::new(
                "Downwards Ratio",
                1.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 300.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_step_size(0.1)
            .with_value_to_string(formatters::v2s_compression_ratio(1))
            .with_string_to_value(formatters::s2v_compression_ratio()),
            upwards_ratio: FloatParam::new(
                "Upwards Ratio",
                1.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 300.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_step_size(0.1)
            .with_value_to_string(formatters::v2s_compression_ratio(1))
            .with_string_to_value(formatters::s2v_compression_ratio()),

            downwards_knee_width_db: FloatParam::new(
                "Downwards Knee",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 36.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
            upwards_knee_width_db: FloatParam::new(
                "Upwards Knee",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 36.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),

            compressor_attack_ms: FloatParam::new(
                "Attack",
                150.0,
                FloatRange::Skewed {
                    // TODO: Make sure to handle 0 attack and release times in the compressor
                    min: 0.0,
                    max: 10_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.1),
            compressor_release_ms: FloatParam::new(
                "Release",
                300.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 10_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.1),
        }
    }
}
