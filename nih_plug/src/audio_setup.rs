//! Types and definitions surrounding a plugin's audio IO setup.

use std::num::NonZeroU32;

use crate::prelude::Buffer;

/// A description of a plugin's audio IO configuration. The [`Plugin`][crate::prelude::Plugin]
/// defines a list of supported audio IO configs, with the first one acting as the default layout.
/// Depending on the plugin API, the host may pick a different configuration from the list and use
/// that instead. The final chosen configuration is passed as an argument to the
/// [`Plugin::initialize()`][crate::prelude::Plugin::initialize] function so the plugin can allocate
/// its data structures based on the number of audio channels it needs to process.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioIOLayout {
    /// The number of main input channels for the plugin, if it has a main input port. This can be
    /// set to `None` if the plugin does not have one.
    pub main_input_channels: Option<NonZeroU32>,
    /// The number of main output channels for the plugin, if it has a main output port. This can be
    /// set to `None` if the plugin does not have one.
    pub main_output_channels: Option<NonZeroU32>,
    /// The plugin's additional sidechain inputs, if it has any. Use the [`new_nonzero_u32()`]
    /// function to construct these values until const `Option::unwrap()` gets stabilized
    /// (<https://github.com/rust-lang/rust/issues/67441>).
    pub aux_input_ports: &'static [NonZeroU32],
    /// The plugin's additional outputs, if it has any. Use the [`new_nonzero_u32()`] function to
    /// construct these values until const `Option::unwrap()` gets stabilized
    /// (<https://github.com/rust-lang/rust/issues/67441>).
    pub aux_output_ports: &'static [NonZeroU32],

    /// Optional names for the audio ports. Defining these can be useful for plugins with multiple
    /// output and input ports.
    pub names: PortNames,
}

/// Construct a `NonZeroU32` value at compile time. Equivalent to `NonZeroU32::new(n).unwrap()`.
pub const fn new_nonzero_u32(n: u32) -> NonZeroU32 {
    match NonZeroU32::new(n) {
        Some(n) => n,
        None => panic!("'new_nonzero_u32()' called with a zero value"),
    }
}

/// Contains auxiliary (sidechain) input and output buffers for a process call.
pub struct AuxiliaryBuffers<'a> {
    /// Buffers for all auxiliary (sidechain) inputs defined for this plugin. The data in these
    /// buffers can safely be overwritten. Auxiliary inputs can be defined using the
    /// [`AudioIOLayout::aux_input_ports`] field.
    pub inputs: &'a mut [Buffer<'a>],
    /// Buffers for all auxiliary outputs defined for this plugin. Auxiliary outputs can be defined using the
    /// [`AudioIOLayout::aux_output_ports`] field.
    pub outputs: &'a mut [Buffer<'a>],
}

/// Contains names for the ports defined in an `AudioIOLayout`. Setting these is optional, but it
/// makes working with multi-output plugins much more convenient.
///
/// All of these names should start with a capital letter to be consistent with automatically
/// generated names.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortNames {
    /// The name for the audio IO layout as a whole. Useful when a plugin has multiple distinct
    /// layouts. Will be generated if not set.
    pub layout: Option<&'static str>,

    /// The name for the main input port. Will be generated if not set.
    pub main_input: Option<&'static str>,
    /// The name for the main output port. Will be generated if not set.
    pub main_output: Option<&'static str>,
    /// Names for auxiliary (sidechain) input ports. Will be generated if not set or if this slice
    /// does not contain enough names.
    pub aux_inputs: &'static [&'static str],
    /// Names for auxiliary output ports. Will be generated if not set or if this slice does not
    /// contain enough names.
    pub aux_outputs: &'static [&'static str],
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

impl AudioIOLayout {
    /// [`AudioIOLayout::default()`], but as a const function. Used when initializing
    /// `Plugin::AUDIO_IO_LAYOUTS`. (<https://github.com/rust-lang/rust/issues/67792>)
    pub const fn const_default() -> Self {
        Self {
            main_input_channels: None,
            main_output_channels: None,
            aux_input_ports: &[],
            aux_output_ports: &[],
            names: PortNames::const_default(),
        }
    }

    /// A descriptive name for the layout. This is taken from `PortNames::layout` if set. Otherwise
    /// it is generated based on the layout.
    pub fn name(&self) -> String {
        if let Some(name) = self.names.layout {
            return name.to_owned();
        }

        // If the name is not set then we'll try to come up with something descriptive
        match (
            self.main_input_channels
                .map(NonZeroU32::get)
                .unwrap_or_default(),
            self.main_output_channels
                .map(NonZeroU32::get)
                .unwrap_or_default(),
            self.aux_input_ports.len(),
            self.aux_output_ports.len(),
        ) {
            (0, 0, 0, 0) => String::from("Empty"),
            (_, 1, 0, _) | (1, 0, _, _) => String::from("Mono"),
            (_, 2, 0, _) | (2, 0, _, _) => String::from("Stereo"),
            (_, 1, _, _) => String::from("Mono with sidechain"),
            (_, 2, _, _) => String::from("Stereo with sidechain"),
            // These probably, hopefully won't occur
            (i, o, 0, 0) => format!("{i} inputs, {o} outputs"),
            (i, o, _, 0) => format!("{i} inputs, {o} outputs, with sidechain"),
            // And these don't make much sense, suggestions for something better are welcome
            (i, o, 0, aux_o) => format!("{i} inputs, {o}*{} outputs", aux_o + 1),
            (i, o, aux_i, aux_o) => format!("{i}*{} inputs, {o}*{} outputs", aux_i + 1, aux_o + 1),
        }
    }

    /// The name for the main input port. Either generated or taken from the `names` field.
    pub fn main_input_name(&self) -> String {
        self.names.main_input.unwrap_or("Input").to_owned()
    }

    /// The name for the main output port. Either generated or taken from the `names` field.
    pub fn main_output_name(&self) -> String {
        self.names.main_input.unwrap_or("Output").to_owned()
    }

    /// The name for the auxiliary input port with the given index. Either generated or taken from
    /// the `names` field.
    pub fn aux_input_name(&self, idx: usize) -> Option<String> {
        if idx >= self.aux_input_ports.len() {
            None
        } else {
            match self.names.aux_inputs.get(idx) {
                Some(name) => Some(String::from(*name)),
                None if self.aux_input_ports.len() == 1 => Some(String::from("Sidechain Input")),
                None => Some(format!("Sidechain Input {}", idx + 1)),
            }
        }
    }

    /// The name for the auxiliary output port with the given index. Either generated or taken from
    /// the `names` field.
    pub fn aux_output_name(&self, idx: usize) -> Option<String> {
        if idx >= self.aux_output_ports.len() {
            None
        } else {
            match self.names.aux_outputs.get(idx) {
                Some(name) => Some(String::from(*name)),
                None if self.aux_output_ports.len() == 1 => Some(String::from("Auxiliary Output")),
                None => Some(format!("Auxiliary Output {}", idx + 1)),
            }
        }
    }
}

impl PortNames {
    /// [`PortNames::default()`], but as a const function. Used when initializing
    /// `Plugin::AUDIO_IO_LAYOUTS`. (<https://github.com/rust-lang/rust/issues/67792>)
    pub const fn const_default() -> Self {
        Self {
            layout: None,
            main_input: None,
            main_output: None,
            aux_inputs: &[],
            aux_outputs: &[],
        }
    }
}
