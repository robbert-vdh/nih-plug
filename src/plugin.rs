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

use crate::context::ProcessContext;
use crate::param::Params;

/// Basic functionality that needs to be implemented by a plugin. The wrappers will use this to
/// expose the plugin in a particular plugin format.
///
/// This is super basic, and lots of things I didn't need or want to use yet haven't been
/// implemented. Notable missing features include:
///
/// - MIDI
/// - Sidechain inputs
/// - Multiple output busses
/// - Special handling for offline processing
/// - Transport and other context information in the process call
/// - Sample accurate automation (this would be great, but sadly few hosts even support it so until
///   they do we'll ignore that it's a thing)
/// - Parameter hierarchies/groups
/// - Bypass parameters, right now the VST3 wrapper generates one for you
/// - Outputting parameter changes from the plugin
/// - GUIs
pub trait Plugin: Default + Send + Sync {
    const NAME: &'static str;
    const VENDOR: &'static str;
    const URL: &'static str;
    const EMAIL: &'static str;

    /// Semver compatible version string (e.g. `0.0.1`). Hosts likely won't do anything with this,
    /// but just in case they do this should only contain decimals values and dots.
    const VERSION: &'static str;

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

    /// Initialize the plugin for the given bus and buffer configurations. If the plugin is being
    /// restored from an old state, then that state will have already been restored at this point.
    /// If based on those parameters (or for any reason whatsoever) the plugin needs to introduce
    /// latency, then you can do so here using the process context.
    ///
    /// Before this point, the plugin should not have done any expensive initialization. Please
    /// don't be that plugin that takes twenty seconds to scan.
    fn initialize(
        &mut self,
        bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        context: &dyn ProcessContext,
    ) -> bool;

    /// Process audio. To not have to worry about aliasing, the host's input buffer have already
    /// been copied to the output buffers if they are not handling buffers in place (most hosts do
    /// however). All channels are also guarenteed to contain the same number of samples.
    ///
    /// TODO: Provide a way to access auxiliary input channels if the IO configuration is
    ///       assymetric
    /// TODO: Handle FTZ stuff on the wrapper side and mention that this has been handled
    /// TODO: Pass transport and other context information to the plugin
    fn process(&mut self, buffer: &mut Buffer, context: &dyn ProcessContext) -> ProcessStatus;
}

/// Provides auxiliary metadata needed for a VST3 plugin.
pub trait Vst3Plugin: Plugin {
    /// The unique class ID that identifies this particular plugin. You can use the
    /// `*b"fooofooofooofooo"` syntax for this.
    const VST3_CLASS_ID: [u8; 16];
    /// One or more categories, separated by pipe characters (`|`), up to 127 characters. Anything
    /// logner than that will be truncated. See the VST3 SDK for examples of common categories:
    /// <https://github.com/steinbergmedia/vst3_pluginterfaces/blob/2ad397ade5b51007860bedb3b01b8afd2c5f6fba/vst/ivstaudioprocessor.h#L49-L90>
    const VST3_CATEGORIES: &'static str;
}

/// We only support a single main input and output bus at the moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusConfig {
    /// The number of input channels for the plugin.
    pub num_input_channels: u32,
    /// The number of output channels for the plugin.
    pub num_output_channels: u32,
}

/// Configuration for (the host's) audio buffers.
#[derive(Debug, Clone, PartialEq)]
pub struct BufferConfig {
    /// The current sample rate.
    pub sample_rate: f32,
    /// The maximum buffer size the host will use. The plugin should be able to accept variable
    /// sized buffers up to this size.
    pub max_buffer_size: u32,
}

/// Indicates the current situation after the plugin has processed audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Something went wrong while processing audio.
    Error(&'static str),
    /// The plugin has finished processing audio. When the input is silent, the most may suspend the
    /// plugin to save resources as it sees fit.
    Normal,
    /// The plugin has a (reverb) tail with a specific length in samples.
    Tail(u32),
    /// This plugin will continue to produce sound regardless of whether or not the input is silent,
    /// and should thus not be deactivated by the host. This is essentially the same as having an
    /// infite tail.
    KeepAlive,
}

/// The audio buffers used during processing. This contains the output audio output buffers with the
/// inputs already copied to the outputs. You can either use the iterator adapters to conveniently
/// and efficiently iterate over the samples, or you can do your own thing using the raw audio
/// buffers.
pub struct Buffer<'a> {
    /// Contains slices for the plugin's outputs. You can't directly create a nested slice form
    /// apointer to pointers, so this needs to be preallocated in the setup call and kept around
    /// between process calls. And because storing a reference to this means a) that you need a lot
    /// of lifetime annotations everywhere and b) that at some point you need unsound lifetime casts
    /// because this `Buffers` either cannot have the same lifetime as the separately stored output
    /// buffers, and it also cannot be stored in a field next to it because that would mean
    /// containing mutable references to data stored in a mutex.
    output_slices: Vec<&'a mut [f32]>,
}

impl<'a> Buffer<'a> {
    /// Construct a new buffer adapter based on a set of audio buffers.
    pub fn new() -> Self {
        Self {
            output_slices: Vec::new(),
        }
    }

    /// Returns true if this buffer does not contain any samples.
    pub fn is_empty(&self) -> bool {
        self.output_slices.is_empty() || self.output_slices[0].is_empty()
    }

    /// Obtain the raw audio buffers.
    pub fn as_raw(&mut self) -> &mut [&'a mut [f32]] {
        &mut self.output_slices
    }

    /// Iterate over the samples, returning a channel iterator for each sample.
    pub fn iter_mut(&mut self) -> Samples<'_, 'a> {
        Samples {
            buffers: &mut self.output_slices,
            current_sample: 0,
        }
    }

    /// Access the raw output slice vector. This neds to be resized to match the number of output
    /// channels during the plugin's initialization. Then during audio processing, these slices
    /// should be updated to point to the plugin's audio buffers.
    ///
    /// # Safety
    ///
    /// The stored slices must point to live data when this object is passed to the plugins' process
    /// function. The rest of this object also assumes all channel lengths are equal. Panics will
    /// likely occur if this is not the case.
    pub unsafe fn as_raw_vec(&mut self) -> &mut Vec<&'a mut [f32]> {
        &mut self.output_slices
    }
}

/// An iterator over all samples in the buffer, yielding iterators over each channel for every
/// sample. This iteration order offers good cache locality for per-sample access.
pub struct Samples<'outer, 'inner> {
    /// The raw output buffers.
    pub(self) buffers: &'outer mut [&'inner mut [f32]],
    pub(self) current_sample: usize,
}

impl<'outer, 'inner> Iterator for Samples<'outer, 'inner> {
    type Item = Channels<'outer, 'inner>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_sample < self.buffers[0].len() {
            // SAFETY: We guarantee that each sample is only mutably borrowed once in the channels
            // iterator
            let buffers: &'outer mut _ = unsafe { &mut *(self.buffers as *mut _) };
            let channels = Channels {
                buffers,
                current_sample: self.current_sample,
                current_channel: 0,
            };

            self.current_sample += 1;

            Some(channels)
        } else {
            None
        }
    }
}

/// An iterator over the channel data for a sample, yielded by [Samples].
pub struct Channels<'outer, 'inner> {
    /// The raw output buffers.
    pub(self) buffers: &'outer mut [&'inner mut [f32]],
    pub(self) current_sample: usize,
    pub(self) current_channel: usize,
}

impl<'outer, 'inner> Iterator for Channels<'outer, 'inner> {
    type Item = &'inner mut f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_channel < self.buffers.len() {
            // SAFETY: These bounds have already been checked
            let sample = unsafe {
                self.buffers
                    .get_unchecked_mut(self.current_channel)
                    .get_unchecked_mut(self.current_sample)
            };
            // SAFETY: It is not possible to have multiple mutable references to the same sample at
            // the same time
            let sample: &'inner mut f32 = unsafe { &mut *(sample as *mut f32) };

            self.current_channel += 1;

            Some(sample)
        } else {
            None
        }
    }
}
