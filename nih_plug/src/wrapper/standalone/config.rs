use clap::{Parser, ValueEnum};
use std::num::NonZeroU32;

use crate::prelude::{AudioIOLayout, Plugin};

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
    /// The audio layout to use. Defaults to the first layout.
    ///
    /// Specifying an empty argument or other invalid value will list all available audio layouts.
    //
    // NOTE: This takes a `String` instead of a `usize` so we can list the layouts when the argument
    //       is invalid
    #[clap(value_parser, short = 'l', long)]
    pub audio_layout: Option<String>,
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
    /// The input MIDI device for the ALSA, CoreAudio, and WASAPI backends.
    ///
    /// Specifying an empty string or other invalid value will list all available MIDI inputs.
    #[clap(value_parser, long)]
    pub midi_input: Option<String>,
    /// The output output device for the ALSA, CoreAudio, and WASAPI backends.
    ///
    /// Specifying an empty string or other invalid value will list all available MIDI output.
    #[clap(value_parser, long)]
    pub midi_output: Option<String>,

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

impl WrapperConfig {
    /// Get the audio IO layout for a plugin based on this configuration. Exits the application if
    /// the IO layout could not be parsed from the config. This doesn't return a `Result` to be able to differentiate between backend-specific errors and config parsing errors.
    pub fn audio_io_layout_or_exit<P: Plugin>(&self) -> AudioIOLayout {
        // The layouts are one-indexed here
        match &self.audio_layout {
            Some(audio_layout) if !P::AUDIO_IO_LAYOUTS.is_empty() => {
                match audio_layout.parse::<usize>() {
                    Ok(n) if n >= 1 && n - 1 < P::AUDIO_IO_LAYOUTS.len() => {
                        P::AUDIO_IO_LAYOUTS[n - 1]
                    }
                    _ => {
                        // This is made to be consistent with how audio input and output devices are
                        // listed in the CPAL backend
                        let mut layouts_str = String::new();
                        for (idx, layout) in P::AUDIO_IO_LAYOUTS.iter().enumerate() {
                            let num_input_channels = layout
                                .main_input_channels
                                .map(NonZeroU32::get)
                                .unwrap_or_default();
                            let num_output_channels = layout
                                .main_output_channels
                                .map(NonZeroU32::get)
                                .unwrap_or_default();
                            layouts_str.push_str(&format!(
                                "\n{}: {} ({} input {}, {} output {}{}{})",
                                idx + 1,
                                layout.name(),
                                num_input_channels,
                                if num_input_channels == 1 {
                                    "channel"
                                } else {
                                    "channels"
                                },
                                num_output_channels,
                                if num_output_channels == 1 {
                                    "channel"
                                } else {
                                    "channels"
                                },
                                if layout.aux_input_ports.is_empty() {
                                    String::new()
                                } else {
                                    format!("{} sidechain inputs", layout.aux_input_ports.len())
                                },
                                if layout.aux_output_ports.is_empty() {
                                    String::new()
                                } else {
                                    format!("{} sidechain outputs", layout.aux_output_ports.len())
                                },
                            ))
                        }

                        nih_log!("The available audio layouts are:{layouts_str}");

                        std::process::exit(1);
                    }
                }
            }
            _ => P::AUDIO_IO_LAYOUTS.first().copied().unwrap_or_default(),
        }
    }
}
