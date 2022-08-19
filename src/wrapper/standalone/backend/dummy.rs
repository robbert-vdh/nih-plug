use std::time::{Duration, Instant};

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;
use crate::context::Transport;
use crate::midi::NoteEvent;
use crate::plugin::{AuxiliaryIOConfig, BusConfig, Plugin};

/// This backend doesn't input or output any audio or MIDI. It only exists so the standalone
/// application can continue to run even when there is no audio backend available. This can be
/// useful for testing plugin GUIs.
pub struct Dummy {
    config: WrapperConfig,
    bus_config: BusConfig,
}

impl Backend for Dummy {
    fn run(
        &mut self,
        mut cb: impl FnMut(&mut Buffer, Transport, &[NoteEvent], &mut Vec<NoteEvent>) -> bool
            + 'static
            + Send,
    ) {
        // We can't really do anything meaningful here, so we'll simply periodically call the
        // callback with empty buffers.
        let interval =
            Duration::from_secs_f32(self.config.period_size as f32 / self.config.sample_rate);

        let mut channels = vec![
            vec![0.0f32; self.config.period_size as usize];
            self.bus_config.num_output_channels as usize
        ];
        let mut buffer = Buffer::default();
        unsafe {
            buffer.with_raw_vec(|output_slices| {
                // SAFETY: `channels` is no longer used directly after this
                *output_slices = channels
                    .iter_mut()
                    .map(|channel| &mut *(channel.as_mut_slice() as *mut [f32]))
                    .collect();
            })
        }

        // This queue will never actually be used
        let mut midi_output_events = Vec::with_capacity(1024);
        let mut num_processed_samples = 0;
        loop {
            let period_start = Instant::now();

            let mut transport = Transport::new(self.config.sample_rate);
            transport.pos_samples = Some(num_processed_samples);
            transport.tempo = Some(self.config.tempo as f64);
            transport.time_sig_numerator = Some(self.config.timesig_num as i32);
            transport.time_sig_denominator = Some(self.config.timesig_denom as i32);
            transport.playing = true;

            for channel in buffer.as_slice() {
                channel.fill(0.0);
            }

            midi_output_events.clear();
            if !cb(&mut buffer, transport, &[], &mut midi_output_events) {
                break;
            }

            num_processed_samples += buffer.len() as i64;

            let period_end = Instant::now();
            std::thread::sleep((period_start + interval).saturating_duration_since(period_end));
        }
    }
}

impl Dummy {
    pub fn new<P: Plugin>(config: WrapperConfig) -> Self {
        Self {
            bus_config: BusConfig {
                num_input_channels: config.input_channels.unwrap_or(P::DEFAULT_INPUT_CHANNELS),
                num_output_channels: config.output_channels.unwrap_or(P::DEFAULT_OUTPUT_CHANNELS),
                // TODO: Support these in the standalone
                aux_input_busses: AuxiliaryIOConfig::default(),
                aux_output_busses: AuxiliaryIOConfig::default(),
            },
            config,
        }
    }
}
