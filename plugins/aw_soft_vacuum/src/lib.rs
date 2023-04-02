// Soft Vacuum: Airwindows Hard Vacuum port with oversampling
// Copyright (C) 2023 Robbert van der Helm
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

use std::f32::consts::PI;
use std::sync::Arc;

use nih_plug::prelude::*;

mod hard_vacuum;

struct SoftVacuum {
    params: Arc<SoftVacuumParams>,

    /// Stores implementations of the Hard Vacuum algorithm for each channel, since each channel
    /// needs to maintain its own state.
    hard_vacuum_processors: Vec<hard_vacuum::HardVacuum>,
}

// The parameters are the same as in the original plugin, except that they have different value
// names
#[derive(Params)]
struct SoftVacuumParams {
    /// The drive/multistage parameter. Goes from `[0, 2]`, which is displayed as `0%` through
    /// `200%`. Above 100% up to four distortion stages are applied.
    #[id = "drive"]
    drive: FloatParam,
    /// The 'warmth' DC bias parameter. Shown as a percentage in this version.
    #[id = "warmth"]
    warmth: FloatParam,
    /// The 'aura' parameter which is essentially extra input gain. Shown as a percentage, but maps
    /// to a `[0, pi]` value.
    #[id = "aura"]
    aura: FloatParam,

    /// The output gain, shown in decibel.
    #[id = "output_gain"]
    pub output_gain: FloatParam,
    /// A linear dry/wet mix parameter.
    #[id = "dry_wet_ratio"]
    pub dry_wet_ratio: FloatParam,
}

impl Default for SoftVacuumParams {
    fn default() -> Self {
        Self {
            // Goes up to 200%, with the second half being nonlinear
            drive: FloatParam::new("Drive", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 })
                .with_unit("%")
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            warmth: FloatParam::new("Warmth", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_smoother(SmoothingStyle::Linear(10.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            aura: FloatParam::new("Aura", 0.0, FloatRange::Linear { min: 0.0, max: PI })
                .with_unit("%")
                .with_smoother(SmoothingStyle::Linear(10.0))
                // We're displaying the value as a percentage even though it goes from `[0, pi]`
                .with_value_to_string({
                    let formatter = formatters::v2s_f32_percentage(0);
                    Arc::new(move |value| formatter(value / PI))
                })
                .with_string_to_value({
                    let formatter = formatters::s2v_f32_percentage();
                    Arc::new(move |string| formatter(string).map(|value| value * PI))
                }),

            output_gain: FloatParam::new(
                "Output Gain",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    // This doesn't go down to 0.0 so we can use logarithmic smoothing
                    min: util::MINUS_INFINITY_GAIN,
                    max: util::db_to_gain(0.0),
                    factor: FloatRange::gain_skew_factor(util::MINUS_INFINITY_DB, 0.0),
                },
            )
            .with_unit(" dB")
            // The value does not go down to 0 so we can do logarithmic here
            .with_smoother(SmoothingStyle::Logarithmic(10.0))
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            dry_wet_ratio: FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_smoother(SmoothingStyle::Linear(10.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

impl Default for SoftVacuum {
    fn default() -> Self {
        Self {
            params: Arc::new(SoftVacuumParams::default()),

            hard_vacuum_processors: Vec::new(),
        }
    }
}

impl Plugin for SoftVacuum {
    const NAME: &'static str = "Soft Vacuum (Hard Vacuum port)";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let num_channels = audio_io_layout
            .main_output_channels
            .expect("Plugin was initialized without any outputs")
            .get() as usize;
        self.hard_vacuum_processors
            .resize_with(num_channels, hard_vacuum::HardVacuum::default);

        true
    }

    fn reset(&mut self) {
        for hard_vacuum in &mut self.hard_vacuum_processors {
            hard_vacuum.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // TODO: Parameters
        // TODO: Dry/wet mixing and output scaling
        // TODO: Oversampling
        for channel_samples in buffer.iter_samples() {
            let output_gain = self.params.output_gain.smoothed.next();
            let dry_wet_ratio = self.params.dry_wet_ratio.smoothed.next();
            let hard_vacuum_params = hard_vacuum::Params {
                drive: self.params.drive.smoothed.next(),
                warmth: self.params.warmth.smoothed.next(),
                aura: self.params.aura.smoothed.next(),
            };

            for (sample, hard_vacuum) in channel_samples
                .into_iter()
                .zip(self.hard_vacuum_processors.iter_mut())
            {
                let distorted = hard_vacuum.process(*sample, &hard_vacuum_params);
                *sample =
                    (distorted * output_gain * dry_wet_ratio) + (*sample * (1.0 - dry_wet_ratio));
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for SoftVacuum {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.soft-vacuum";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Airwindows Hard Vacuum port with oversampling");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Mono,
        ClapFeature::Distortion,
    ];
}

impl Vst3Plugin for SoftVacuum {
    const VST3_CLASS_ID: [u8; 16] = *b"SoftVacuum.RvdH.";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

nih_export_clap!(SoftVacuum);
nih_export_vst3!(SoftVacuum);
