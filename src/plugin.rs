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

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use std::pin::Pin;
use std::sync::Arc;

use crate::buffer::Buffer;
use crate::context::{GuiContext, ProcessContext};
use crate::param::internals::Params;

/// Basic functionality that needs to be implemented by a plugin. The wrappers will use this to
/// expose the plugin in a particular plugin format.
///
/// This is super basic, and lots of things I didn't need or want to use yet haven't been
/// implemented. Notable missing features include:
///
/// - Sidechain inputs
/// - Multiple output busses
/// - Special handling for offline processing
/// - Transport and other context information in the process call
/// - Sample accurate automation (this would be great, but sadly few hosts even support it so until
///   they do we'll ignore that it's a thing)
/// - Parameter hierarchies/groups
/// - Bypass parameters, right now the VST3 wrapper generates one for you
/// - Outputting parameter changes from the plugin
/// - MIDI CC handling
/// - Outputting MIDI events
#[allow(unused_variables)]
pub trait Plugin: Default + Send + Sync + 'static {
    const NAME: &'static str;
    const VENDOR: &'static str;
    const URL: &'static str;
    const EMAIL: &'static str;

    /// Semver compatible version string (e.g. `0.0.1`). Hosts likely won't do anything with this,
    /// but just in case they do this should only contain decimals values and dots.
    const VERSION: &'static str;

    /// The default number of inputs. Some hosts like, like Bitwig and Ardour, use the defaults
    /// instead of setting up the busses properly.
    const DEFAULT_NUM_INPUTS: u32 = 2;
    /// The default number of inputs. Some hosts like, like Bitwig and Ardour, use the defaults
    /// instead of setting up the busses properly.
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    /// Whether the plugin accepts note events. If this is set to `false`, then the plugin won't
    /// receive any note events.
    const ACCEPTS_MIDI: bool = false;

    /// The plugin's parameters. The host will update the parameter values before calling
    /// `process()`. These parameters are identified by strings that should never change when the
    /// plugin receives an update.
    fn params(&self) -> Pin<&dyn Params>;

    /// Create an editor for this plugin and embed it in the parent window. A plugin editor will
    /// likely want to interact with the plugin's parameters, so the idea is that you take a
    /// reference to your [Params] object in your editor as well as the [GuiContext] that's passed
    /// to this function. You can then read the parameter values directly from your [Params] object,
    /// and modifying the values can be done using the functions on [GuiContext::setter()]. When you
    /// change a parameter value that way it will be broadcasted to the host and also updated in
    /// your [Params] struct.
    //
    // TODO: Think of how this would work with the event loop. On Linux the wrapper must provide a
    //       timer using VST3's `IRunLoop` interface, but on Window and macOS the window would
    //       normally register its own timer. Right now we just ignore this because it would
    //       otherwise be basically impossible to have this still be GUI-framework agnostic. Any
    //       callback that deos involve actual GUI operations will still be spooled to the IRunLoop
    //       instance.
    fn create_editor(
        &self,
        parent: EditorWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Option<Box<dyn Editor>> {
        None
    }

    /// Return the current size of the plugin's editor, if it has one. This is also used to check
    /// whether the plugin has an editor without creating one.
    fn editor_size(&self) -> Option<(u32, u32)> {
        None
    }

    //
    // The following functions follow the lifetime of the plugin.
    //

    /// Whether the plugin supports a bus config. This only acts as a check, and the plugin
    /// shouldn't do anything beyond returning true or false.
    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        config.num_input_channels == 2 && config.num_output_channels == 2
    }

    /// Initialize the plugin for the given bus and buffer configurations. If the plugin is being
    /// restored from an old state, then that state will have already been restored at this point.
    /// If based on those parameters (or for any reason whatsoever) the plugin needs to introduce
    /// latency, then you can do so here using the process context. Depending on how the host
    /// restores plugin state, this function may also be called twice in rapid succession. If the
    /// plugin fails to inialize for whatever reason, then this should return `false`.
    ///
    /// Before this point, the plugin should not have done any expensive initialization. Please
    /// don't be that plugin that takes twenty seconds to scan.
    fn initialize(
        &mut self,
        bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        context: &mut impl ProcessContext,
    ) -> bool {
        true
    }

    /// Process audio. The host's input buffers have already been copied to the output buffers if
    /// they are not processing audio in place (most hosts do however). All channels are also
    /// guarenteed to contain the same number of samples. Lastly, denormals have already been taken
    /// case of by NIH-plug, and you can optionally enable the `assert_process_allocs` feature to
    /// abort the program when any allocation accurs in the process function while running in debug
    /// mode.
    ///
    /// TODO: Provide a way to access auxiliary input channels if the IO configuration is
    ///       assymetric
    /// TODO: Pass transport and other context information to the plugin
    fn process(&mut self, buffer: &mut Buffer, context: &mut impl ProcessContext) -> ProcessStatus;
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

/// An editor for a [Plugin]. This object gets dropped when the host closes the editor.
pub trait Editor {
    /// Return the (currnent) size of the editor in pixels as a `(width, height)` pair.
    fn size(&self) -> (u32, u32);

    // TODO: Reconsider adding a tick function here for the Linux `IRunLoop`. To keep this platform
    //       and API agnostic, add a way to ask the GuiContext if the wrapper already provides a
    //       tick function. If it does not, then the Editor implementation must handle this by
    //       itself. This would also need an associated `PREFERRED_FRAME_RATE` constant.
    // TODO: Add the things needed for DPI scaling
    // TODO: Resizing
}

/// A raw window handle for platform and GUI framework agnostic editors.
pub struct EditorWindowHandle {
    pub handle: RawWindowHandle,
}

unsafe impl HasRawWindowHandle for EditorWindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        self.handle
    }
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
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// Event for (incoming) notes. Right now this only supports a very small subset of the MIDI
/// specification. See the util module for convenient conversion functions.
///
/// All of the timings are sample offsets withing the current buffer.
///
/// TODO: Add more events as needed
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NoteEvent {
    NoteOn {
        timing: u32,
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        timing: u32,
        channel: u8,
        note: u8,
        velocity: u8,
    },
}

impl NoteEvent {
    /// Return the sample within the current buffer this event belongs to.
    pub fn timing(&self) -> u32 {
        match &self {
            NoteEvent::NoteOn { timing, .. } => *timing,
            NoteEvent::NoteOff { timing, .. } => *timing,
        }
    }
}
