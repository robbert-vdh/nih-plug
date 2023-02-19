//! Types and definitions surrounding a plugin's audio bus setup.

use crate::buffer::Buffer;

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
