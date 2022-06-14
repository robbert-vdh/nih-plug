use std::sync::Arc;

use anyhow::{Context, Result};
use atomic_refcell::AtomicRefCell;
use crossbeam::channel;
use jack::{AudioIn, AudioOut, Client, ClientOptions, ClosureProcessHandler, Control, Port};

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;

/// Uses JACK audio and MIDI.
pub struct Jack {
    /// The JACK client, wrapped in an option since it needs to be transformed into an `AsyncClient`
    /// and then back into a regular `Client`.
    client: Option<Client>,

    inputs: Arc<Vec<Port<AudioIn>>>,
    outputs: Arc<AtomicRefCell<Vec<Port<AudioOut>>>>,
}

/// A simple message to tell the audio thread to shut down, since the actual processing happens in
/// these callbacks.
enum Task {
    Shutdown,
}

impl Backend for Jack {
    fn run(&mut self, mut cb: impl FnMut(&mut Buffer) -> bool + 'static + Send) {
        let client = self.client.take().unwrap();
        let buffer_size = client.buffer_size();

        let mut buffer = Buffer::default();
        unsafe {
            buffer.with_raw_vec(|output_slices| {
                output_slices.resize_with(self.outputs.borrow().len(), || &mut []);
            })
        }

        let (control_sender, control_receiver) = channel::bounded(32);
        let inputs = self.inputs.clone();
        let outputs = self.outputs.clone();
        let process_handler = ClosureProcessHandler::new(move |_client, ps| {
            // In theory we could handle `num_frames <= buffer_size`, but JACK will never chop up
            // buffers like that so we'll just make it easier for ourselves by not supporting that
            let num_frames = ps.n_frames();
            if num_frames != buffer_size {
                nih_error!("Buffer size changed from {buffer_size} to {num_frames}. Buffer size changes are currently not supported, aborting...");
                control_sender.send(Task::Shutdown).unwrap();
                return Control::Quit;
            }

            // Just like all of the plugin backends, we need to grab the output slices and copy the
            // inputs to the outputs
            let mut outputs = outputs.borrow_mut();
            for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
                // XXX: Since the JACK bindings let us do this, presumably these two can't alias,
                //      right?
                output.as_mut_slice(ps).copy_from_slice(input.as_slice(ps));
            }

            // And the buffer's slices need to point to the JACK output ports
            unsafe {
                buffer.with_raw_vec(|output_slices| {
                    for (output_slice, output) in output_slices.iter_mut().zip(outputs.iter_mut()) {
                        // SAFETY: This buffer is only read from after in this callback, and the
                        //         reference passed to `cb` cannot outlive that function call
                        *output_slice = &mut *(output.as_mut_slice(ps) as *mut _);
                    }
                })
            }

            if cb(&mut buffer) {
                Control::Continue
            } else {
                control_sender.send(Task::Shutdown).unwrap();
                Control::Quit
            }
        });
        // TODO: What can go wrong here that would cause an error?
        let async_client = client.activate_async((), process_handler).unwrap();

        // The process callback happens on another thread, so we need to block this thread until we
        // get the request to shut down or until the process callback runs into an error
        #[allow(clippy::never_loop)]
        loop {
            match control_receiver.recv() {
                Ok(Task::Shutdown) => break,
                Err(err) => {
                    nih_debug_assert_failure!("Error reading from channel: {}", err);
                    break;
                }
            }
        }

        // And put the client back where it belongs in case this function is called a second time
        let (client, _, _) = async_client.deactivate().unwrap();
        self.client = Some(client);
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
            client: Some(client),

            inputs: Arc::new(inputs),
            outputs: Arc::new(AtomicRefCell::new(outputs)),
        })
    }
}
