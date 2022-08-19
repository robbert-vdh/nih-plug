use anyhow::{Context, Result};
use cpal::{traits::*, Device, SampleFormat, StreamConfig};

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;
use crate::context::Transport;
use crate::midi::NoteEvent;
use crate::plugin::{AuxiliaryIOConfig, BusConfig, Plugin};

/// Uses CPAL for audio and midir for MIDI.
pub struct Cpal {
    bus_config: BusConfig,
    input_device: Option<Device>,
    input_config: Option<StreamConfig>,
    output_device: Device,
    output_config: StreamConfig,
    // TODO: MIDI
}

impl Backend for Cpal {
    fn run(
        &mut self,
        mut cb: impl FnMut(&mut Buffer, Transport, &[NoteEvent], &mut Vec<NoteEvent>) -> bool
            + 'static
            + Send,
    ) {
        // TODO:
    }
}

impl Cpal {
    /// Initialize the backend with the specified host. Returns an error if this failed for whatever
    /// reason.
    pub fn new<P: Plugin>(config: WrapperConfig, cpal_host_id: cpal::HostId) -> Result<Self> {
        let host = cpal::host_from_id(cpal_host_id).context("The Audio API is unavailable")?;

        // No input device is connected unless requested by the user to avoid feedback loops
        let input_device = config
            .input_device
            .map(|name| -> Result<Device> {
                let device = host
                    .input_devices()
                    .context("No audio input devices available")?
                    // `.name()` returns a `Result` with a non-Eq error type so you can't compare this
                    // directly
                    .find(|d| d.name().as_deref().map(|n| n == name).unwrap_or(false))
                    .with_context(|| {
                        // This is a bit awkward, but instead of adding a dedicated option we'll just
                        // list all of the available devices in the error message when the chosen device
                        // does not exist
                        let mut message =
                            format!("Unknown input device '{name}'. Available devices are:");
                        for device_name in host.input_devices().unwrap().flat_map(|d| d.name()) {
                            message.push_str(&format!("\n{device_name}"))
                        }

                        message
                    })?;

                Ok(device)
            })
            .transpose()?;

        let output_device = match config.output_device {
            Some(name) => host
                .output_devices()
                .context("No audio output devices available")?
                .find(|d| d.name().as_deref().map(|n| n == name).unwrap_or(false))
                .with_context(|| {
                    let mut message =
                        format!("Unknown output device '{name}'. Available devices are:");
                    for device_name in host.output_devices().unwrap().flat_map(|d| d.name()) {
                        message.push_str(&format!("\n{device_name}"))
                    }

                    message
                })?,
            None => host
                .default_output_device()
                .context("No default audio output device available")?,
        };

        let bus_config = BusConfig {
            num_input_channels: config.input_channels.unwrap_or(P::DEFAULT_INPUT_CHANNELS),
            num_output_channels: config.output_channels.unwrap_or(P::DEFAULT_OUTPUT_CHANNELS),
            // TODO: Support these in the standalone
            aux_input_busses: AuxiliaryIOConfig::default(),
            aux_output_busses: AuxiliaryIOConfig::default(),
        };
        let requested_sample_rate = cpal::SampleRate(config.sample_rate as u32);
        let requested_buffer_size = cpal::BufferSize::Fixed(config.period_size);

        let input_config = input_device
            .as_ref()
            .map(|device| -> Result<StreamConfig> {
                let input_configs: Vec<_> = device
                    .supported_input_configs()
                    .context("Could not get supported audio input configurations")?
                    .filter(|c| match c.buffer_size() {
                        cpal::SupportedBufferSize::Range { min, max } => {
                            c.channels() as u32 == bus_config.num_input_channels
                                && (c.min_sample_rate()..=c.max_sample_rate())
                                    .contains(&requested_sample_rate)
                                && (min..=max).contains(&&config.period_size)
                        }
                        cpal::SupportedBufferSize::Unknown => false,
                    })
                    .collect();
                let input_config_range = input_configs
                    .iter()
                    // Prefer floating point samples to avoid conversions
                    .find(|c| c.sample_format() == SampleFormat::F32)
                    .or_else(|| input_configs.first())
                    .cloned()
                    .with_context(|| {
                        format!(
                            "The audio input device does not support {} audio channels at a \
                             sample rate of {} Hz and a period size of {} samples",
                            bus_config.num_input_channels, config.sample_rate, config.period_size,
                        )
                    })?;

                // We already checked that these settings are valid
                Ok(StreamConfig {
                    channels: input_config_range.channels(),
                    sample_rate: requested_sample_rate,
                    buffer_size: requested_buffer_size.clone(),
                })
            })
            .transpose()?;

        let output_configs: Vec<_> = output_device
            .supported_output_configs()
            .context("Could not get supported audio output configurations")?
            .filter(|c| match c.buffer_size() {
                cpal::SupportedBufferSize::Range { min, max } => {
                    c.channels() as u32 == bus_config.num_output_channels
                        && (c.min_sample_rate()..=c.max_sample_rate())
                            .contains(&requested_sample_rate)
                        && (min..=max).contains(&&config.period_size)
                }
                cpal::SupportedBufferSize::Unknown => false,
            })
            .collect();
        let output_config_range = output_configs
            .iter()
            .find(|c| c.sample_format() == SampleFormat::F32)
            .or_else(|| output_configs.first())
            .cloned()
            .with_context(|| {
                format!(
                    "The audio output device does not support {} audio channels at a sample rate \
                     of {} Hz and a period size of {} samples",
                    bus_config.num_output_channels, config.sample_rate, config.period_size,
                )
            })?;
        let output_config = StreamConfig {
            channels: output_config_range.channels(),
            sample_rate: requested_sample_rate,
            buffer_size: requested_buffer_size,
        };

        Ok(Cpal {
            bus_config,
            input_device,
            input_config,
            output_device,
            output_config,
        })
    }
}
