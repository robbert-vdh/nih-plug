use anyhow::{Context, Result};
use cpal::traits::*;

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;
use crate::context::Transport;
use crate::midi::NoteEvent;
use crate::plugin::Plugin;

/// Uses CPAL for audio and midir for MIDI.
pub struct Cpal {
    // TODO:
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
            .map(|name| -> Result<cpal::Device> {
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

        anyhow::bail!("Not yet implemented");
    }
}
