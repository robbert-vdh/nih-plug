//! Traits and structs describing plugins and editors.

use std::sync::Arc;

use crate::buffer::Buffer;
use crate::context::gui::AsyncExecutor;
use crate::context::init::InitContext;
use crate::context::process::ProcessContext;
use crate::editor::Editor;
use crate::midi::sysex::SysExMessage;
use crate::midi::MidiConfig;
use crate::params::Params;
use crate::wrapper::clap::features::ClapFeature;
use crate::wrapper::state::PluginState;
#[cfg(feature = "vst3")]
pub use crate::wrapper::vst3::subcategories::Vst3SubCategory;

/// A function that can execute a plugin's [`BackgroundTask`][Plugin::BackgroundTask]s. A plugin can
/// dispatch these tasks from the `initialize()` function, the `process()` function, or the GUI, so
/// they can be deferred for later to avoid blocking realtime contexts.
pub type TaskExecutor<P> = Box<dyn Fn(<P as Plugin>::BackgroundTask) + Send>;

/// Basic functionality that needs to be implemented by a plugin. The wrappers will use this to
/// expose the plugin in a particular plugin format.
///
/// The main thing you need to do is define a `[Params]` struct containing all of your parameters.
/// See the trait's documentation for more information on how to do that, or check out the examples.
/// Most of the other functionality is optional and comes with default trait method implementations.
///
/// Some notable not yet implemented features include:
///
/// - MIDI2 for CLAP, note expressions, polyphonic modulation and MIDI1, and MIDI SysEx are already
///   supported
/// - Audio thread thread pools (with host integration in CLAP)
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

    /// The default number of input channels. This merely serves as a default. The host will probe
    /// the plugin's supported configuration using
    /// [`accepts_bus_config()`][Self::accepts_bus_config()], and the selected configuration is
    /// passed to [`initialize()`][Self::initialize()]. Some hosts like, like Bitwig and Ardour, use
    /// the defaults instead of setting up the busses properly.
    ///
    /// Setting this to zero causes the plugin to have no main input bus.
    const DEFAULT_INPUT_CHANNELS: u32 = 2;
    /// The default number of output channels. All of the same caveats mentioned for
    /// `DEFAULT_INPUT_CHANNELS` apply here.
    ///
    /// Setting this to zero causes the plugin to have no main output bus.
    const DEFAULT_OUTPUT_CHANNELS: u32 = 2;

    /// If set, then the plugin will have this many sidechain input busses with a default number of
    /// channels. Not all hosts support more than one sidechain input bus. Negotiating the actual
    /// configuration works the same was as with `DEFAULT_INPUT_CHANNELS`.
    const DEFAULT_AUX_INPUTS: Option<AuxiliaryIOConfig> = None;
    /// If set, then the plugin will have this many auxiliary output busses with a default number of
    /// channels. Negotiating the actual configuration works the same was as with
    /// `DEFAULT_INPUT_CHANNELS`.
    const DEFAULT_AUX_OUTPUTS: Option<AuxiliaryIOConfig> = None;

    /// Optional names for the main and auxiliary input and output ports. Will be generated if not
    /// set. This is mostly useful to give descriptive names to the outputs for multi-output
    /// plugins.
    const PORT_NAMES: PortNames = PortNames {
        main_input: None,
        main_output: None,
        aux_inputs: None,
        aux_outputs: None,
    };

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
    /// byte buffers.
    type SysExMessage: SysExMessage;

    /// A type encoding the different background tasks this plugin wants to run, or `()` if it
    /// doesn't have any background tasks. This is usually set to an enum type. The task type should
    /// not contain any heap allocated data like [`Vec`]s and [`Box`]es. Tasks can be send using the
    /// methods on the various [`*Context`][crate::context] objects.
    //
    // NOTE: Sadly it's not yet possible to default this and the `async_executor()` function to
    //       `()`: https://github.com/rust-lang/rust/issues/29661
    type BackgroundTask: Send;
    /// A function that executes the plugin's tasks. Queried once when the plugin instance is
    /// created. See [`BackgroundTask`][Self::BackgroundTask].
    fn task_executor(&self) -> TaskExecutor<Self> {
        // In the default implementation we can simply ignore the value
        Box::new(|_| ())
    }

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
    fn editor(&self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
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

    /// Whether the plugin supports a bus config. This only acts as a check, and the plugin
    /// shouldn't do anything beyond returning true or false.
    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        config.num_input_channels == Self::DEFAULT_INPUT_CHANNELS
            && config.num_output_channels == Self::DEFAULT_OUTPUT_CHANNELS
    }

    /// Initialize the plugin for the given bus and buffer configurations. These configurations will
    /// not change until this function is called again, so feel free to copy these objects to your
    /// plugin's object. If the plugin is being restored from an old state, then that state will
    /// have already been restored at this point. If based on those parameters (or for any reason
    /// whatsoever) the plugin needs to introduce latency, then you can do so here using the process
    /// context. Depending on how the host restores plugin state, this function may also be called
    /// twice in rapid succession. If the plugin fails to initialize for whatever reason, then this
    /// should return `false`.
    ///
    /// Before this point, the plugin should not have done any expensive initialization. Please
    /// don't be that plugin that takes twenty seconds to scan.
    ///
    /// After this function [`reset()`][Self::reset()] will always be called. If you need to clear
    /// state, such as filters or envelopes, then you should do so in that function instead.
    fn initialize(
        &mut self,
        bus_config: &BusConfig,
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

/// Provides auxiliary metadata needed for a CLAP plugin.
pub trait ClapPlugin: Plugin {
    /// A unique ID that identifies this particular plugin. This is usually in reverse domain name
    /// notation, e.g. `com.manufacturer.plugin-name`.
    const CLAP_ID: &'static str;
    /// An optional short description for the plugin.
    const CLAP_DESCRIPTION: Option<&'static str>;
    /// The URL to the plugin's manual, if available.
    const CLAP_MANUAL_URL: Option<&'static str>;
    /// The URL to the plugin's support page, if available.
    const CLAP_SUPPORT_URL: Option<&'static str>;
    /// Keywords describing the plugin. The host may use this to classify the plugin in its plugin
    /// browser.
    const CLAP_FEATURES: &'static [ClapFeature];

    /// If set, this informs the host about the plugin's capabilities for polyphonic modulation.
    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = None;
}

/// Provides auxiliary metadata needed for a VST3 plugin.
#[cfg(feature = "vst3")]
pub trait Vst3Plugin: Plugin {
    /// The unique class ID that identifies this particular plugin. You can use the
    /// `*b"fooofooofooofooo"` syntax for this.
    ///
    /// This will be shuffled into a different byte order on Windows for project-compatibility.
    const VST3_CLASS_ID: [u8; 16];
    /// One or more subcategories. The host may use these to categorize the plugin. Internally this
    /// slice will be converted to a string where each character is separated by a pipe character
    /// (`|`). This string has a limit of 127 characters, and anything longer than that will be
    /// truncated.
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory];

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

/// The plugin's IO configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusConfig {
    /// The number of input channels for the plugin.
    pub num_input_channels: u32,
    /// The number of output channels for the plugin.
    pub num_output_channels: u32,
    /// Any additional sidechain inputs.
    pub aux_input_busses: AuxiliaryIOConfig,
    /// Any additional outputs.
    pub aux_output_busses: AuxiliaryIOConfig,
}

/// Configuration for auxiliary inputs or outputs on [`BusConfig`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AuxiliaryIOConfig {
    /// The number of auxiliary input or output busses.
    pub num_busses: u32,
    /// The number of channels in each bus.
    pub num_channels: u32,
}

/// Contains names for the main input and output ports as well as for all of the auxiliary input and
/// output ports. Setting these is optional, but it makes working with multi-output plugins much
/// more convenient.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PortNames {
    /// The name for the main input port. Will be generated if not set.
    pub main_input: Option<&'static str>,
    /// The name for the main output port. Will be generated if not set.
    pub main_output: Option<&'static str>,
    /// Names for auxiliary (sidechain) input ports. Will be generated if not set or if this slice
    /// does not contain enough names.
    pub aux_inputs: Option<&'static [&'static str]>,
    /// Names for auxiliary output ports. Will be generated if not set or if this slice does not
    /// contain enough names.
    pub aux_outputs: Option<&'static [&'static str]>,
}

/// Configuration for (the host's) audio buffers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferConfig {
    /// The current sample rate.
    pub sample_rate: f32,
    /// The minimum buffer size the host will use. This may not be set.
    pub min_buffer_size: Option<u32>,
    /// The maximum buffer size the host will use. The plugin should be able to accept variable
    /// sized buffers up to this size, or between the minimum and the maximum buffer size if both
    /// are set.
    pub max_buffer_size: u32,
    /// The current processing mode. The host will reinitialize the plugin any time this changes.
    pub process_mode: ProcessMode,
}

/// Contains auxiliary (sidechain) input and output buffers for a process call.
pub struct AuxiliaryBuffers<'a> {
    /// All auxiliary (sidechain) inputs defined for this plugin. The data in these buffers can
    /// safely be overwritten. Auxiliary inputs can be defined by setting
    /// [`Plugin::DEFAULT_AUX_INPUTS`][`crate::prelude::Plugin::DEFAULT_AUX_INPUTS`].
    pub inputs: &'a mut [Buffer<'a>],
    /// Get all auxiliary outputs defined for this plugin. Auxiliary outputs can be defined by
    /// setting [`Plugin::DEFAULT_AUX_OUTPUTS`][`crate::prelude::Plugin::DEFAULT_AUX_OUTPUTS`].
    pub outputs: &'a mut [Buffer<'a>],
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
    /// infinite tail.
    KeepAlive,
}

/// The plugin's current processing mode. Exposed through [`BufferConfig::process_mode`]. The host
/// will reinitialize the plugin whenever this changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessMode {
    /// The plugin is processing audio in real time at a fixed rate.
    Realtime,
    /// The plugin is processing audio at a real time-like pace, but at irregular intervals. The
    /// host may do this to process audio ahead of time to loosen realtime constraints and to reduce
    /// the chance of xruns happening. This is only used by VST3.
    Buffered,
    /// The plugin is rendering audio offline, potentially faster than realtime ('freewheeling').
    /// The host will continuously call the process function back to back until all audio has been
    /// processed.
    Offline,
}

/// Configuration for the plugin's polyphonic modulation options, if it supports .
pub struct PolyModulationConfig {
    /// The maximum number of voices this plugin will ever use. Call the context's
    /// `set_current_voice_capacity()` method during initialization or audio processing to set the
    /// polyphony limit.
    pub max_voice_capacity: u32,
    /// If set to `true`, then the host may send note events for the same channel and key, but using
    /// different voice IDs. Bitwig Studio, for instance, can use this to do voice stacking. After
    /// enabling this, you should always prioritize using voice IDs to map note events to voices.
    pub supports_overlapping_voices: bool,
}
