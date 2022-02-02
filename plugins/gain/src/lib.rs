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
    buffer::Buffer,
    context::ProcessContext,
    formatters,
    param::{BoolParam, FloatParam, Param, Params, Range},
    plugin::{BufferConfig, BusConfig, Plugin, ProcessStatus, Vst3Plugin},
    util,
};
use parking_lot::RwLock;
use std::pin::Pin;

struct Gain {
    params: Pin<Box<GainParams>>,
}

#[derive(Params)]
struct GainParams {
    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "as_long_as_this_name_stays_constant"]
    pub the_field_name_can_change: BoolParam,

    /// This field isn't used in this exampleq, but anything written to the vector would be restored
    /// together with a preset/state file saved for this plugin. This can be useful for storign
    /// things like sample data.
    #[persist = "industry_secrets"]
    pub random_data: RwLock<Vec<f32>>,
}

impl Default for Gain {
    fn default() -> Self {
        Self {
            params: Box::pin(GainParams::default()),
        }
    }
}

impl Default for GainParams {
    fn default() -> Self {
        Self {
            gain: FloatParam {
                value: 0.0,
                value_changed: None,
                // If, for instance, updating this parameter would require other parts of the
                // plugin's internal state to be updated other values to also be updated, then you
                // can use a callback like this, where `requires_updates` is an `Arc<AtomicBool>`
                // that's also stored on the parameters struct:
                // value_changed: Some(Arc::new(move |_new| { requires_update.store(true, Ordering::Release); })),
                range: Range::Linear {
                    min: -30.0,
                    max: 30.0,
                },
                name: "Gain",
                unit: " dB",
                value_to_string: formatters::f32_rounded(2),
                string_to_value: None,
            },
            // For brevity's sake you can also use the default values. Don't forget to set the field
            // name, default value, and range though.
            the_field_name_can_change: BoolParam {
                value: false,
                name: "Important Value",
                ..Default::default()
            },
            random_data: RwLock::new(Vec::new()),
        }
    }
}

impl Plugin for Gain {
    const NAME: &'static str = "Gain";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This works with any symmetrical IO layout
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        _buffer_config: &BufferConfig,
        _context: &dyn ProcessContext,
    ) -> bool {
        // This plugin doesn't need any special initialization, but if you need to do anything
        // expensive then this would be the place. State is kept around while when the host
        // reconfigures the plugin.
        true
    }

    fn process(&mut self, buffer: &mut Buffer, _context: &dyn ProcessContext) -> ProcessStatus {
        // TODO: The wrapper should set FTZ if not yet enabled, mention ths in the process fuctnion
        for samples in buffer.iter_mut() {
            for sample in samples {
                // TODO: Smoothing
                *sample *= util::db_to_gain(self.params.gain.value);
            }
        }

        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Gain {
    const VST3_CLASS_ID: [u8; 16] = *b"GainMoistestPlug";
    const VST3_CATEGORIES: &'static str = "Fx|Dynamics";
}

nih_export_vst3!(Gain);
