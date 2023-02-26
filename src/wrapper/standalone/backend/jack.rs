use std::borrow::Borrow;
use std::num::NonZeroU32;
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
use crate::audio_setup::{AudioIOLayout, AuxiliaryBuffers};
use crate::buffer::Buffer;
use crate::context::process::Transport;
use crate::midi::{MidiConfig, MidiResult, NoteEvent, PluginNoteEvent};
use crate::plugin::Plugin;
use crate::wrapper::util::{clamp_input_event_timing, clamp_output_event_timing};

/// Uses JACK audio and MIDI.
pub struct Jack {
    audio_io_layout: AudioIOLayout,
    config: WrapperConfig,
    /// The JACK client, wrapped in an option since it needs to be transformed into an `AsyncClient`
    /// and then back into a regular `Client`.
    client: Option<Client>,

    main_inputs: Arc<Vec<Port<AudioIn>>>,
    main_outputs: Arc<Mutex<Vec<Port<AudioOut>>>>,
    aux_input_ports: Arc<Mutex<Vec<Vec<Port<AudioIn>>>>>,
    aux_output_ports: Arc<Mutex<Vec<Vec<Port<AudioOut>>>>>,
    midi_input: Option<Arc<Port<MidiIn>>>,
    midi_output: Option<Arc<Mutex<Port<MidiOut>>>>,
}

impl<P: Plugin> Backend<P> for Jack {
    fn run(
        &mut self,
        mut cb: impl FnMut(
                &mut Buffer,
                &mut AuxiliaryBuffers,
                Transport,
                &[PluginNoteEvent<P>],
                &mut Vec<PluginNoteEvent<P>>,
            ) -> bool
            + 'static
            + Send,
    ) {
        let client = self.client.take().unwrap();
        let buffer_size = client.buffer_size();

        // We'll preallocate the buffers here, and then assign them to the slices belonging to the
        // JACK ports later
        let mut buffer = Buffer::default();
        unsafe {
            buffer.set_slices(0, |output_slices| {
                output_slices.resize_with(self.main_outputs.lock().len(), || &mut []);
            })
        }

        // For the inputs we'll need to allocate storage because the NIH-plug API expects all
        // buffers to be mutable, and the jack crate doesn't give us mutable slices on audio input
        // ports
        let mut aux_input_storage: Vec<Vec<Vec<f32>>> = Vec::new();
        let mut aux_input_buffers: Vec<Buffer> = Vec::new();
        for channel_count in self.audio_io_layout.aux_input_ports {
            aux_input_storage.push(vec![
                vec![0.0f32; self.config.period_size as usize];
                channel_count.get() as usize
            ]);

            let mut aux_buffer = Buffer::default();
            unsafe {
                aux_buffer.set_slices(0, |output_slices| {
                    output_slices.resize_with(channel_count.get() as usize, || &mut []);
                })
            }
            aux_input_buffers.push(aux_buffer);
        }

        let mut aux_output_buffers: Vec<Buffer> = Vec::new();
        for channel_count in self.audio_io_layout.aux_output_ports {
            let mut aux_buffer = Buffer::default();
            unsafe {
                aux_buffer.set_slices(0, |output_slices| {
                    output_slices.resize_with(channel_count.get() as usize, || &mut []);
                })
            }
            aux_output_buffers.push(aux_buffer);
        }

        let mut input_events: Vec<PluginNoteEvent<P>> = Vec::with_capacity(2048);
        let mut output_events: Vec<PluginNoteEvent<P>> = Vec::with_capacity(2048);

        // This thread needs to be blocked until processing is finished
        let parker = Parker::new();
        let unparker = parker.unparker().clone();

        let config = self.config.clone();
        let main_inputs = self.main_inputs.clone();
        let main_outputs = self.main_outputs.clone();
        let aux_input_ports = self.aux_input_ports.clone();
        let aux_output_ports = self.aux_output_ports.clone();
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
            let mut main_outputs = main_outputs.lock();
            for (input, output) in main_inputs.iter().zip(main_outputs.iter_mut()) {
                // XXX: Since the JACK bindings let us do this, presumably these two can't alias,
                //      right?
                output.as_mut_slice(ps).copy_from_slice(input.as_slice(ps));
            }

            // And the buffer's slices need to point to the JACK output ports
            unsafe {
                buffer.set_slices(num_frames as usize, |output_slices| {
                    for (output_slice, output) in
                        output_slices.iter_mut().zip(main_outputs.iter_mut())
                    {
                        // SAFETY: This buffer is only read from after in this callback, and the
                        //         reference passed to `cb` cannot outlive that function call
                        *output_slice = &mut *(output.as_mut_slice(ps) as *mut _);
                    }
                });
            }

            // For auxiliary input ports we first need to copy the port data to our input storage
            // because the NIH-plug API expects every buffer to be mutable
            let mut aux_input_ports = aux_input_ports.lock();
            for (aux_inputs, (aux_storage, aux_buffer)) in aux_input_ports.iter_mut().zip(
                aux_input_storage
                    .iter_mut()
                    .zip(aux_input_buffers.iter_mut()),
            ) {
                for (aux_input, channel) in aux_inputs.iter_mut().zip(aux_storage.iter_mut()) {
                    channel.copy_from_slice(aux_input.as_slice(ps));
                }

                unsafe {
                    aux_buffer.set_slices(num_frames as usize, |input_slices| {
                        for (input_slice, channel) in
                            input_slices.iter_mut().zip(aux_storage.iter_mut())
                        {
                            // SAFETY: This buffer is only read from after in this callback, and the
                            //         reference passed to `cb` cannot outlive that function call
                            *input_slice = &mut *(channel.as_mut_slice() as *mut _);
                        }
                    });
                }
            }

            // We can point the buffers for the auxiliary output pots directly at the ports
            let mut aux_output_ports = aux_output_ports.lock();
            for (aux_outputs, aux_buffer) in aux_output_ports
                .iter_mut()
                .zip(aux_output_buffers.iter_mut())
            {
                unsafe {
                    aux_buffer.set_slices(num_frames as usize, |output_slices| {
                        for (output_slice, channel) in
                            output_slices.iter_mut().zip(aux_outputs.iter_mut())
                        {
                            // SAFETY: This buffer is only read from after in this callback, and the
                            //         reference passed to `cb` cannot outlive that function call
                            *output_slice = &mut *(channel.as_mut_slice(ps) as *mut _);
                        }
                    });
                }
            }

            input_events.clear();
            if let Some(midi_input) = &midi_input {
                input_events.extend(midi_input.iter(ps).filter_map(|midi| {
                    let timing = clamp_input_event_timing(midi.time, num_frames);

                    NoteEvent::from_midi(timing, midi.bytes).ok()
                }));
            }

            // SAFETY: Shortening these borrows is safe as even if the plugin overwrites the
            //         slices (which it cannot do without using unsafe code), then they
            //         would still be reset on the next iteration
            let mut aux = unsafe {
                AuxiliaryBuffers {
                    inputs: &mut *(aux_input_buffers.as_mut_slice() as *mut [Buffer]),
                    outputs: &mut *(aux_output_buffers.as_mut_slice() as *mut [Buffer]),
                }
            };

            output_events.clear();
            if cb(
                &mut buffer,
                &mut aux,
                transport,
                &input_events,
                &mut output_events,
            ) {
                if let Some(midi_output) = &midi_output {
                    let mut midi_output = midi_output.lock();
                    let mut midi_writer = midi_output.writer(ps);
                    for event in output_events.drain(..) {
                        // Out of bounds events are clamped to the buffer's size
                        let timing = clamp_output_event_timing(event.timing(), num_frames);

                        match event.as_midi() {
                            Some(MidiResult::Basic(midi_data)) => {
                                let write_result = midi_writer.write(&jack::RawMidi {
                                    time: timing,
                                    bytes: &midi_data,
                                });

                                nih_debug_assert!(write_result.is_ok(), "The MIDI buffer is full");
                            }
                            Some(MidiResult::SysEx(padded_sysex_buffer, length)) => {
                                // The SysEx buffer may contain padding
                                let padded_sysex_buffer = padded_sysex_buffer.borrow();
                                nih_debug_assert!(length <= padded_sysex_buffer.len());
                                let write_result = midi_writer.write(&jack::RawMidi {
                                    time: timing,
                                    bytes: &padded_sysex_buffer[..length],
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
        let audio_io_layout = config.audio_io_layout_or_exit::<P>();
        let plugin_name = P::NAME.to_lowercase().replace(' ', "_");
        let (client, status) = Client::new(&plugin_name, ClientOptions::NO_START_SERVER)
            .context("Error while initializing the JACK client")?;
        if !status.is_empty() {
            anyhow::bail!("The JACK server returned an error: {status:?}");
        }

        if config.connect_jack_inputs.is_none() && audio_io_layout.main_input_channels.is_some() {
            nih_log!(
                "Audio inputs are not connected automatically to prevent feedback. Use the \
                 '--connect-jack-inputs' option to connect the input ports."
            )
        }

        let mut main_inputs = Vec::new();
        let num_input_channels = audio_io_layout
            .main_input_channels
            .map(NonZeroU32::get)
            .unwrap_or_default() as usize;
        let main_input_name = audio_io_layout
            .main_input_name()
            .to_lowercase()
            .replace(' ', "_");
        for port_no in 1..num_input_channels + 1 {
            main_inputs
                .push(client.register_port(&format!("{main_input_name}_{port_no}"), AudioIn)?);
        }

        // We can't immediately connect the outputs. Or well we can with PipeWire, but JACK2 says
        // no. So the connections are made just after activating the client in the `run()` function
        // above.
        let mut main_outputs = Vec::new();
        let num_output_channels = audio_io_layout
            .main_output_channels
            .map(NonZeroU32::get)
            .unwrap_or_default() as usize;
        let main_output_name = audio_io_layout
            .main_output_name()
            .to_lowercase()
            .replace(' ', "_");
        for port_no in 1..num_output_channels + 1 {
            main_outputs
                .push(client.register_port(&format!("{main_output_name}_{port_no}"), AudioOut)?);
        }

        // The JACK backend also exposes ports for auxiliary inputs and outputs
        let mut aux_input_ports = Vec::new();
        for (aux_input_idx, channel_count) in audio_io_layout.aux_input_ports.iter().enumerate() {
            let aux_input_name = audio_io_layout
                .aux_input_name(aux_input_idx)
                .expect("Out of range aux input port")
                .to_lowercase()
                .replace(' ', "_");

            let mut ports = Vec::new();
            for port_no in 1..channel_count.get() + 1 {
                ports.push(client.register_port(&format!("{aux_input_name}_{port_no}"), AudioIn)?);
            }

            aux_input_ports.push(ports);
        }

        let mut aux_output_ports = Vec::new();
        for (aux_output_idx, channel_count) in audio_io_layout.aux_output_ports.iter().enumerate() {
            let aux_output_name = audio_io_layout
                .aux_output_name(aux_output_idx)
                .expect("Out of range aux output port")
                .to_lowercase()
                .replace(' ', "_");

            let mut ports = Vec::new();
            for port_no in 1..channel_count.get() + 1 {
                ports
                    .push(client.register_port(&format!("{aux_output_name}_{port_no}"), AudioOut)?);
            }

            aux_output_ports.push(ports);
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
            audio_io_layout,
            config,
            client: Some(client),

            main_inputs: Arc::new(main_inputs),
            main_outputs: Arc::new(Mutex::new(main_outputs)),
            aux_input_ports: Arc::new(Mutex::new(aux_input_ports)),
            aux_output_ports: Arc::new(Mutex::new(aux_output_ports)),
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
        for (i, output) in self.main_outputs.lock().iter().enumerate() {
            // The system ports are 1-indexed
            let port_no = i + 1;

            let system_playback_port_name = &format!("system:playback_{port_no}");
            let _ = client.connect_ports_by_name(&output.name()?, system_playback_port_name);
        }

        // This option can either be set to a single port all inputs should be connected to, or a
        // comma separated list of ports
        if let Some(port_name) = &self.config.connect_jack_inputs {
            if port_name.contains(',') {
                for (port_name, input) in port_name.split(',').zip(self.main_inputs.iter()) {
                    if let Err(err) = client.connect_ports_by_name(port_name, &input.name()?) {
                        nih_error!("Could not connect to '{port_name}': {err}");
                    }
                }
            } else {
                for input in self.main_inputs.iter() {
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
