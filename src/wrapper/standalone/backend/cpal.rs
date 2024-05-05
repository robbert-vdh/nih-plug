use anyhow::{Context, Result};
use cpal::{
    traits::*, Device, FromSample, InputCallbackInfo, OutputCallbackInfo, Sample, SampleFormat,
    Stream, StreamConfig,
};
use crossbeam::sync::{Parker, Unparker};
use midir::{
    MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection, MidiOutputPort,
};
use parking_lot::Mutex;
use rtrb::RingBuffer;
use std::borrow::Borrow;
use std::num::NonZeroU32;
use std::ptr::NonNull;
use std::thread::ScopedJoinHandle;

use super::super::config::WrapperConfig;
use super::Backend;
use crate::midi::MidiResult;
use crate::prelude::{
    AudioIOLayout, AuxiliaryBuffers, Buffer, MidiConfig, NoteEvent, Plugin, PluginNoteEvent,
    Transport,
};
use crate::wrapper::util::buffer_management::{BufferManager, ChannelPointers};

const MIDI_EVENT_QUEUE_CAPACITY: usize = 2048;

/// Uses CPAL for audio and midir for MIDI.
pub struct CpalMidir {
    config: WrapperConfig,
    audio_io_layout: AudioIOLayout,

    input: Option<CpalDevice>,
    output: CpalDevice,

    midi_input: Mutex<Option<MidirInputDevice>>,
    midi_output: Mutex<Option<MidirOutputDevice>>,
}

/// All data needed for a CPAL input or output stream.
struct CpalDevice {
    pub device: Device,
    pub config: StreamConfig,
    pub sample_format: SampleFormat,
}

/// All data needed to create a Midir input stream.
struct MidirInputDevice {
    pub backend: MidiInput,
    pub port: MidiInputPort,
}

/// An active `MidirInputDevice`. Transformed back and from this during the `.run()` function.
struct ActiveMidirInputDevice {
    pub connection: MidiInputConnection<()>,
    pub port: MidiInputPort,
}

/// All data needed to create a Midir output stream.
struct MidirOutputDevice {
    pub backend: MidiOutput,
    pub port: MidiOutputPort,
}

/// An active `MidirOutputDevice`. Transformed back and from this during the `.run()` function.
struct ActiveMidirOutputDevice {
    pub connection: MidiOutputConnection,
    pub port: MidiOutputPort,
}

/// Send+Sync wrapper for `Vec<*mut f32>` so we can preallocate channel pointer vectors for use with
/// the `BufferManager` API.
struct ChannelPointerVec(Vec<*mut f32>);

unsafe impl Send for ChannelPointerVec {}
unsafe impl Sync for ChannelPointerVec {}

impl ChannelPointerVec {
    // If you directly access the `.0` field then it will try to move it out of the struct which
    // undoes the Send+Sync impl.
    pub fn get(&mut self) -> &mut Vec<*mut f32> {
        &mut self.0
    }
}

/// A task for the MIDI output thread.
enum MidiOutputTask<P: Plugin> {
    /// Send an event as MIDI data.
    Send(PluginNoteEvent<P>),
    /// Terminate the thread, stopping it from blocking and allowing it to be joined.
    Terminate,
}

impl<P: Plugin> Backend<P> for CpalMidir {
    fn run(
        &mut self,
        cb: impl FnMut(
                &mut Buffer,
                &mut AuxiliaryBuffers,
                Transport,
                &[PluginNoteEvent<P>],
                &mut Vec<PluginNoteEvent<P>>,
            ) -> bool
            + 'static
            + Send,
    ) {
        // So this is a lot of fun. There are up to four separate streams here, all using their own
        // callbacks. The audio output stream acts as the primary stream, and everything else either
        // sends data to it or (in the case of the MIDI output stream) receives data from it using
        // channels.
        //
        // Audio input is read from the input device (if configured), and is send at a period at a
        // time to the output stream in an interleaved format. Because of that the audio output
        // stream is delayed for one period using a parker to you don't immediately get xruns. CPAL
        // audio devices may also not accept floating point samples, so all of the actual audio
        // handling and buffer management handles in the `build_*_data_callback()` functions defined
        // below.
        //
        // MIDI input is parsed in the Midir callback and the events are sent over a callback to the
        // output audio thread where the process callback happens. If that process callback outputs
        // events then those are sent over another ringbuffer to a thread that handles MIDI output.
        // Both MIDI input and MIDI output are disabled by default.
        //
        // The thread scope is needed to accomodate the midir MIDI output API. Outputting MIDI is
        // realtime unsafe, and to be able to output MIDI with midir you need to transform between
        // `MidiOutputPort` and `MidiOutputPortConnection` types by taking values out of an
        // `Option`.
        std::thread::scope(|s| {
            let mut _input_stream: Option<Stream> = None;
            let mut input_rb_consumer: Option<rtrb::Consumer<f32>> = None;
            if let Some(input) = &self.input {
                // Data is sent to the output data callback using a wait-free ring buffer
                let (rb_producer, rb_consumer) = RingBuffer::new(
                    self.output.config.channels as usize * self.config.period_size as usize,
                );
                input_rb_consumer = Some(rb_consumer);

                let input_parker = Parker::new();
                let input_unparker = input_parker.unparker().clone();
                let error_cb = {
                    let input_unparker = input_unparker.clone();
                    move |err| {
                        nih_error!("Error during capture: {err:#}");
                        input_unparker.clone().unpark();
                    }
                };

                macro_rules! build_input_streams {
                    ($sample_format:expr, $(($format:path, $primitive_type:ty)),*) => {
                        match $sample_format {
                            $($format => input.device.build_input_stream(
                                &input.config,
                                self.build_input_data_callback::<$primitive_type>(input_unparker, rb_producer),
                                error_cb,
                                None,
                            ),)*
                            format => todo!("Unsupported sample format {format}"),
                        }
                    }
                }
                let stream = build_input_streams!(
                    input.sample_format,
                    (SampleFormat::I8, i8),
                    (SampleFormat::I16, i16),
                    (SampleFormat::I32, i32),
                    (SampleFormat::I64, i64),
                    (SampleFormat::U8, u8),
                    (SampleFormat::U16, u16),
                    (SampleFormat::U32, u32),
                    (SampleFormat::U64, u64),
                    (SampleFormat::F32, f32),
                    (SampleFormat::F64, f64)
                )
                .expect("Fatal error creating the capture stream");
                stream
                    .play()
                    .expect("Fatal error trying to start the capture stream");
                _input_stream = Some(stream);

                // Playback is delayed one period if we're capturing audio so it has something to
                // process
                input_parker.park()
            }

            // The output callback can read input events from this ringbuffer
            let mut midi_input_rb_consumer: Option<rtrb::Consumer<PluginNoteEvent<P>>> = None;
            let midi_input_connection: Option<ActiveMidirInputDevice> =
                self.midi_input.lock().take().and_then(|midi_input| {
                    // Data is sent to the output data callback using a wait-free ring buffer
                    let (rb_producer, rb_consumer) = RingBuffer::new(MIDI_EVENT_QUEUE_CAPACITY);
                    midi_input_rb_consumer = Some(rb_consumer);

                    let result = midi_input.backend.connect(
                        &midi_input.port,
                        "MIDI input",
                        self.build_midi_input_thread::<P>(rb_producer),
                        (),
                    );

                    match result {
                        Ok(connection) => Some(ActiveMidirInputDevice {
                            connection,
                            port: midi_input.port,
                        }),
                        Err(err) => {
                            // We won't retry once this fails
                            nih_error!("Could not create the MIDI input connection: {err:#}");
                            midi_input_rb_consumer = None;

                            None
                        }
                    }
                });

            // The output callback can also emit MIDI events. To handle these we'll need to spawn
            // our own thread. This can be simplified a lot by using the `MidiOutputConnection`
            // directly inside the audio output callback, but looking at the implementation sending
            // MIDI events is not realtime safe in most midir backends.
            // NOTE: This uses crossbeam channels instead of rtrb specifically for the optional
            //        blocking API. This lets the MIDI sending thread sleep when there's no work to
            //        do.
            let mut midi_output_rb_producer: Option<crossbeam::channel::Sender<MidiOutputTask<P>>> =
                None;
            let midi_output_connection: Option<ScopedJoinHandle<ActiveMidirOutputDevice>> =
                self.midi_output.lock().take().and_then(|midi_output| {
                    // This uses crossbeam channels for the reason mentioned above, but to keep
                    // things cohesive we'll use the same naming scheme as we use for rtrb
                    let (sender, receiver) = crossbeam::channel::bounded(MIDI_EVENT_QUEUE_CAPACITY);
                    midi_output_rb_producer = Some(sender);

                    let result = midi_output
                        .backend
                        .connect(&midi_output.port, "MIDI output");

                    match result {
                        Ok(mut connection) => Some(s.spawn(move || {
                            while let Ok(task) = receiver.recv() {
                                match task {
                                    MidiOutputTask::Send(event) => match event.as_midi() {
                                        Some(MidiResult::Basic(midi_data)) => {
                                            if let Err(err) = connection.send(&midi_data) {
                                                nih_error!("Could not send MIDI event: {err}");
                                            }
                                        }
                                        Some(MidiResult::SysEx(padded_sysex_buffer, length)) => {
                                            // The SysEx buffer may contain padding
                                            let padded_sysex_buffer = padded_sysex_buffer.borrow();
                                            nih_debug_assert!(length <= padded_sysex_buffer.len());

                                            if let Err(err) =
                                                connection.send(&padded_sysex_buffer[..length])
                                            {
                                                nih_error!("Could not send MIDI event: {err}");
                                            }
                                        }
                                        None => (),
                                    },
                                    MidiOutputTask::Terminate => break,
                                }
                            }

                            // We'll return the same value from the join handle as what ends up
                            // being stored in `midi_input_connection` to keep this symmetrical with
                            // the input handling
                            ActiveMidirOutputDevice {
                                connection,
                                port: midi_output.port,
                            }
                        })),
                        Err(err) => {
                            nih_error!("Could not create the MIDI output connection: {err:#}");
                            midi_output_rb_producer = None;

                            None
                        }
                    }
                });

            // This thread needs to be blocked until audio processing ends as CPAL processes the
            // streams on another thread instead of blocking
            let parker = Parker::new();
            let unparker = parker.unparker().clone();
            let error_cb = {
                let unparker = unparker.clone();
                move |err| {
                    nih_error!("Error during playback: {err:#}");
                    unparker.clone().unpark();
                }
            };

            macro_rules! build_output_streams {
                ($sample_format:expr, $(($format:path, $primitive_type:ty)),*) => {
                    match $sample_format {
                        $($format => self.output.device.build_output_stream(
                            &self.output.config,
                            self.build_output_data_callback::<P, $primitive_type>(
                                unparker,
                                input_rb_consumer,
                                midi_input_rb_consumer,
                                // This is a MPMC crossbeam channel instead of an rtrb ringbuffer, and we
                                // also need it to terminate the thread
                                midi_output_rb_producer.clone(),
                                cb,
                            ),
                            error_cb,
                            None,
                        ),)*
                        format => todo!("Unsupported sample format {format}"),
                    }
                }
            }
            let output_stream = build_output_streams!(
                self.output.sample_format,
                (SampleFormat::I8, i8),
                (SampleFormat::I16, i16),
                (SampleFormat::I32, i32),
                (SampleFormat::I64, i64),
                (SampleFormat::U8, u8),
                (SampleFormat::U16, u16),
                (SampleFormat::U32, u32),
                (SampleFormat::U64, u64),
                (SampleFormat::F32, f32),
                (SampleFormat::F64, f64)
            )
            .expect("Fatal error creating the output stream");

            // TODO: Wait a period before doing this when also reading the input
            output_stream
                .play()
                .expect("Fatal error trying to start the output stream");

            // Wait for the audio thread to exit
            parker.park();

            // The Midir API requires us to take things out of Options and transform between these
            // structs
            *self.midi_input.lock() =
                midi_input_connection.map(|midi_input_connection| MidirInputDevice {
                    backend: midi_input_connection.connection.close().0,
                    port: midi_input_connection.port,
                });
            *self.midi_output.lock() =
                midi_output_connection.map(move |midi_output_connection_handle| {
                    // The thread needs to be terminated first
                    midi_output_rb_producer
                        .expect("Inconsistent internal MIDI output state")
                        .send(MidiOutputTask::Terminate)
                        .expect("Could not terminate the MIDI output thread");

                    let midi_output_connection = midi_output_connection_handle
                        .join()
                        .expect("MIDI output thread panicked");

                    MidirOutputDevice {
                        backend: midi_output_connection.connection.close(),
                        port: midi_output_connection.port,
                    }
                });
        });
    }
}

impl CpalMidir {
    /// Initialize the backend with the specified host. Returns an error if this failed for whatever
    /// reason.
    pub fn new<P: Plugin>(config: WrapperConfig, cpal_host_id: cpal::HostId) -> Result<Self> {
        let audio_io_layout = config.audio_io_layout_or_exit::<P>();
        let host = cpal::host_from_id(cpal_host_id).context("The Audio API is unavailable")?;

        if config.input_device.is_none() && audio_io_layout.main_input_channels.is_some() {
            nih_log!(
                "Audio inputs are not connected automatically to prevent feedback. Use the \
                 '--input-device' option to choose an input device."
            )
        }

        if config.midi_input.is_none() && P::MIDI_INPUT >= MidiConfig::Basic {
            nih_log!("Use the '--midi-input' option to select a MIDI input device.")
        }
        if config.midi_output.is_none() && P::MIDI_OUTPUT >= MidiConfig::Basic {
            nih_log!("Use the '--midi-output' option to select a MIDI output device.")
        }

        // No input device is connected unless requested by the user to avoid feedback loops
        let input_device = config
            .input_device
            .as_ref()
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

        let output_device = match config.output_device.as_ref() {
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

        let requested_sample_rate = cpal::SampleRate(config.sample_rate as u32);
        let requested_buffer_size = cpal::BufferSize::Fixed(config.period_size);
        let num_input_channels = audio_io_layout
            .main_input_channels
            .map(NonZeroU32::get)
            .unwrap_or_default() as usize;
        let input = input_device
            .map(|device| -> Result<CpalDevice> {
                let input_configs: Vec<_> = device
                    .supported_input_configs()
                    .context("Could not get supported audio input configurations")?
                    .filter(|c| match c.buffer_size() {
                        cpal::SupportedBufferSize::Range { min, max } => {
                            c.channels() as usize == num_input_channels
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
                            num_input_channels, config.sample_rate, config.period_size,
                        )
                    })?;

                // We already checked that these settings are valid
                let input_config = StreamConfig {
                    channels: input_config_range.channels(),
                    sample_rate: requested_sample_rate,
                    buffer_size: requested_buffer_size,
                };
                let input_sample_format = input_config_range.sample_format();

                Ok(CpalDevice {
                    device,
                    config: input_config,
                    sample_format: input_sample_format,
                })
            })
            .transpose()?;

        let num_output_channels = audio_io_layout
            .main_output_channels
            .map(NonZeroU32::get)
            .unwrap_or_default() as usize;
        let output = {
            let output_configs: Vec<_> = output_device
                .supported_output_configs()
                .context("Could not get supported audio output configurations")?
                .filter(|c| match c.buffer_size() {
                    cpal::SupportedBufferSize::Range { min, max } => {
                        c.channels() as usize == num_output_channels
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
                        "The audio output device does not support {} audio channels at a sample \
                         rate of {} Hz and a period size of {} samples",
                        num_output_channels, config.sample_rate, config.period_size,
                    )
                })?;
            let output_config = StreamConfig {
                channels: output_config_range.channels(),
                sample_rate: requested_sample_rate,
                buffer_size: requested_buffer_size,
            };
            let output_sample_format = output_config_range.sample_format();

            CpalDevice {
                device: output_device,
                config: output_config,
                sample_format: output_sample_format,
            }
        };

        // There's no obvious way to do sidechain inputs and additional outputs with the CPAL
        // backends like there is with JACK. So we'll just provide empty buffers instead.
        if !audio_io_layout.aux_input_ports.is_empty() {
            nih_warn!("Sidechain inputs are not supported with this audio backend");
        }
        if !audio_io_layout.aux_output_ports.is_empty() {
            nih_warn!("Auxiliary outputs are not supported with this audio backend");
        }

        let midi_input = match &config.midi_input {
            Some(midi_input_name) => {
                // Midir lets us preemptively ignore MIDI messages we'll never use like active
                // sensing and timing, but for maximum flexibility with NIH-plug's SysEx parsing
                // types (which could technically be used to also parse those things) we won't do
                // that.
                let midi_backend = MidiInput::new(P::NAME)
                    .context("Could not initialize the MIDI input backend")?;
                let available_ports = midi_backend.ports();

                // In case there somehow is a MIDI port with an empty name, we'll still want to
                // preserve the behavior of an empty argument resulting in a listing of options.
                let found_port = if !midi_input_name.is_empty() {
                    // This API is a bit weird
                    available_ports
                        .iter()
                        .find(|port| midi_backend.port_name(port).as_deref() == Ok(midi_input_name))
                } else {
                    None
                };

                match found_port {
                    Some(port) => Some(MidirInputDevice {
                        backend: midi_backend,
                        port: port.clone(),
                    }),
                    None => {
                        let mut message = format!(
                            "Unknown input MIDI device '{midi_input_name}'. Available devices are:"
                        );
                        for port in available_ports {
                            match midi_backend.port_name(&port) {
                                Ok(device_name) => message.push_str(&format!("\n{device_name}")),
                                Err(err) => message.push_str(&format!("\nERROR: {err:#}")),
                            }
                        }

                        anyhow::bail!(message);
                    }
                }
            }
            None => None,
        };

        let midi_output = match &config.midi_output {
            Some(midi_output_name) => {
                let midi_backend = MidiOutput::new(P::NAME)
                    .context("Could not initialize the MIDI output backend")?;
                let available_ports = midi_backend.ports();

                let found_port = if !midi_output_name.is_empty() {
                    available_ports.iter().find(|port| {
                        midi_backend.port_name(port).as_deref() == Ok(midi_output_name)
                    })
                } else {
                    None
                };

                match found_port {
                    Some(port) => Some(MidirOutputDevice {
                        backend: midi_backend,
                        port: port.clone(),
                    }),
                    None => {
                        let mut message = format!(
                            "Unknown output MIDI device '{midi_output_name}'. Available devices \
                             are:"
                        );
                        for port in available_ports {
                            match midi_backend.port_name(&port) {
                                Ok(device_name) => message.push_str(&format!("\n{device_name}")),
                                Err(err) => message.push_str(&format!("\nERROR: {err:#}")),
                            }
                        }

                        anyhow::bail!(message);
                    }
                }
            }
            None => None,
        };

        Ok(CpalMidir {
            config,
            audio_io_layout,

            input,
            output,

            midi_input: Mutex::new(midi_input),
            midi_output: Mutex::new(midi_output),
        })
    }

    fn build_input_data_callback<T>(
        &self,
        input_unparker: Unparker,
        mut input_rb_producer: rtrb::Producer<f32>,
    ) -> impl FnMut(&[T], &InputCallbackInfo) + Send + 'static
    where
        T: Sample,
        // The CPAL update made the whole interface more complicated by switching to dasp's sample
        // trait, and then they also forgot to expose the `ToSample` trait so now you need to do
        // this
        f32: FromSample<T>,
    {
        // This callback needs to copy input samples to a ring buffer that can be read from in the
        // output data callback
        move |data, _info| {
            for sample in data {
                // If for whatever reason the input callback is fired twice before an output
                // callback, then just spin on this until the push succeeds
                while input_rb_producer.push(sample.to_sample()).is_err() {}
            }

            // The run function is blocked until a single period has been processed here. After this
            // point output playback can start.
            input_unparker.unpark();
        }
    }

    fn build_midi_input_thread<P: Plugin>(
        &self,
        mut midi_input_rb_producer: rtrb::Producer<PluginNoteEvent<P>>,
    ) -> impl FnMut(u64, &[u8], &mut ()) + Send + 'static {
        // This callback parses the received MIDI bytes and sends them to a ring buffer
        move |_timing, midi_data, _data| {
            // Since this is system MIDI there's no real useful timing information and we'll set all
            // the timings to the first sample in the buffer
            if let Ok(event) = NoteEvent::from_midi(0, midi_data) {
                if midi_input_rb_producer.push(event).is_err() {
                    nih_error!("The MIDI input event queue was full, dropping event");
                }
            }
        }
    }

    fn build_output_data_callback<P, T>(
        &self,
        unparker: Unparker,
        mut input_rb_consumer: Option<rtrb::Consumer<f32>>,
        mut input_event_rb_consumer: Option<rtrb::Consumer<PluginNoteEvent<P>>>,
        mut output_event_rb_producer: Option<crossbeam::channel::Sender<MidiOutputTask<P>>>,
        mut cb: impl FnMut(
                &mut Buffer,
                &mut AuxiliaryBuffers,
                Transport,
                &[PluginNoteEvent<P>],
                &mut Vec<PluginNoteEvent<P>>,
            ) -> bool
            + 'static
            + Send,
    ) -> impl FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static
    where
        P: Plugin,
        T: Sample + FromSample<f32>,
    {
        // We'll receive interlaced input samples from CPAL. These need to converted to deinterlaced
        // channels, processed, and then copied those back to an interlaced buffer for the output.
        let buffer_size = self.config.period_size as usize;
        let num_output_channels = self
            .audio_io_layout
            .main_output_channels
            .map(NonZeroU32::get)
            .unwrap_or(0) as usize;
        let num_input_channels = self
            .audio_io_layout
            .main_input_channels
            .map(NonZeroU32::get)
            .unwrap_or(0) as usize;
        // This may contain excess unused space at the end if we get fewer samples than configured
        // from CPAL
        let mut main_io_storage = vec![vec![0.0f32; buffer_size]; num_output_channels];

        // This backend does not support auxiliary inputs and outputs, so in order to have the same
        // behavior as the other backends we'll provide some dummy buffers that we'll zero out every
        // time
        let mut aux_input_storage: Vec<Vec<Vec<f32>>> = Vec::new();
        for channel_count in self.audio_io_layout.aux_input_ports {
            aux_input_storage.push(vec![
                vec![0.0f32; buffer_size];
                channel_count.get() as usize
            ]);
        }

        let mut aux_output_storage: Vec<Vec<Vec<f32>>> = Vec::new();
        for channel_count in self.audio_io_layout.aux_output_ports {
            aux_output_storage.push(vec![
                vec![0.0f32; buffer_size];
                channel_count.get() as usize
            ]);
        }

        // The actual buffer management here works the same as in the JACK backend. See that
        // implementation for more information.
        let mut buffer_manager =
            BufferManager::for_audio_io_layout(buffer_size, self.audio_io_layout);
        let mut main_io_channel_pointers = ChannelPointerVec(Vec::with_capacity(
            self.audio_io_layout
                .main_output_channels
                .map(NonZeroU32::get)
                .unwrap_or(0) as usize,
        ));
        let mut aux_input_channel_pointers =
            Vec::with_capacity(self.audio_io_layout.aux_input_ports.len());
        for channel_count in self.audio_io_layout.aux_input_ports {
            aux_input_channel_pointers.push(ChannelPointerVec(Vec::with_capacity(
                channel_count.get() as usize,
            )));
        }
        let mut aux_output_channel_pointers =
            Vec::with_capacity(self.audio_io_layout.aux_output_ports.len());
        for channel_count in self.audio_io_layout.aux_output_ports {
            aux_output_channel_pointers.push(ChannelPointerVec(Vec::with_capacity(
                channel_count.get() as usize,
            )));
        }

        let mut midi_input_events = Vec::with_capacity(MIDI_EVENT_QUEUE_CAPACITY);
        let mut midi_output_events = Vec::with_capacity(MIDI_EVENT_QUEUE_CAPACITY);

        // Can't borrow from `self` in the callback
        let config = self.config.clone();
        let mut num_processed_samples = 0usize;
        move |data, _info| {
            let mut transport = Transport::new(config.sample_rate);
            transport.pos_samples = Some(num_processed_samples as i64);
            transport.tempo = Some(config.tempo as f64);
            transport.time_sig_numerator = Some(config.timesig_num as i32);
            transport.time_sig_denominator = Some(config.timesig_denom as i32);
            transport.playing = true;

            // If an input was configured, then the output buffer is filled with (interleaved) input
            // samples. Otherwise it gets filled with silence. There is no need to zero out any of
            // the other buffers. The `BufferManager` will copy the auxiliary input data to its own
            // storage buffers because it cannot assume that these buffers are safe to write to.
            // Because of that we'll never need to reinitialize these, and the output storage is
            // write-only (with `BufferManager` always zeroing them out when creating the buffers).
            match &mut input_rb_consumer {
                Some(input_rb_consumer) => {
                    for channel in main_io_storage.iter_mut() {
                        for sample in channel {
                            loop {
                                // Keep spinning on this if the output callback somehow outpaces the
                                // input callback
                                if let Ok(input_sample) = input_rb_consumer.pop() {
                                    *sample = input_sample;
                                    break;
                                }
                            }
                        }
                    }
                }
                None => {
                    for channel in main_io_storage.iter_mut() {
                        channel.fill(0.0);
                    }
                }
            }

            // Things may have been moved in between callbacks, so these pointers need to be set up
            // again on each invocation
            main_io_channel_pointers.get().clear();
            for channel in main_io_storage.iter_mut() {
                assert!(channel.len() == buffer_size);

                main_io_channel_pointers.get().push(channel.as_mut_ptr());
            }

            for (input_channel_pointers, input_storage) in aux_input_channel_pointers
                .iter_mut()
                .zip(aux_input_storage.iter_mut())
            {
                input_channel_pointers.get().clear();
                for channel in input_storage.iter_mut() {
                    assert!(channel.len() == buffer_size);

                    input_channel_pointers.get().push(channel.as_mut_ptr());
                }
            }

            for (output_channel_pointers, output_storage) in aux_output_channel_pointers
                .iter_mut()
                .zip(aux_output_storage.iter_mut())
            {
                output_channel_pointers.get().clear();
                for channel in output_storage.iter_mut() {
                    assert!(channel.len() == buffer_size);

                    output_channel_pointers.get().push(channel.as_mut_ptr());
                }
            }

            {
                // Even though we told CPAL that we wanted `buffer_size` samples, it may still give
                // us fewer. If we receive more than what we configured, then this will panic.
                let actual_sample_count = data.len() / num_output_channels;
                assert!(
                    actual_sample_count <= buffer_size,
                    "Received {actual_sample_count} samples, while the configured buffer size is \
                     {buffer_size}"
                );
                let buffers = unsafe {
                    buffer_manager.create_buffers(0, actual_sample_count, |buffer_sources| {
                        *buffer_sources.main_output_channel_pointers = Some(ChannelPointers {
                            ptrs: NonNull::new(main_io_channel_pointers.get().as_mut_ptr())
                                .unwrap(),
                            num_channels: main_io_channel_pointers.get().len(),
                        });
                        *buffer_sources.main_input_channel_pointers = Some(ChannelPointers {
                            ptrs: NonNull::new(main_io_channel_pointers.get().as_mut_ptr())
                                .unwrap(),
                            num_channels: num_input_channels
                                .min(main_io_channel_pointers.get().len()),
                        });

                        for (input_source_channel_pointers, input_channel_pointers) in
                            buffer_sources
                                .aux_input_channel_pointers
                                .iter_mut()
                                .zip(aux_input_channel_pointers.iter_mut())
                        {
                            *input_source_channel_pointers = Some(ChannelPointers {
                                ptrs: NonNull::new(input_channel_pointers.get().as_mut_ptr())
                                    .unwrap(),
                                num_channels: input_channel_pointers.get().len(),
                            });
                        }

                        for (output_source_channel_pointers, output_channel_pointers) in
                            buffer_sources
                                .aux_output_channel_pointers
                                .iter_mut()
                                .zip(aux_output_channel_pointers.iter_mut())
                        {
                            *output_source_channel_pointers = Some(ChannelPointers {
                                ptrs: NonNull::new(output_channel_pointers.get().as_mut_ptr())
                                    .unwrap(),
                                num_channels: output_channel_pointers.get().len(),
                            });
                        }
                    })
                };

                midi_input_events.clear();
                if let Some(input_event_rb_consumer) = &mut input_event_rb_consumer {
                    if let Ok(event) = input_event_rb_consumer.pop() {
                        midi_input_events.push(event);
                    }
                }

                midi_output_events.clear();
                let mut aux = AuxiliaryBuffers {
                    inputs: buffers.aux_inputs,
                    outputs: buffers.aux_outputs,
                };
                if !cb(
                    buffers.main_buffer,
                    &mut aux,
                    transport,
                    &midi_input_events,
                    &mut midi_output_events,
                ) {
                    // TODO: Some way to immediately terminate the stream here would be nice
                    unparker.unpark();
                    return;
                }
            }

            // The buffer's samples need to be written to `data` in an interlaced format
            // SAFETY: Dropping `buffers` allows us to borrow `main_io_storage` again
            for (i, output_sample) in data.iter_mut().enumerate() {
                let ch = i % num_output_channels;
                let n = i / num_output_channels;
                *output_sample = T::from_sample(main_io_storage[ch][n]);
            }

            if let Some(output_event_rb_producer) = &mut output_event_rb_producer {
                for event in midi_output_events.drain(..) {
                    if output_event_rb_producer
                        .try_send(MidiOutputTask::Send(event))
                        .is_err()
                    {
                        nih_error!("The MIDI output event queue was full, dropping event");
                        break;
                    }
                }
            }

            num_processed_samples += buffer_size;
        }
    }
}
