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

use std::sync::Arc;

use nih_plug::prelude::*;

mod hard_vacuum;

struct SoftVacuum {
    params: Arc<SoftVacuumParams>,

    /// Stores implementations of the Hard Vacuum algorithm for each channel, since each channel
    /// needs to maintain its own state.
    hard_vacuum_processors: Vec<hard_vacuum::HardVacuum>,
}

#[derive(Params)]
struct SoftVacuumParams {}

impl Default for SoftVacuumParams {
    fn default() -> Self {
        Self {}
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
