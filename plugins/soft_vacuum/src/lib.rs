// Soft Vacuum: Airwindows Hard Vacuum port with oversampling
// Copyright (C) 2023-2024 Robbert van der Helm
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
use std::sync::atomic::Ordering;
use std::sync::Arc;

use nih_plug::prelude::*;

mod hard_vacuum;
mod oversampling;

/// The maximum number of samples to process at a time. Used to create scratch buffers for the
/// oversampling.
const MAX_BLOCK_SIZE: usize = 32;

/// The 2-logarithm of the maximum oversampling amount to use. 16x oversampling corresponds to factor
/// 4.
const MAX_OVERSAMPLING_FACTOR: usize = 4;
const MAX_OVERSAMPLING_TIMES: usize = oversampling_factor_to_times(MAX_OVERSAMPLING_FACTOR);
const MAX_OVERSAMPLED_BLOCK_SIZE: usize = MAX_BLOCK_SIZE * MAX_OVERSAMPLING_TIMES;

/// This corresponds to 2x oversampling.
const DEFAULT_OVERSAMPLING_FACTOR: usize = 1;

struct SoftVacuum {
    params: Arc<SoftVacuumParams>,

    /// Stores implementations of the Hard Vacuum algorithm for each channel, since each channel
    /// needs to maintain its own state.
    hard_vacuum_processors: Vec<hard_vacuum::HardVacuum>,
    /// Oversampling for each channel.
    oversamplers: Vec<oversampling::Lanczos3Oversampler>,
    /// Oversampling for each channel's slew control signal. This is upsampled separately to make
    /// the oversampled algorithm sound similar to the regular, non oversampled version as the slews
    /// will necessarily be lower in the oversampled version.
    slew_oversamplers: Vec<oversampling::Lanczos3Oversampler>,

    /// Scratch buffers that the smoothed parameters can be rendered to. Allocated on the heap
    /// because Windows uses tiny stack sizes which may eventually cause problems in some hosts.
    scratch_buffers: Box<ScratchBuffers>,
}

struct ScratchBuffers {
    // These are for the Hard Vacuum parameters
    drive: [f32; MAX_OVERSAMPLED_BLOCK_SIZE],
    warmth: [f32; MAX_OVERSAMPLED_BLOCK_SIZE],
    aura: [f32; MAX_OVERSAMPLED_BLOCK_SIZE],

    // These are for the mix parameters
    output_gain: [f32; MAX_OVERSAMPLED_BLOCK_SIZE],
    dry_wet_ratio: [f32; MAX_OVERSAMPLED_BLOCK_SIZE],
}

impl Default for ScratchBuffers {
    fn default() -> Self {
        Self {
            drive: [0.0; MAX_OVERSAMPLED_BLOCK_SIZE],
            warmth: [0.0; MAX_OVERSAMPLED_BLOCK_SIZE],
            aura: [0.0; MAX_OVERSAMPLED_BLOCK_SIZE],
            output_gain: [0.0; MAX_OVERSAMPLED_BLOCK_SIZE],
            dry_wet_ratio: [0.0; MAX_OVERSAMPLED_BLOCK_SIZE],
        }
    }
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

    /// The current oversampling factor. This is the 2-logarithm of the oversampling amount. 0
    /// corresponds to 1x/no oversampling, 1 to 2x oversampling, 2 to 4x, etc..
    #[id = "oversampling_factor"]
    pub oversampling_factor: IntParam,
}

impl Default for SoftVacuumParams {
    fn default() -> Self {
        // This is set by the `oversampling_factor` parameter and is used by the smoothers of the
        // other parmaeters so the oversampling amount always stays in sync
        let oversampling_times = Arc::new(AtomicF32::new(oversampling_factor_to_times(
            DEFAULT_OVERSAMPLING_FACTOR,
        ) as f32));

        Self {
            // Goes up to 200%, with the second half being nonlinear
            drive: FloatParam::new("Drive", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 })
                .with_unit("%")
                .with_smoother(SmoothingStyle::OversamplingAware(
                    oversampling_times.clone(),
                    &SmoothingStyle::Linear(20.0),
                ))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            warmth: FloatParam::new("Warmth", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_smoother(SmoothingStyle::OversamplingAware(
                    oversampling_times.clone(),
                    &SmoothingStyle::Linear(10.0),
                ))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            aura: FloatParam::new("Aura", 0.0, FloatRange::Linear { min: 0.0, max: PI })
                .with_unit("%")
                .with_smoother(SmoothingStyle::OversamplingAware(
                    oversampling_times.clone(),
                    &SmoothingStyle::Linear(10.0),
                ))
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
                    min: util::db_to_gain(-40.0),
                    max: util::db_to_gain(0.0),
                    factor: FloatRange::gain_skew_factor(-40.0, 0.0),
                },
            )
            .with_unit(" dB")
            // The value does not go down to 0 so we can do logarithmic here
            .with_smoother(SmoothingStyle::OversamplingAware(
                oversampling_times.clone(),
                &SmoothingStyle::Logarithmic(10.0),
            ))
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            dry_wet_ratio: FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_smoother(SmoothingStyle::OversamplingAware(
                    oversampling_times.clone(),
                    &SmoothingStyle::Linear(10.0),
                ))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            oversampling_factor: IntParam::new(
                "Oversampling",
                DEFAULT_OVERSAMPLING_FACTOR as i32,
                IntRange::Linear {
                    min: 0,
                    max: MAX_OVERSAMPLING_FACTOR as i32,
                },
            )
            .with_unit("x")
            .with_callback(Arc::new(move |new_factor| {
                oversampling_times.store(
                    oversampling_factor_to_times(new_factor as usize) as f32,
                    Ordering::Relaxed,
                );
            }))
            .with_value_to_string(Arc::new(|value| {
                // NIH-plug prevents `value` from being out of range and thus negative
                let oversampling_times = oversampling_factor_to_times(value as usize);

                oversampling_times.to_string()
            }))
            .with_string_to_value(Arc::new(|string| {
                let oversampling_times: usize = string.parse().ok()?;

                Some(oversampling_times_to_factor(oversampling_times) as i32)
            })),
        }
    }
}

impl Default for SoftVacuum {
    fn default() -> Self {
        Self {
            params: Arc::new(SoftVacuumParams::default()),

            hard_vacuum_processors: Vec::new(),
            oversamplers: Vec::new(),
            slew_oversamplers: Vec::new(),

            scratch_buffers: Box::default(),
        }
    }
}

impl Plugin for SoftVacuum {
    const NAME: &'static str = "Soft Vacuum";
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

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        let num_channels = audio_io_layout
            .main_output_channels
            .expect("Plugin was initialized without any outputs")
            .get() as usize;

        self.hard_vacuum_processors
            .resize_with(num_channels, hard_vacuum::HardVacuum::default);
        self.oversamplers.resize_with(num_channels, || {
            oversampling::Lanczos3Oversampler::new(MAX_BLOCK_SIZE, MAX_OVERSAMPLING_FACTOR)
        });
        self.slew_oversamplers.resize_with(num_channels, || {
            oversampling::Lanczos3Oversampler::new(MAX_BLOCK_SIZE, MAX_OVERSAMPLING_FACTOR)
        });

        if let Some(oversampler) = self.oversamplers.first() {
            context.set_latency_samples(
                oversampler.latency(self.params.oversampling_factor.value() as usize),
            );
        }

        true
    }

    fn reset(&mut self) {
        for hard_vacuum in &mut self.hard_vacuum_processors {
            hard_vacuum.reset();
        }

        for oversampler in &mut self.oversamplers {
            oversampler.reset();
        }
        for oversampler in &mut self.slew_oversamplers {
            oversampler.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let oversampling_factor = self.params.oversampling_factor.value() as usize;
        let oversampling_times = oversampling_factor_to_times(oversampling_factor);

        // If the oversampling factor parameter is changed then the host needs to know about the new
        // latency
        if let Some(oversampler) = self.oversamplers.first() {
            context.set_latency_samples(oversampler.latency(oversampling_factor));
        }

        for (_, block) in buffer.iter_blocks(MAX_BLOCK_SIZE) {
            let block_len = block.samples();
            let upsampled_block_len = block_len * oversampling_times;

            // These are the parameters for the distortion algorithm
            let drive = &mut self.scratch_buffers.drive;
            self.params
                .drive
                .smoothed
                .next_block(drive, upsampled_block_len);
            let warmth = &mut self.scratch_buffers.warmth;
            self.params
                .warmth
                .smoothed
                .next_block(warmth, upsampled_block_len);
            let aura = &mut self.scratch_buffers.aura;
            self.params
                .aura
                .smoothed
                .next_block(aura, upsampled_block_len);

            // And the general output mixing
            let output_gain = &mut self.scratch_buffers.output_gain;
            self.params
                .output_gain
                .smoothed
                .next_block(output_gain, upsampled_block_len);
            let dry_wet_ratio = &mut self.scratch_buffers.dry_wet_ratio;
            self.params
                .dry_wet_ratio
                .smoothed
                .next_block(dry_wet_ratio, upsampled_block_len);

            for (block_channel, ((oversampler, slew_oversampler), hard_vacuum)) in
                block.into_iter().zip(
                    self.oversamplers
                        .iter_mut()
                        .zip(self.slew_oversamplers.iter_mut())
                        .zip(self.hard_vacuum_processors.iter_mut()),
                )
            {
                // The slew signal is computed and oversampled first. This is then used as a control
                // signal in the oversampled version of the algorithm so it sounds more similar to
                // the non-oversampled version. Otherwise the slews are necessarily going to be much
                // lower.
                let mut slews = [0.0f32; MAX_BLOCK_SIZE];
                for (sample, slew) in block_channel.iter().zip(slews.iter_mut()) {
                    *slew = hard_vacuum.compute_slew(*sample);
                }

                let upsampled_slews =
                    slew_oversampler.upsample_only(&mut slews, oversampling_factor);

                oversampler.process(block_channel, oversampling_factor, |upsampled| {
                    assert!(upsampled.len() == upsampled_block_len);

                    for (sample_idx, (sample, slew)) in
                        upsampled.iter_mut().zip(upsampled_slews).enumerate()
                    {
                        // SAFETY: We already made sure that the blocks are equal in size. We could
                        //         zip iterators instead but with six iterators that's already a bit
                        //         too much without a first class way to zip more than two iterators
                        //         together into a single tuple of iterators.
                        let hard_vacuum_params = hard_vacuum::Params {
                            drive: unsafe { *drive.get_unchecked(sample_idx) },
                            warmth: unsafe { *warmth.get_unchecked(sample_idx) },
                            aura: unsafe { *aura.get_unchecked(sample_idx) },
                        };
                        let output_gain = unsafe { *output_gain.get_unchecked(sample_idx) };
                        let dry_wet_ratio = unsafe { *dry_wet_ratio.get_unchecked(sample_idx) };

                        let distorted =
                            hard_vacuum.process_with_slew(*sample, &hard_vacuum_params, *slew);
                        *sample = (distorted * output_gain * dry_wet_ratio)
                            + (*sample * (1.0 - dry_wet_ratio));
                    }
                });
            }
        }

        ProcessStatus::Normal
    }
}

// Used in the conversion for the oversampling amount parameter
const fn oversampling_factor_to_times(factor: usize) -> usize {
    2usize.pow(factor as u32)
}

const fn oversampling_times_to_factor(times: usize) -> usize {
    times.ilog2() as usize
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
