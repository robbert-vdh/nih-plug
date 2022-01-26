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

use nih_plug::{
    params::{FloatParam, Params, Range},
    plugin::{BufferConfig, BusConfig, Plugin},
};
use nih_plug_derive::Params;
use std::pin::Pin;

struct Gain {
    params: Pin<Box<GainParams>>,
}

#[derive(Params)]
struct GainParams {
    #[id("gain")]
    pub gain: FloatParam,
}

impl Default for Gain {
    fn default() -> Self {
        Self {
            params: Pin::new(Box::default()),
        }
    }
}

impl Default for GainParams {
    fn default() -> Self {
        Self {
            gain: FloatParam {
                value: 0.00,
                range: Range::Linear {
                    min: -30.0,
                    max: 300.0,
                },
                name: "Gain",
                unit: " dB",
                value_to_string: None,
                string_to_value: None,
            },
        }
    }
}

impl Plugin for Gain {
    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Pin<&dyn Params> {
        todo!()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This works with any symmetrical IO layout
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn initialize(&mut self, _bus_config: &BusConfig, _buffer_config: &BufferConfig) -> bool {
        // This plugin doesn't need any special initialization, but if you need to do anything
        // expensive then this would be the place. State is kept around while when the host
        // reconfigures the plugin.
        true
    }

    fn process(&mut self, samples: &mut [&mut [f32]]) {
        if samples.is_empty() {
            return;
        }

        // TODO: The wrapper should set FTZ if not yet enabled, mention ths in the process fuctnion
        // TODO: Move this iterator to an adapter
        let num_channels = samples.len();
        let num_samples = samples[0].len();
        for channel in &samples[1..] {
            if channel.len() != num_samples {
                // TODO: Debug assert
                eprintln!("Mismatched channel lengths, aborting");
                return;
            }
        }

        for sample_idx in 0..num_samples {
            for channel_idx in 0..num_channels {
                let sample = unsafe {
                    samples
                        .get_unchecked_mut(channel_idx)
                        .get_unchecked_mut(sample_idx)
                };

                // TODO: Smoothing
                // TODO: Gain to decibel function in a separate module, add a minus infinity check when I do
                *sample *= 10.0f32.powf(self.params.gain.value * 0.05);
            }
        }
    }
}
