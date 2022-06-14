use anyhow::{Context, Result};
use jack::{AudioIn, AudioOut, Client, ClientOptions, Port};

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;

/// Uses JACK audio and MIDI.
pub struct Jack {
    config: WrapperConfig,
    client: Client,

    inputs: Vec<Port<AudioIn>>,
    outputs: Vec<Port<AudioOut>>,
}

impl Backend for Jack {
    fn run(&mut self, cb: impl FnMut(&mut Buffer) -> bool) {
        // TODO: Create an async client and do The Thing (tm)
        todo!()
    }
}

impl Jack {
    /// Initialize the JACK backend. Returns an error if this failed for whatever reason.
    pub fn new(name: &str, config: WrapperConfig) -> Result<Self> {
        let (client, status) = Client::new(name, ClientOptions::NO_START_SERVER)
            .context("Error while initializing the JACK client")?;
        if !status.is_empty() {
            anyhow::bail!("The JACK server returned an error: {status:?}");
        }

        let mut inputs = Vec::new();
        for port_no in 1..config.input_channels + 1 {
            inputs.push(client.register_port(&format!("input_{port_no}"), AudioIn)?);
        }

        let mut outputs = Vec::new();
        for port_no in 1..config.output_channels + 1 {
            let port = client.register_port(&format!("output_{port_no}"), AudioOut)?;

            // We don't connect the inputs automatically to avoid feedback loops, but this should be
            // safe. And if this fails, then that's fine.
            let system_playback_port_name = &format!("system:playback_{port_no}");
            let _ = client.connect_ports_by_name(&port.name()?, system_playback_port_name);

            outputs.push(port);
        }

        // TODO: Command line argument to connect the inputs?

        Ok(Self {
            config,
            client,

            inputs,
            outputs,
        })
    }
}
