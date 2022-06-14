use anyhow::{Context, Result};
use jack::{Client, ClientOptions};

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;

/// Uses JACK audio and MIDI.
pub struct Jack {
    config: WrapperConfig,
    client: Client,
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

        // TODO: Register ports
        // TODO: Connect output
        // TODO: Command line argument to connect the inputs?

        Ok(Self { config, client })
    }
}
