//! Traits and structs describing plugins and editors. This includes extension structs for features
//! that are specific to one or more plugin-APIs.

use std::sync::Arc;

use crate::prelude::{
    AsyncExecutor, AudioIOLayout, AuxiliaryBuffers, Buffer, BufferConfig, Editor, InitContext,
    MidiConfig, Params, PluginState, ProcessContext, SysExMessage,
};

pub mod clap;
#[cfg(feature = "vst3")]
pub mod vst3;

/// A function that can execute a plugin's [`BackgroundTask`][Plugin::BackgroundTask]s. A plugin can
/// dispatch these tasks from the `initialize()` function, the `process()` function, or the GUI, so
/// they can be deferred for later to avoid blocking realtime contexts.
pub type TaskExecutor<P> = Box<dyn Fn(<P as Plugin>::BackgroundTask) + Send>;

/// The main plugin trait covering functionality common across most plugin formats. Most formats
/// also have another trait with more specific data and functionality that needs to be implemented
/// before the plugin can be exported to that format. The wrappers will use this to expose the
/// plugin in a particular plugin format.
///
/// NIH-plug is semi-declarative, meaning that most information about a plugin is defined
/// declaratively but it also doesn't shy away from maintaining state when that is the path of least
/// resistance. As such, the definitions on this trait fall in one of the following classes:
///
/// - `Plugin` objects are stateful. During their lifetime the plugin API wrappers will call the
///   various lifecycle methods defined below, with the `initialize()`, `reset()`, and `process()`
///   functions being the most important ones.
/// - Most of the rest of the trait statically describes the plugin. You will find this done in
///   three different ways:
///   - Most of this data, including the supported audio IO layouts, is simple enough that it can be
///     defined through compile-time constants.
///   - Some of the data is queried through a method as doing everything at compile time would
///     impose a lot of restrictions on code structure and meta programming without any real
///     benefits. In those cases the trait defines a method that is queried once and only once,
///     immediately after instantiating the `Plugin` through `Plugin::default()`. Examples of these
///     methods are [`Plugin::params()`], and
///     [`ClapPlugin::remote_controls()`][clap::ClapPlugin::remote_controls()].
///   - Some of the data is defined through associated types. Rust currently sadly does not support
///     default values for associated types, but all of these types can be set to `()` if you wish
///     to ignore them. Examples of these types are [`Plugin::SysExMessage`] and
///     [`Plugin::BackgroundTask`].
/// - Finally, there are some functions that return extension structs and handlers, similar to how
///   the `params()` function returns a data structure describing the plugin's parameters. Examples
///   of these are the [`Plugin::editor()`] and [`Plugin::task_executor()`] functions, and they're
///   also called once and only once after the plugin object has been created. This allows the audio
///   thread to have exclusive access to the `Plugin` object, and it makes it easier to compose
///   these extension structs since they're more loosely coupled to a specific `Plugin`
///   implementation.
///
/// The main thing you need to do is define a `[Params]` struct containing all of your parameters.
/// See the trait's documentation for more information on how to do that, or check out the examples.
/// The plugin also needs a `Default` implementation so it can be initialized. Most of the other
/// functionality is optional and comes with default trait method implementations.
#[allow(unused_variables)]
pub trait Plugin: Default + Send + 'static {
    /// The plugin's name.
    const NAME: &'static str;
    /// The name of the plugin's vendor.
    const VENDOR: &'static str;
    /// A URL pointing to the plugin's web page.
    const URL: &'static str;
    /// The vendor's email address.
    const EMAIL: &'static str;

    /// Semver compatible version string (e.g. `0.0.1`). Hosts likely won't do anything with this,
    /// but just in case they do this should only contain decimals values and dots.
    const VERSION: &'static str;

    /// The plugin's supported audio IO layouts. The first config will be used as the default config
    /// if the host doesn't or can't select an alternative configuration. Because of that it's
    /// recommended to begin this slice with a stereo layout. For maximum compatibility with the
    /// different plugin formats this default layout should also include all of the plugin's
    /// auxiliary input and output ports, if the plugin has any. If the slice is empty, then the
    /// plugin will not have any audio IO.
    ///
    /// Both [`AudioIOLayout`] and [`PortNames`][crate::prelude::PortNames] have `.const_default()`
    /// functions for compile-time equivalents to `Default::default()`:
    ///
    /// ```
    /// # use nih_plug::prelude::*;
    /// const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
    ///     main_input_channels: NonZeroU32::new(2),
    ///     main_output_channels: NonZeroU32::new(2),
    ///
    ///     aux_input_ports: &[new_nonzero_u32(2)],
    ///
    ///     ..AudioIOLayout::const_default()
    /// }];
    /// ```
    ///
    /// # Note
    ///
    /// Some plugin hosts, like Ableton Live, don't support MIDI-only plugins and may refuse to load
    /// plugins with no main output or with zero main output channels.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout];

    /// Whether the plugin accepts note events, and what which events it wants to receive. If this
    /// is set to [`MidiConfig::None`], then the plugin won't receive any note events.
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    /// Whether the plugin can output note events. If this is set to [`MidiConfig::None`], then the
    /// plugin won't have a note output port. When this is set to another value, then in most hosts
    /// the plugin will consume all note and MIDI CC input. If you don't want that, then you will
    /// need to forward those events yourself.
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    /// If enabled, the audio processing cycle may be split up into multiple smaller chunks if
    /// parameter values change occur in the middle of the buffer. Depending on the host these
    /// blocks may be as small as a single sample. Bitwig Studio sends at most one parameter change
    /// every 64 samples.
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    /// If this is set to true, then the plugin will report itself as having a hard realtime
    /// processing requirement when the host asks for it. Supported hosts will never ask the plugin
    /// to do offline processing.
    const HARD_REALTIME_ONLY: bool = false;

    /// The plugin's SysEx message type if it supports sending or receiving MIDI SysEx messages, or
    /// `()` if it does not. This type can be a struct or enum wrapping around one or more message
    /// types, and the [`SysExMessage`] trait is then used to convert between this type and basic
    /// byte buffers. The [`MIDI_INPUT`][Self::MIDI_INPUT] and [`MIDI_OUTPUT`][Self::MIDI_OUTPUT]
    /// fields need to be set to [`MidiConfig::Basic`] or above to be able to send and receive
    /// SysEx.
    type SysExMessage: SysExMessage;

    /// A type encoding the different background tasks this plugin wants to run, or `()` if it
    /// doesn't have any background tasks. This is usually set to an enum type. The task type should
    /// not contain any heap allocated data like [`Vec`]s and [`Box`]es. Tasks can be send using the
    /// methods on the various [`*Context`][crate::context] objects.
    //
    // NOTE: Sadly it's not yet possible to default this and the `async_executor()` function to
    //       `()`: https://github.com/rust-lang/rust/issues/29661
    type BackgroundTask: Send;
    /// A function that executes the plugin's tasks. When implementing this you will likely want to
    /// pattern match on the task type, and then send any resulting data back over a channel or
    /// triple buffer. See [`BackgroundTask`][Self::BackgroundTask].
    ///
    /// Queried only once immediately after the plugin instance is created. This function takes
    /// `&mut self` to make it easier to move data into the closure.
    fn task_executor(&mut self) -> TaskExecutor<Self> {
        // In the default implementation we can simply ignore the value
        Box::new(|_| ())
    }

    /// The plugin's parameters. The host will update the parameter values before calling
    /// `process()`. These string parameter IDs parameters should never change as they are used to
    /// distinguish between parameters.
    ///
    /// Queried only once immediately after the plugin instance is created.
    fn params(&self) -> Arc<dyn Params>;

    /// Returns an extension struct for interacting with the plugin's editor, if it has one. Later
    /// the host may call [`Editor::spawn()`] to create an editor instance. To read the current
    /// parameter values, you will need to clone and move the `Arc` containing your `Params` object
    /// into the editor. You can later modify the parameters through the
    /// [`GuiContext`][crate::prelude::GuiContext] and [`ParamSetter`][crate::prelude::ParamSetter]
    /// after the editor GUI has been created. NIH-plug comes with wrappers for several common GUI
    /// frameworks that may have their own ways of interacting with parameters. See the repo's
    /// readme for more information.
    ///
    /// Queried only once immediately after the plugin instance is created. This function takes
    /// `&mut self` to make it easier to move data into the `Editor` implementation.
    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        None
    }

    /// This function is always called just before a [`PluginState`] is loaded. This lets you
    /// directly modify old plugin state to perform migrations based on the [`PluginState::version`]
    /// field. Some examples of use cases for this are renaming parameter indices, remapping
    /// parameter values, and preserving old preset compatibility when introducing new parameters
    /// with default values that would otherwise change the sound of a preset. Keep in mind that
    /// automation may still be broken in the first two use cases.
    ///
    /// # Note
    ///
    /// This is an advanced feature that the vast majority of plugins won't need to implement.
    fn filter_state(state: &mut PluginState) {}

    //
    // The following functions follow the lifetime of the plugin.
    //

    /// Initialize the plugin for the given audio IO configuration. From this point onwards the
    /// audio IO layouts and the buffer sizes are fixed until this function is called again.
    ///
    /// Before this point, the plugin should not have done any expensive initialization. Please
    /// don't be that plugin that takes twenty seconds to scan.
    ///
    /// After this function [`reset()`][Self::reset()] will always be called. If you need to clear
    /// state, such as filters or envelopes, then you should do so in that function instead.
    ///
    /// - If you need to access this information in your process function, then you can copy the
    ///   values to your plugin instance's object.
    /// - If the plugin is being restored from an old state,
    ///   then that state will have already been restored at this point.
    /// - If based on those parameters (or for any reason whatsoever) the plugin needs to introduce
    ///   latency, then you can do so here using the process context.
    /// - Depending on how the host restores plugin state, this function may be called multiple
    ///   times in rapid succession. It may thus be useful to check if the initialization work for
    ///   the current bufffer and audio IO configurations has already been performed first.
    /// - If the plugin fails to initialize for whatever reason, then this should return `false`.
    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        true
    }

    /// Clear internal state such as filters and envelopes. This is always called after
    /// [`initialize()`][Self::initialize()], and it may also be called at any other time from the
    /// audio thread. You should thus not do any allocations in this function.
    fn reset(&mut self) {}

    /// Process audio. The host's input buffers have already been copied to the output buffers if
    /// they are not processing audio in place (most hosts do however). All channels are also
    /// guaranteed to contain the same number of samples. Lastly, denormals have already been taken
    /// case of by NIH-plug, and you can optionally enable the `assert_process_allocs` feature to
    /// abort the program when any allocation occurs in the process function while running in debug
    /// mode.
    ///
    /// The framework provides convenient iterators on the [`Buffer`] object to process audio either
    /// either per-sample per-channel, or per-block per-channel per-sample. The first approach is
    /// preferred for plugins that don't require block-based processing because of their use of
    /// per-sample SIMD or excessive branching. The parameter smoothers can also work in both modes:
    /// use [`Smoother::next()`][crate::prelude::Smoother::next()] for per-sample processing, and
    /// [`Smoother::next_block()`][crate::prelude::Smoother::next_block()] for block-based
    /// processing.
    ///
    /// The `context` object contains context information as well as callbacks for working with note
    /// events. The [`AuxiliaryBuffers`] contain the plugin's sidechain input buffers and
    /// auxiliary output buffers if it has any.
    ///
    /// TODO: Provide a way to access auxiliary input channels if the IO configuration is
    ///       asymmetric
    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus;

    /// Called when the plugin is deactivated. The host will call
    /// [`initialize()`][Self::initialize()] again before the plugin resumes processing audio. These
    /// two functions will not be called when the host only temporarily stops processing audio. You
    /// can clean up or deallocate resources here. In most cases you can safely ignore this.
    ///
    /// There is no one-to-one relationship between calls to `initialize()` and `deactivate()`.
    /// `initialize()` may be called more than once before `deactivate()` is called, for instance
    /// when restoring state while the plugin is still activate.
    fn deactivate(&mut self) {}
}

/// Indicates the current situation after the plugin has processed audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Something went wrong while processing audio.
    Error(&'static str),
    /// The plugin has finished processing audio. When the input is silent, the host may suspend the
    /// plugin to save resources as it sees fit.
    Normal,
    /// The plugin has a (reverb) tail with a specific length in samples.
    Tail(u32),
    /// This plugin will continue to produce sound regardless of whether or not the input is silent,
    /// and should thus not be deactivated by the host. This is essentially the same as having an
    /// infinite tail.
    KeepAlive,
}
