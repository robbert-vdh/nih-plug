use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use super::super::config::WrapperConfig;
use super::Backend;
use crate::audio_setup::AudioIOLayout;
use crate::buffer::Buffer;
use crate::context::process::Transport;
use crate::midi::PluginNoteEvent;
use crate::plugin::Plugin;

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
                Transport,
                &[PluginNoteEvent<P>],
                &mut Vec<PluginNoteEvent<P>>,
            ) -> bool
            + 'static
            + Send,
    ) {
        // We can't really do anything meaningful here, so we'll simply periodically call the
        // callback with empty buffers.
        let interval =
            Duration::from_secs_f32(self.config.period_size as f32 / self.config.sample_rate);

        let num_output_channels = self
            .audio_io_layout
            .main_output_channels
            .map(NonZeroU32::get)
            .unwrap_or_default() as usize;
        let mut channels =
            vec![vec![0.0f32; self.config.period_size as usize]; num_output_channels];
        let mut buffer = Buffer::default();
        unsafe {
            buffer.set_slices(self.config.period_size as usize, |output_slices| {
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

            num_processed_samples += buffer.samples() as i64;

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
