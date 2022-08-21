use anyhow::{Context, Result};
use cpal::{
    traits::*, Device, InputCallbackInfo, OutputCallbackInfo, Sample, SampleFormat, Stream,
    StreamConfig,
};
use crossbeam::sync::{Parker, Unparker};
use rtrb::RingBuffer;

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;
use crate::context::Transport;
use crate::midi::NoteEvent;
use crate::plugin::{AuxiliaryIOConfig, BusConfig, Plugin};

/// Uses CPAL for audio and midir for MIDI.
pub struct Cpal {
    config: WrapperConfig,
    bus_config: BusConfig,

    input: Option<(Device, StreamConfig, SampleFormat)>,

    output_device: Device,
    output_config: StreamConfig,
    output_sample_format: SampleFormat,
    // TODO: MIDI
}

impl Backend for Cpal {
    fn run(
        &mut self,
        cb: impl FnMut(&mut Buffer, Transport, &[NoteEvent], &mut Vec<NoteEvent>) -> bool
            + 'static
            + Send,
    ) {
        // The CPAL audio devices may not accept floating point samples, so all of the actual audio
        // handling and buffer management handles in the `build_*_data_callback()` functions defined
        // below.

        // CPAL does not support duplex streams, so audio input (when enabled, inputs aren't
        // connected by default) waits a read a period of data before starting the output stream
        let mut _input_stream: Option<Stream> = None;
        let mut input_rb_consumer: Option<rtrb::Consumer<f32>> = None;
        if let Some((input_device, input_config, input_sample_format)) = &self.input {
            // Data is sent to the output data callback using a wait-free ring buffer
            let (rb_producer, rb_consumer) = RingBuffer::new(
                self.output_config.channels as usize * self.config.period_size as usize,
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

            let stream = match input_sample_format {
                SampleFormat::I16 => input_device.build_input_stream(
                    input_config,
                    self.build_input_data_callback::<i16>(input_unparker, rb_producer),
                    error_cb,
                ),
                SampleFormat::U16 => input_device.build_input_stream(
                    input_config,
                    self.build_input_data_callback::<u16>(input_unparker, rb_producer),
                    error_cb,
                ),
                SampleFormat::F32 => input_device.build_input_stream(
                    input_config,
                    self.build_input_data_callback::<f32>(input_unparker, rb_producer),
                    error_cb,
                ),
            }
            .expect("Fatal error creating the capture stream");
            stream
                .play()
                .expect("Fatal error trying to start the capture stream");
            _input_stream = Some(stream);

            // Playback is delayed one period if we're capturing audio so it has something to process
            input_parker.park()
        }

        // This thread needs to be blocked until audio processing ends as CPAL processes the streams
        // on another thread instead of blocking
        let parker = Parker::new();
        let unparker = parker.unparker().clone();
        let error_cb = {
            let unparker = unparker.clone();
            move |err| {
                nih_error!("Error during playback: {err:#}");
                unparker.clone().unpark();
            }
        };

        let output_stream = match self.output_sample_format {
            SampleFormat::I16 => self.output_device.build_output_stream(
                &self.output_config,
                self.build_output_data_callback::<i16>(unparker, input_rb_consumer, cb),
                error_cb,
            ),
            SampleFormat::U16 => self.output_device.build_output_stream(
                &self.output_config,
                self.build_output_data_callback::<u16>(unparker, input_rb_consumer, cb),
                error_cb,
            ),
            SampleFormat::F32 => self.output_device.build_output_stream(
                &self.output_config,
                self.build_output_data_callback::<f32>(unparker, input_rb_consumer, cb),
                error_cb,
            ),
        }
        .expect("Fatal error creating the output stream");

        // TODO: Wait a period before doing this when also reading the input
        output_stream
            .play()
            .expect("Fatal error trying to start the output stream");

        // Wait for the audio thread to exit
        parker.park();
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

        let bus_config = BusConfig {
            num_input_channels: config.input_channels.unwrap_or(P::DEFAULT_INPUT_CHANNELS),
            num_output_channels: config.output_channels.unwrap_or(P::DEFAULT_OUTPUT_CHANNELS),
            // TODO: Support these in the standalone
            aux_input_busses: AuxiliaryIOConfig::default(),
            aux_output_busses: AuxiliaryIOConfig::default(),
        };
        let requested_sample_rate = cpal::SampleRate(config.sample_rate as u32);
        let requested_buffer_size = cpal::BufferSize::Fixed(config.period_size);

        let input = input_device
            .map(|device| -> Result<(Device, StreamConfig, SampleFormat)> {
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
                let input_config = StreamConfig {
                    channels: input_config_range.channels(),
                    sample_rate: requested_sample_rate,
                    buffer_size: requested_buffer_size.clone(),
                };
                let input_sample_format = input_config_range.sample_format();

                Ok((device, input_config, input_sample_format))
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
        let output_sample_format = output_config_range.sample_format();

        Ok(Cpal {
            config,
            bus_config,

            input,

            output_device,
            output_config,
            output_sample_format,
        })
    }

    fn build_input_data_callback<T: Sample>(
        &self,
        input_unparker: Unparker,
        mut input_rb_producer: rtrb::Producer<f32>,
    ) -> impl FnMut(&[T], &InputCallbackInfo) + Send + 'static {
        // This callback needs to copy input samples to a ring buffer that can be read from in the
        // output data callback
        move |data, _info| {
            for sample in data {
                // If for whatever reason the input callback is fired twice before an output
                // callback, then just spin on this until the push succeeds
                while input_rb_producer.push(sample.to_f32()).is_err() {}
            }

            // The run function is blocked until a single period has been processed here. After this
            // point output playback can start.
            input_unparker.unpark();
        }
    }

    fn build_output_data_callback<T: Sample>(
        &self,
        unparker: Unparker,
        mut input_rb_consumer: Option<rtrb::Consumer<f32>>,
        mut cb: impl FnMut(&mut Buffer, Transport, &[NoteEvent], &mut Vec<NoteEvent>) -> bool
            + 'static
            + Send,
    ) -> impl FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static {
        // We'll receive interlaced input samples from CPAL. These need to converted to deinterlaced
        // channels, processed, and then copied those back to an interlaced buffer for the output.
        // This needs to be wrapped in a struct like this and boxed because the `channels` vectors
        // need to live just as long as `buffer` when they get moved into the closure. FIXME: This
        // is pretty nasty, come up with a cleaner alternative
        let mut channels = vec![
            vec![0.0f32; self.config.period_size as usize];
            self.bus_config.num_output_channels as usize
        ];
        let mut buffer = Buffer::default();
        unsafe {
            buffer.with_raw_vec(|output_slices| {
                // Pre-allocate enough storage, the pointers are set in the data callback because
                // `channels` will have been moved between now and the next callback
                output_slices.resize_with(channels.len(), || &mut []);
            })
        }

        // TODO: MIDI input and output
        let midi_input_events = Vec::with_capacity(1024);
        let mut midi_output_events = Vec::with_capacity(1024);

        // Can't borrow from `self` in the callback
        let config = self.config.clone();
        let mut num_processed_samples = 0;

        move |data, _info| {
            // Things may have been moved in between callbacks, so these pointers need to be set up
            // agian on each invocation
            unsafe {
                buffer.with_raw_vec(|output_slices| {
                    for (output_slice, channel) in output_slices.iter_mut().zip(channels.iter_mut())
                    {
                        // SAFETY: `channels` is no longer used directly after this, and it outlives
                        // the data closure
                        *output_slice = &mut *(channel.as_mut_slice() as *mut [f32]);
                    }
                })
            }

            let mut transport = Transport::new(config.sample_rate);
            transport.pos_samples = Some(num_processed_samples);
            transport.tempo = Some(config.tempo as f64);
            transport.time_sig_numerator = Some(config.timesig_num as i32);
            transport.time_sig_denominator = Some(config.timesig_denom as i32);
            transport.playing = true;

            // If an input was configured, then the output buffer is filled with (interleaved) input
            // samples. Otherwise it gets filled with silence.
            match &mut input_rb_consumer {
                Some(input_rb_consumer) => {
                    for channels in buffer.iter_samples() {
                        for sample in channels {
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
                    for channel in buffer.as_slice() {
                        channel.fill(0.0);
                    }
                }
            }

            midi_output_events.clear();
            if !cb(
                &mut buffer,
                transport,
                &midi_input_events,
                &mut midi_output_events,
            ) {
                // TODO: Some way to immediately terminate the stream here would be nice
                unparker.unpark();
                return;
            }

            // The buffer's samples need to be written to `data` in an interlaced format
            for (output_sample, buffer_sample) in data.iter_mut().zip(
                buffer
                    .iter_samples()
                    .flat_map(|channels| channels.into_iter()),
            ) {
                *output_sample = T::from(buffer_sample);
            }

            // TODO: Handle MIDI output events

            num_processed_samples += buffer.len() as i64;
        }
    }
}
