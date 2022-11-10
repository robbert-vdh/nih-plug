use clap::{Parser, ValueEnum};

/// Configuration for a standalone plugin that would normally be provided by the DAW.
#[derive(Debug, Clone, Parser)]
#[clap(about = None, long_about = None)]
pub struct WrapperConfig {
    /// The audio and MIDI backend to use.
    ///
    /// The 'auto' option will try all backends in order, and falls back to the dummy backend with
    /// no audio input or output if the other backends are not available.
    #[clap(value_parser, short = 'b', long, default_value = "auto")]
    pub backend: BackendType,
    /// The input device for the ALSA, CoreAudio, and WASAPI backends. No input will be connected if
    /// this is not specified.
    ///
    /// Specifying an empty string or other invalid value will list all available input devices.
    #[clap(value_parser, long)]
    pub input_device: Option<String>,
    /// The output device for the ALSA, CoreAudio, and WASAPI backends.
    ///
    /// Specifying an empty string or other invalid value will list all available output devices.
    #[clap(value_parser, long)]
    pub output_device: Option<String>,

    // These will default to the plugin's default input and output channel count. We could set the
    // default value here to match those, but that would require a custom Args+FromArgMatches
    // implementation and access to the `Plugin` type.
    /// The number of input channels.
    #[clap(value_parser, short = 'i', long)]
    pub input_channels: Option<u32>,
    /// The number of output channels.
    #[clap(value_parser, short = 'o', long)]
    pub output_channels: Option<u32>,
    /// The audio backend's sample rate.
    ///
    /// This setting is ignored when using the JACK backend.
    #[clap(value_parser, short = 'r', long, default_value = "48000")]
    pub sample_rate: f32,
    /// The audio backend's period size.
    ///
    /// This setting is ignored when using the JACK backend.
    #[clap(value_parser, short = 'p', long, default_value = "512")]
    pub period_size: u32,

    /// If set to a port name ('foo:bar_1'), then all all inputs will be connected to that port. If
    /// the option is set to a comma separated list of port names ('foo:bar_1,foo:bar_2') then the
    /// input ports will be connected in that order. No inputs will be connected if the port option
    /// is not set.
    ///
    /// This option is only used with the JACK backend.
    #[clap(value_parser, long)]
    pub connect_jack_inputs: Option<String>,

    /// If set, then the plugin's MIDI input port will be connected to this JACK MIDI output port.
    ///
    /// This option is only used with the JACK backend.
    #[clap(value_parser, long)]
    pub connect_jack_midi_input: Option<String>,

    /// If set, then the plugin's MIDI output port will be connected to this JACK MIDI input port.
    ///
    /// This option is only used with the JACK backend.
    #[clap(value_parser, long)]
    pub connect_jack_midi_output: Option<String>,

    /// The editor's DPI scaling factor.
    ///
    /// This option is ignored on macOS.
    //
    // Currently baseview has no way to report this to us, so we'll expose it as a command line
    // option instead.
    #[clap(value_parser, long, default_value = "1.0")]
    pub dpi_scale: f32,

    /// The transport's tempo.
    #[clap(value_parser, long, default_value = "120")]
    pub tempo: f32,
    /// The time signature's numerator.
    #[clap(value_parser, long, default_value = "4")]
    pub timesig_num: u32,
    /// The time signature's denominator.
    #[clap(value_parser, long, default_value = "4")]
    pub timesig_denom: u32,
}

/// Determines which audio and MIDI backend should be used.
#[derive(Debug, Clone, ValueEnum)]
pub enum BackendType {
    /// Automatically pick the backend depending on what's available.
    ///
    /// This defaults to JACK if JACK is available, and falls back to the dummy backend if not.
    Auto,
    /// Use JACK for audio and MIDI.
    Jack,
    /// Use ALSA for audio and MIDI.
    #[cfg(target_os = "linux")]
    Alsa,
    /// Use CoreAudio for audio and MIDI.
    #[cfg(target_os = "macos")]
    CoreAudio,
    /// Use WASAPI for audio and MIDI.
    #[cfg(target_os = "windows")]
    Wasapi,
    /// Does not playback or receive any audio or MIDI.
    Dummy,
}
