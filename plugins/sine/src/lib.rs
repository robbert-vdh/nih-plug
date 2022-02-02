// nih-plug: plugins, but rewritten in Rust
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

#[macro_use]
extern crate nih_plug;

use nih_plug::{
    context::ProcessContext,
    formatters,
    param::{FloatParam, Param, Params, Range},
    plugin::{Buffer, BufferConfig, BusConfig, Plugin, ProcessStatus, Vst3Plugin},
    util,
};
use std::f32::consts;
use std::pin::Pin;

/// A test tone generator.
///
/// TODO: Add MIDI support, this seems like a nice minimal example for that.
struct Sine {
    params: Pin<Box<SineParams>>,
    sample_rate: f32,

    phase: f32,
}

#[derive(Params)]
struct SineParams {
    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "frequency"]
    pub frequency: FloatParam,
}

impl Default for Sine {
    fn default() -> Self {
        Self {
            params: Box::pin(SineParams::default()),
            sample_rate: 1.0,

            phase: 0.0,
        }
    }
}

impl Default for SineParams {
    fn default() -> Self {
        Self {
            gain: FloatParam {
                value: -10.0,
                range: Range::Linear {
                    min: -30.0,
                    max: 0.0,
                },
                name: "Gain",
                unit: " dB",
                value_to_string: formatters::f32_rounded(2),
                ..Default::default()
            },
            frequency: FloatParam {
                value: 420.0,
                range: Range::Skewed {
                    min: 1.0,
                    max: 20_000.0,
                    factor: Range::skew_factor(-2.0),
                },
                name: "Frequency",
                unit: " Hz",
                value_to_string: formatters::f32_rounded(0),
                ..Default::default()
            },
        }
    }
}

impl Plugin for Sine {
    const NAME: &'static str = "Sine Test Tone";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 0;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This can output to any number of channels, but it doesn't take any audio inputs
        config.num_input_channels == 0 && config.num_input_channels > 0
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &dyn ProcessContext,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;

        true
    }

    fn process(&mut self, buffer: &mut Buffer, _context: &dyn ProcessContext) -> ProcessStatus {
        let phase_delta = self.params.frequency.value / self.sample_rate;
        for samples in buffer.iter_mut() {
            let sine = (self.phase * consts::TAU).sin();

            self.phase += phase_delta;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }

            for sample in samples {
                // TODO: Parameter smoothing
                *sample = sine * util::db_to_gain(self.params.gain.value);
            }
        }

        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Sine {
    const VST3_CLASS_ID: [u8; 16] = *b"SineMoistestPlug";
    const VST3_CATEGORIES: &'static str = "Instrument|Synth|Tools";
}

nih_export_vst3!(Sine);
