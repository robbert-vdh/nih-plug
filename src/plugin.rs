//! Traits and structs describing plugins and editors.

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use std::any::Any;
use std::sync::Arc;

use crate::buffer::Buffer;
use crate::context::{GuiContext, ProcessContext};
use crate::midi::MidiConfig;
use crate::param::internals::Params;

/// Basic functionality that needs to be implemented by a plugin. The wrappers will use this to
/// expose the plugin in a particular plugin format.
///
/// The main thing you need to do is define a `[Params]` struct containing all of your parmaeters.
/// See the trait's documentation for more information on how to do that, or check out the examples.
///
/// This is super basic, and lots of things I didn't need or want to use yet haven't been
/// implemented. Notable missing features include:
///
/// - Sidechain inputs
/// - Multiple output busses
/// - Special handling for offline processing
/// - MIDI SysEx and MIDI2 for CLAP, note expressions and MIDI1 are already supported
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

    /// Whether the plugin accepts note events, and what which events it wants to receive. If this
    /// is set to [`MidiConfig::None`], then the plugin won't receive any note events.
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    /// Whether the plugin can output note events. If this is set to [`MidiConfig::None`], then the
    /// plugin won't have a note output port. When this is set to another value, then in most hsots
    /// the plugin will consume all note and MIDI CC input. If you don't want that, then you will
    /// need to forward those events yourself.
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    /// If enabled, the audio processing cycle may be split up into multiple smaller chunks if
    /// parameter values change occur in the middle of the buffer. Depending on the host these
    /// blocks may be as small as a single sample. Bitwig Studio sends at most one parameter change
    /// every 64 samples.
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    /// The plugin's parameters. The host will update the parameter values before calling
    /// `process()`. These parameters are identified by strings that should never change when the
    /// plugin receives an update.
    fn params(&self) -> Arc<dyn Params>;

    /// The plugin's editor, if it has one. The actual editor instance is created in
    /// [`Editor::spawn()`]. A plugin editor likely wants to interact with the plugin's parameters
    /// and other shared data, so you'll need to move [`Arc`] pointing to any data you want to
    /// access into the editor. You can later modify the parameters through the
    /// [`GuiContext`][crate::prelude::GuiContext] and [`ParamSetter`][crate::prelude::ParamSetter] after the editor
    /// GUI has been created.
    fn editor(&self) -> Option<Box<dyn Editor>> {
        None
    }

    //
    // The following functions follow the lifetime of the plugin.
    //

    /// Whether the plugin supports a bus config. This only acts as a check, and the plugin
    /// shouldn't do anything beyond returning true or false.
    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        config.num_input_channels == Self::DEFAULT_NUM_INPUTS
            && config.num_output_channels == Self::DEFAULT_NUM_OUTPUTS
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
    ///
    /// After this function [`reset()`][Self::reset()] will always be called. If you need to clear
    /// state, such as filters or envelopes, then you should do so in that function inistead.
    fn initialize(
        &mut self,
        bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        context: &mut impl ProcessContext,
    ) -> bool {
        true
    }

    /// Clear internal state such as filters and envelopes. This is always called after
    /// [`initialize()`][Self::initialize(0)], and it may also be called at any other time from the
    /// audio thread. You should thus not do any allocations in this function.
    fn reset(&mut self) {}

    /// Process audio. The host's input buffers have already been copied to the output buffers if
    /// they are not processing audio in place (most hosts do however). All channels are also
    /// guarenteed to contain the same number of samples. Lastly, denormals have already been taken
    /// case of by NIH-plug, and you can optionally enable the `assert_process_allocs` feature to
    /// abort the program when any allocation accurs in the process function while running in debug
    /// mode.
    ///
    /// The framework provides convenient iterators on the [`Buffer`] object to process audio either
    /// either per-sample per-channel, or per-block per-channel per-sample. The first approach is
    /// preferred for plugins that don't require block-based processing because of their use of
    /// per-sample SIMD or excessive branching. The parameter smoothers can also work in both modes:
    /// use [`Smoother::next()`][crate::prelude::Smoother::next()] for per-sample processing, and
    /// [`Smoother::next_block()`][crate::prelude::Smoother::next_block()] for block-based
    /// processing. In order to use block-based smoothing, you will need to call
    /// [`initialize_block_smoothers()`][Self::initialize_block_smoothers()] in your
    /// [`initialize()`][Self::initialize()] function first to reserve enough capacity in the
    /// smoothers.
    ///
    /// TODO: Provide a way to access auxiliary input channels if the IO configuration is
    ///       assymetric
    /// TODO: Pass transport and other context information to the plugin
    /// TODO: Create an example plugin that uses block-based processing
    fn process(&mut self, buffer: &mut Buffer, context: &mut impl ProcessContext) -> ProcessStatus;

    /// Convenience function provided to allocate memory for block-based smoothing for this plugin.
    /// Since this allocates memory, this should be called in [`initialize()`][Self::initialize()].
    /// If you are going to use [`Buffer::iter_blocks()`] and want to use parameter smoothing in
    /// those blocks, then call this function with the same maximum block size first before calling
    /// [`Smoother::next_block()`][crate::prelude::Smoother::next_block()].
    fn initialize_block_smoothers(&mut self, max_block_size: usize) {
        for (_, mut param, _) in self.params().param_map() {
            unsafe { param.initialize_block_smoother(max_block_size) };
        }
    }
}

/// Provides auxiliary metadata needed for a CLAP plugin.
pub trait ClapPlugin: Plugin {
    /// A unique ID that identifies this particular plugin. This is usually in reverse domain name
    /// notation, e.g. `com.manufacturer.plugin-name`.
    const CLAP_ID: &'static str;
    /// A short description for the plugin.
    const CLAP_DESCRIPTION: &'static str;
    /// Arbitrary keywords describing the plugin. See the CLAP specification for examples:
    /// <https://github.com/free-audio/clap/blob/main/include/clap/plugin.h>.
    ///
    /// On windows `win32-dpi-aware` is automatically added.
    const CLAP_FEATURES: &'static [&'static str];
    /// A URL to the plugin's manual, CLAP does not specify what to do when there is none.
    //
    // TODO: CLAP does not specify this, can these manual fields be null pointers?
    const CLAP_MANUAL_URL: &'static str;
    /// A URL to the plugin's support page, CLAP does not specify what to do when there is none.
    const CLAP_SUPPORT_URL: &'static str;
}

/// Provides auxiliary metadata needed for a VST3 plugin.
pub trait Vst3Plugin: Plugin {
    /// The unique class ID that identifies this particular plugin. You can use the
    /// `*b"fooofooofooofooo"` syntax for this.
    ///
    /// This will be shuffled into a different byte order on Windows for project-compatibility.
    const VST3_CLASS_ID: [u8; 16];
    /// One or more categories, separated by pipe characters (`|), up to 127 characters. Anything
    /// logner than that will be truncated. See the VST3 SDK for examples of common categories:
    /// <https://github.com/steinbergmedia/vst3_pluginterfaces/blob/2ad397ade5b51007860bedb3b01b8afd2c5f6fba/vst/ivstaudioprocessor.h#L49-L90>
    const VST3_CATEGORIES: &'static str;

    /// [`VST3_CLASS_ID`][Self::VST3_CLASS_ID`] in the correct order for the current platform so
    /// projects and presets can be shared between platforms. This should not be overridden.
    const PLATFORM_VST3_CLASS_ID: [u8; 16] = swap_vst3_uid_byte_order(Self::VST3_CLASS_ID);
}

#[cfg(not(target_os = "windows"))]
const fn swap_vst3_uid_byte_order(uid: [u8; 16]) -> [u8; 16] {
    uid
}

#[cfg(target_os = "windows")]
const fn swap_vst3_uid_byte_order(mut uid: [u8; 16]) -> [u8; 16] {
    // No mutable references in const functions, so we can't use `uid.swap()`
    let original_uid = uid;

    uid[0] = original_uid[3];
    uid[1] = original_uid[2];
    uid[2] = original_uid[1];
    uid[3] = original_uid[0];

    uid[4] = original_uid[5];
    uid[5] = original_uid[4];
    uid[6] = original_uid[7];
    uid[7] = original_uid[6];

    uid
}

/// An editor for a [`Plugin`].
pub trait Editor: Send + Sync {
    /// Create an instance of the plugin's editor and embed it in the parent window. As explained in
    /// [`Plugin::editor()`], you can then read the parameter values directly from your [`Params`]
    /// object, and modifying the values can be done using the functions on the
    /// [`ParamSetter`][crate::prelude::ParamSetter]. When you change a parameter value that way it will be
    /// broadcasted to the host and also updated in your [`Params`] struct.
    ///
    /// This function should return a handle to the editor, which will be dropped when the editor
    /// gets closed. Implement the [`Drop`] trait on the returned handle if you need to explicitly
    /// handle the editor's closing behavior.
    ///
    /// If [`set_scale_factor()`][Self::set_scale_factor()] has been called, then any created
    /// windows should have their sizes multiplied by that factor.
    ///
    /// The wrapper guarantees that a previous handle has been dropped before this function is
    /// called again.
    //
    // TODO: Think of how this would work with the event loop. On Linux the wrapper must provide a
    //       timer using VST3's `IRunLoop` interface, but on Window and macOS the window would
    //       normally register its own timer. Right now we just ignore this because it would
    //       otherwise be basically impossible to have this still be GUI-framework agnostic. Any
    //       callback that deos involve actual GUI operations will still be spooled to the IRunLoop
    //       instance.
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send + Sync>;

    /// Return the (currnent) size of the editor in pixels as a `(width, height)` pair. This size
    /// must be reported in _logical pixels_, i.e. the size before being multiplied by the DPI
    /// scaling factor to get the actual physical screen pixels.
    fn size(&self) -> (u32, u32);

    /// Set the DPI scaling factor, if supported. The plugin APIs don't make any guarantees on when
    /// this is called, but for now just assume it will be the first function that gets called
    /// before creating the editor. If this is set, then any windows created by this editor should
    /// have their sizes multiplied by this scaling factor on Windows and Linux.
    ///
    /// Right now this is never called on macOS since DPI scaling is built into the operating system
    /// there.
    fn set_scale_factor(&self, factor: f32) -> bool;

    /// A callback that will be called wheneer the parameter values changed while the editor is
    /// open. You don't need to do anything with this, but this can be used to force a redraw when
    /// the host sends a new value for a parameter or when a parameter change sent to the host gets
    /// processed.
    ///
    /// This function will be called from the **audio thread**. It must thus be lock-free and may
    /// not allocate.
    fn param_values_changed(&self);

    // TODO: Reconsider adding a tick function here for the Linux `IRunLoop`. To keep this platform
    //       and API agnostic, add a way to ask the GuiContext if the wrapper already provides a
    //       tick function. If it does not, then the Editor implementation must handle this by
    //       itself. This would also need an associated `PREFERRED_FRAME_RATE` constant.
    // TODO: Host->Plugin resizing
}

/// A raw window handle for platform and GUI framework agnostic editors.
pub struct ParentWindowHandle {
    pub handle: RawWindowHandle,
}

unsafe impl HasRawWindowHandle for ParentWindowHandle {
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
