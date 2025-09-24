use std::num::NonZeroU32;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use super::super::config::WrapperConfig;
use super::Backend;
use crate::prelude::{AudioIOLayout, AuxiliaryBuffers, Buffer, Plugin, PluginNoteEvent, Transport};
use crate::wrapper::util::buffer_management::{BufferManager, ChannelPointers};

/// This backend doesn't input or output any audio or MIDI. It only exists so the standalone
/// application can continue to run even when there is no audio backend available. This can be
/// useful for testing plugin GUIs.
pub struct Dummy {
    config: WrapperConfig,
    audio_io_layout: AudioIOLayout,
}

impl<P: Plugin> Backend<P> for Dummy {
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
        // We can't really do anything meaningful here, so we'll simply periodically call the
        // callback with empty buffers
        let interval =
            Duration::from_secs_f32(self.config.period_size as f32 / self.config.sample_rate);

        let num_samples = self.config.period_size as usize;
        let num_output_channels = self
            .audio_io_layout
            .main_output_channels
            .map(NonZeroU32::get)
            .unwrap_or_default() as usize;
        let num_input_channels = self
            .audio_io_layout
            .main_input_channels
            .map(NonZeroU32::get)
            .unwrap_or_default() as usize;
        let mut main_io_storage = vec![vec![0.0f32; num_samples]; num_output_channels];

        // We'll do the same thing for auxiliary inputs and outputs, so the plugin always gets the
        // buffers it expects
        let mut aux_input_storage: Vec<Vec<Vec<f32>>> = Vec::new();
        for channel_count in self.audio_io_layout.aux_input_ports {
            aux_input_storage.push(vec![
                vec![0.0f32; num_samples];
                channel_count.get() as usize
            ]);
        }

        let mut aux_output_storage: Vec<Vec<Vec<f32>>> = Vec::new();
        for channel_count in self.audio_io_layout.aux_output_ports {
            aux_output_storage.push(vec![
                vec![0.0f32; num_samples];
                channel_count.get() as usize
            ]);
        }

        // We need pointers to this storage to emulate the API used by plugins
        let mut main_io_channel_pointers: Vec<*mut f32> = main_io_storage
            .iter_mut()
            .map(|channel_slice| channel_slice.as_mut_ptr())
            .collect();
        let mut aux_input_channel_pointers: Vec<Vec<*mut f32>> = aux_input_storage
            .iter_mut()
            .map(|aux_input_storage| {
                aux_input_storage
                    .iter_mut()
                    .map(|channel_slice| channel_slice.as_mut_ptr())
                    .collect()
            })
            .collect();
        let mut aux_output_channel_pointers: Vec<Vec<*mut f32>> = aux_output_storage
            .iter_mut()
            .map(|aux_output_storage| {
                aux_output_storage
                    .iter_mut()
                    .map(|channel_slice| channel_slice.as_mut_ptr())
                    .collect()
            })
            .collect();

        // The `BufferManager` can then manage buffers using this storage just like in every other
        // backend
        let mut buffer_manager =
            BufferManager::for_audio_io_layout(num_samples, self.audio_io_layout);

        // This queue will never actually be used
        let mut midi_output_events = Vec::with_capacity(1024);
        let mut num_processed_samples = 0usize;
        loop {
            let period_start = Instant::now();

            let mut transport = Transport::new(self.config.sample_rate);
            transport.pos_samples = Some(num_processed_samples as i64);
            transport.tempo = Some(self.config.tempo as f64);
            transport.time_sig_numerator = Some(self.config.timesig_num as i32);
            transport.time_sig_denominator = Some(self.config.timesig_denom as i32);
            transport.playing = true;

            for channel in &mut main_io_storage {
                channel.fill(0.0);
            }
            for aux_buffer in &mut aux_input_storage {
                for channel in aux_buffer {
                    channel.fill(0.0);
                }
            }
            for aux_buffer in &mut aux_output_storage {
                for channel in aux_buffer {
                    channel.fill(0.0);
                }
            }

            let buffers = unsafe {
                buffer_manager.create_buffers(0, num_samples, |buffer_sources| {
                    *buffer_sources.main_output_channel_pointers = Some(ChannelPointers {
                        ptrs: NonNull::new(main_io_channel_pointers.as_mut_ptr()).unwrap(),
                        num_channels: main_io_channel_pointers.len(),
                    });
                    *buffer_sources.main_input_channel_pointers = Some(ChannelPointers {
                        ptrs: NonNull::new(main_io_channel_pointers.as_mut_ptr()).unwrap(),
                        num_channels: num_input_channels.min(main_io_channel_pointers.len()),
                    });

                    for (input_source_channel_pointers, input_channel_pointers) in buffer_sources
                        .aux_input_channel_pointers
                        .iter_mut()
                        .zip(aux_input_channel_pointers.iter_mut())
                    {
                        *input_source_channel_pointers = Some(ChannelPointers {
                            ptrs: NonNull::new(input_channel_pointers.as_mut_ptr()).unwrap(),
                            num_channels: input_channel_pointers.len(),
                        });
                    }

                    for (output_source_channel_pointers, output_channel_pointers) in buffer_sources
                        .aux_output_channel_pointers
                        .iter_mut()
                        .zip(aux_output_channel_pointers.iter_mut())
                    {
                        *output_source_channel_pointers = Some(ChannelPointers {
                            ptrs: NonNull::new(output_channel_pointers.as_mut_ptr()).unwrap(),
                            num_channels: output_channel_pointers.len(),
                        });
                    }
                })
            };

            midi_output_events.clear();
            let mut aux = AuxiliaryBuffers {
                inputs: buffers.aux_inputs,
                outputs: buffers.aux_outputs,
            };
            if !cb(
                buffers.main_buffer,
                &mut aux,
                transport,
                &[],
                &mut midi_output_events,
            ) {
                break;
            }

            num_processed_samples += num_samples;

            let period_end = Instant::now();
            std::thread::sleep((period_start + interval).saturating_duration_since(period_end));
        }
    }
}

impl Dummy {
    pub fn new<P: Plugin>(config: WrapperConfig) -> Self {
        Self {
            audio_io_layout: config.audio_io_layout_or_exit::<P>(),
            config,
        }
    }
}
