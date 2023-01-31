use std::borrow::Borrow;
use std::sync::Arc;

use anyhow::{Context, Result};
use crossbeam::sync::Parker;
use jack::{
    AsyncClient, AudioIn, AudioOut, Client, ClientOptions, ClosureProcessHandler, Control, MidiIn,
    MidiOut, Port,
};
use parking_lot::Mutex;

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;
use crate::context::process::Transport;
use crate::midi::{MidiConfig, MidiResult, NoteEvent, PluginNoteEvent};
use crate::plugin::Plugin;

/// Uses JACK audio and MIDI.
pub struct Jack {
    config: WrapperConfig,
    /// The JACK client, wrapped in an option since it needs to be transformed into an `AsyncClient`
    /// and then back into a regular `Client`.
    client: Option<Client>,

    inputs: Arc<Vec<Port<AudioIn>>>,
    outputs: Arc<Mutex<Vec<Port<AudioOut>>>>,
    midi_input: Option<Arc<Port<MidiIn>>>,
    midi_output: Option<Arc<Mutex<Port<MidiOut>>>>,
}

impl<P: Plugin> Backend<P> for Jack {
    fn run(
        &mut self,
        mut cb: impl FnMut(
                &mut Buffer,
                Transport,
                &[PluginNoteEvent<P>],
                &mut Vec<PluginNoteEvent<P>>,
            ) -> bool
            + 'static
            + Send,
    ) {
        let client = self.client.take().unwrap();
        let buffer_size = client.buffer_size();

        let mut buffer = Buffer::default();
        unsafe {
            buffer.set_slices(0, |output_slices| {
                output_slices.resize_with(self.outputs.lock().len(), || &mut []);
            })
        }

        let mut input_events: Vec<PluginNoteEvent<P>> = Vec::with_capacity(2048);
        let mut output_events: Vec<PluginNoteEvent<P>> = Vec::with_capacity(2048);

        // This thread needs to be blocked until processing is finished
        let parker = Parker::new();
        let unparker = parker.unparker().clone();

        let config = self.config.clone();
        let inputs = self.inputs.clone();
        let outputs = self.outputs.clone();
        let midi_input = self.midi_input.clone();
        let midi_output = self.midi_output.clone();
        let process_handler = ClosureProcessHandler::new(move |client, ps| {
            // In theory we could handle `num_frames <= buffer_size`, but JACK will never chop up
            // buffers like that so we'll just make it easier for ourselves by not supporting that
            let num_frames = ps.n_frames();
            if num_frames != buffer_size {
                nih_error!(
                    "Buffer size changed from {buffer_size} to {num_frames}. Buffer size changes \
                     are currently not supported, aborting..."
                );
                unparker.unpark();
                return Control::Quit;
            }

            let mut transport = Transport::new(client.sample_rate() as f32);
            transport.tempo = Some(config.tempo as f64);
            transport.time_sig_numerator = Some(config.timesig_num as i32);
            transport.time_sig_denominator = Some(config.timesig_denom as i32);

            if let Ok(jack_transport) = client.transport().query() {
                transport.pos_samples = Some(jack_transport.pos.frame() as i64);
                transport.playing = jack_transport.state == jack::TransportState::Rolling;

                if let Some(bbt) = jack_transport.pos.bbt() {
                    transport.tempo = Some(bbt.bpm);
                    transport.time_sig_numerator = Some(bbt.sig_num as i32);
                    transport.time_sig_denominator = Some(bbt.sig_denom as i32);

                    transport.pos_beats = Some(
                        (bbt.bar as f64 * 4.0)
                            + (bbt.beat as f64 / bbt.sig_denom as f64 * 4.0)
                            + (bbt.tick as f64 / bbt.ticks_per_beat),
                    );
                    transport.bar_number = Some(bbt.bar as i32);
                }
            }

            // Just like all of the plugin backends, we need to grab the output slices and copy the
            // inputs to the outputs
            let mut outputs = outputs.lock();
            for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
                // XXX: Since the JACK bindings let us do this, presumably these two can't alias,
                //      right?
                output.as_mut_slice(ps).copy_from_slice(input.as_slice(ps));
            }

            // And the buffer's slices need to point to the JACK output ports
            unsafe {
                buffer.set_slices(num_frames as usize, |output_slices| {
                    for (output_slice, output) in output_slices.iter_mut().zip(outputs.iter_mut()) {
                        // SAFETY: This buffer is only read from after in this callback, and the
                        //         reference passed to `cb` cannot outlive that function call
                        *output_slice = &mut *(output.as_mut_slice(ps) as *mut _);
                    }
                })
            }

            input_events.clear();
            if let Some(midi_input) = &midi_input {
                input_events.extend(midi_input.iter(ps).filter_map(|midi| {
                    // Unless it is a SysEx message, a JACK MIDI message is always three bytes or
                    // less and is normalized (starts with a status byte and is self-contained).
                    if midi.bytes.len() <= 3 {
                        // JACK may not pad messages with zeroes, so messages for things like channel
                        // pressure may be less than three bytes in length.
                        let mut midi_data = [0u8; 3];
                        midi_data[..midi.bytes.len()].copy_from_slice(midi.bytes);

                        NoteEvent::from_midi(midi.time, &midi_data).ok()
                    } else {
                        None
                    }
                }));
            }

            output_events.clear();
            if cb(&mut buffer, transport, &input_events, &mut output_events) {
                if let Some(midi_output) = &midi_output {
                    let mut midi_output = midi_output.lock();
                    let mut midi_writer = midi_output.writer(ps);
                    for event in output_events.drain(..) {
                        let timing = event.timing();

                        let mut sysex_buffer = Default::default();
                        match event.as_midi(&mut sysex_buffer) {
                            Some(MidiResult::Basic(midi_data)) => {
                                let write_result = midi_writer.write(&jack::RawMidi {
                                    time: timing,
                                    bytes: &midi_data,
                                });

                                nih_debug_assert!(write_result.is_ok(), "The MIDI buffer is full");
                            }
                            Some(MidiResult::SysEx(length)) => {
                                // This feels a bit like gymnastics, but if the event was a SysEx
                                // event then `sysex_buffer` now contains the full message plus
                                // possibly some padding at the end
                                let sysex_buffer = sysex_buffer.borrow();
                                nih_debug_assert!(length <= sysex_buffer.len());
                                let write_result = midi_writer.write(&jack::RawMidi {
                                    time: timing,
                                    bytes: &sysex_buffer[..length],
                                });

                                nih_debug_assert!(write_result.is_ok(), "The MIDI buffer is full");
                            }
                            None => (),
                        }
                    }
                }

                Control::Continue
            } else {
                unparker.unpark();
                Control::Quit
            }
        });

        // PipeWire lets us connect the ports whenever we want, but JACK2 is very strict and only
        // allows us to connect the ports when the client is active. And the connections will
        // disappear when the client is deactivated. Fun.
        let async_client = client.activate_async((), process_handler).unwrap();
        if let Err(err) = self.connect_ports(&async_client) {
            nih_error!("Error connecting JACK ports: {err}")
        }

        // The process callback happens on another thread, so we need to block this thread until we
        // get the request to shut down or until the process callback runs into an error
        parker.park();

        // And put the client back where it belongs in case this function is called a second time
        let (client, _, _) = async_client.deactivate().unwrap();
        self.client = Some(client);
    }
}

impl Jack {
    /// Initialize the JACK backend. Returns an error if this failed for whatever reason. The plugin
    /// generic argument is to get the name for the client, and to know whether or not the
    /// standalone should expose JACK MIDI ports.
    pub fn new<P: Plugin>(config: WrapperConfig) -> Result<Self> {
        let (client, status) = Client::new(P::NAME, ClientOptions::NO_START_SERVER)
            .context("Error while initializing the JACK client")?;
        if !status.is_empty() {
            anyhow::bail!("The JACK server returned an error: {status:?}");
        }

        if config.connect_jack_inputs.is_none() && P::DEFAULT_INPUT_CHANNELS > 0 {
            nih_log!(
                "Audio inputs are not connected automatically to prevent feedback. Use the \
                 '--connect-jack-inputs' option to connect the input ports."
            )
        }

        let mut inputs = Vec::new();
        let num_input_channels = config.input_channels.unwrap_or(P::DEFAULT_INPUT_CHANNELS);
        for port_no in 1..num_input_channels + 1 {
            inputs.push(client.register_port(&format!("input_{port_no}"), AudioIn)?);
        }

        // We can't immediately connect the outputs. Or well we can with PipeWire, but JACK2 says
        // no. So the connections are made just after activating the client in the `run()` function
        // above.
        let mut outputs = Vec::new();
        let num_output_channels = config.output_channels.unwrap_or(P::DEFAULT_OUTPUT_CHANNELS);
        for port_no in 1..num_output_channels + 1 {
            outputs.push(client.register_port(&format!("output_{port_no}"), AudioOut)?);
        }

        let midi_input = if P::MIDI_INPUT >= MidiConfig::Basic {
            Some(Arc::new(client.register_port("midi_input", MidiIn)?))
        } else {
            None
        };

        let midi_output = if P::MIDI_OUTPUT >= MidiConfig::Basic {
            Some(Arc::new(Mutex::new(
                client.register_port("midi_output", MidiOut)?,
            )))
        } else {
            None
        };

        Ok(Self {
            config,
            client: Some(client),

            inputs: Arc::new(inputs),
            outputs: Arc::new(Mutex::new(outputs)),
            midi_input,
            midi_output,
        })
    }

    /// With JACK2 ports can only be connected while the client is active, and they'll be
    /// disconnected automatically on deactivation. So we need to call this as part of the `run()`
    /// function above.
    fn connect_ports<N, P>(&self, async_client: &AsyncClient<N, P>) -> Result<()> {
        let client = async_client.as_client();

        // We don't connect the inputs automatically to avoid feedback loops, but this should be
        // safe. And if this fails, then that's fine.
        for (i, output) in self.outputs.lock().iter().enumerate() {
            // The system ports are 1-indexed
            let port_no = i + 1;

            let system_playback_port_name = &format!("system:playback_{port_no}");
            let _ = client.connect_ports_by_name(&output.name()?, system_playback_port_name);
        }

        // This option can either be set to a single port all inputs should be connected to, or a
        // comma separated list of ports
        if let Some(port_name) = &self.config.connect_jack_inputs {
            if port_name.contains(',') {
                for (port_name, input) in port_name.split(',').zip(self.inputs.iter()) {
                    if let Err(err) = client.connect_ports_by_name(port_name, &input.name()?) {
                        nih_error!("Could not connect to '{port_name}': {err}");
                    }
                }
            } else {
                for input in self.inputs.iter() {
                    if let Err(err) = client.connect_ports_by_name(port_name, &input.name()?) {
                        nih_error!("Could not connect to '{port_name}': {err}");
                        break;
                    }
                }
            }
        }

        if let (Some(port), Some(port_name)) =
            (&self.midi_input, &self.config.connect_jack_midi_input)
        {
            if let Err(err) = client.connect_ports_by_name(port_name, &port.name()?) {
                nih_error!("Could not connect to '{port_name}': {err}");
            }
        }
        if let (Some(port), Some(port_name)) =
            (&self.midi_output, &self.config.connect_jack_midi_output)
        {
            if let Err(err) = client.connect_ports_by_name(&port.lock().name()?, port_name) {
                nih_error!("Could not connect to '{port_name}': {err}");
            }
        }

        Ok(())
    }
}
