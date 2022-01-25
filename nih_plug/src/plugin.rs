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

use std::pin::Pin;

use crate::params::Params;

/// Basic functionality that needs to be implemented by a plugin. The wrappers will use this to
/// expose the plugin in a particular plugin format.
///
/// This is super basic, and lots of things I didn't need or want to use yet haven't been
/// implemented. Notable missing features include:
///
/// - MIDI
/// - Sidechain inputs
/// - Multiple output busses
/// - Storing custom state, only the parameters are saved right now
/// - Sample accurate automation (this would be great, but sadly few hosts even support it so until
///   they do we'll ignore that it's a thing)
/// - Parameter update callbacks
/// - Parameter hierarchies/groups
/// - Outputting parameter changes from the plugin
/// - GUIs
pub trait Plugin: Sync {
    /// The default number of inputs. Some hosts like, like Bitwig and Ardour, use the defaults
    /// instead of setting up the busses properly.
    const DEFAULT_NUM_INPUTS: u32;
    /// The default number of inputs. Some hosts like, like Bitwig and Ardour, use the defaults
    /// instead of setting up the busses properly.
    const DEFAULT_NUM_OUTPUTS: u32;

    /// The plugin's parameters. The host will update the parameter values before calling
    /// `process()`. These parameters are identified by strings that should never change when the
    /// plugin receives an update.
    ///
    /// TODO: Rethink the API a bit more. Also Requiring the pin on self makes more sense, but it's
    ///       not strictly necessary. We'll have to change this once the API is usable to see what's
    ///       ergonmic.
    fn params(&self) -> Pin<&dyn Params>;

    //
    // The following functions follow the lifetime of the plugin.
    //

    /// Whether the plugin supports a bus config. This only acts as a check, and the plugin
    /// shouldn't do anything beyond returning true or false.
    fn accepts_bus_config(&self, config: &BusConfig) -> bool;

    /// Initialize the plugin for the given bus and buffer configurations.
    ///
    /// Before this point, the plugin should not have done any expensive initialization. Pleaes
    /// don't be that plugin that takes twenty seconds to scan.
    fn initialize(&mut self, bus_config: &BusConfig, buffer_config: &BufferConfig) -> bool;

    /// Process audio. To not have to worry about aliasing, the host's input buffer have already
    /// been copied to the output buffers if they are not handling buffers in place (most hosts do
    /// however).
    ///
    /// TODO: &[mut &[f32]] may not be the correct type here
    /// TODO: Provide a way to access auxiliary input channels if the IO configuration is
    ///       assymetric
    fn process(&mut self, samples: &mut &[f32]);
}

/// We only support a single main input and output bus at the moment.
pub struct BusConfig {
    /// The number of input channels for the plugin.
    pub num_input_channels: u32,
    /// The number of output channels for the plugin.
    pub num_output_channels: u32,
}

/// Configuration for (the host's) audio buffers.
pub struct BufferConfig {
    /// The current sample rate.
    pub sample_rate: f32,
    /// The maximum buffer size the host will use. The plugin should be able to accept variable
    /// sized buffers up to this size.
    pub max_buffer_size: u32,
}
