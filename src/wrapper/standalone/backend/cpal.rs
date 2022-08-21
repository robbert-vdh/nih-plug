use anyhow::{Context, Result};
use cpal::{traits::*, Device, OutputCallbackInfo, Sample, SampleFormat, StreamConfig};
use crossbeam::sync::{Parker, Unparker};

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
        // below

        // This thread needs to be blocked until audio processing ends as CPAL processes the streams
        // on another thread instead of blocking
        // TODO: Move this to the output stream handling
        // TODO: Input stream
        // TODO: Block the main thread until this breaky thing
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
                self.build_output_data_callback::<i16>(unparker, cb),
                error_cb,
            ),
            SampleFormat::U16 => self.output_device.build_output_stream(
                &self.output_config,
                self.build_output_data_callback::<u16>(unparker, cb),
                error_cb,
            ),
            SampleFormat::F32 => self.output_device.build_output_stream(
                &self.output_config,
                self.build_output_data_callback::<f32>(unparker, cb),
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

    fn build_output_data_callback<T: Sample>(
        &self,
        unparker: Unparker,
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
        let mut midi_input_events = Vec::with_capacity(1024);
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

            for channel in buffer.as_slice() {
                channel.fill(0.0);
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
