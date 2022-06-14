use std::time::{Duration, Instant};

use super::super::config::WrapperConfig;
use super::Backend;
use crate::buffer::Buffer;

/// This backend doesn't input or output any audio or MIDI. It only exists so the standalone
/// application can continue to run even when there is no audio backend available. This can be
/// useful for testing plugin GUIs.
pub struct Dummy {
    config: WrapperConfig,
}

impl Backend for Dummy {
    fn run(&mut self, mut cb: impl FnMut(&mut Buffer) -> bool) {
        // We can't really do anything meaningful here, so we'll simply periodically call the
        // callback with empty buffers.
        let interval =
            Duration::from_secs_f32(self.config.period_size as f32 / self.config.sample_rate);

        let mut channels = vec![
            vec![0.0f32; self.config.period_size as usize];
            self.config.output_channels as usize
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

        loop {
            let period_start = Instant::now();

            for channel in buffer.as_slice() {
                channel.fill(0.0);
            }

            if !cb(&mut buffer) {
                break;
            }

            let period_end = Instant::now();
            std::thread::sleep((period_start + interval).saturating_duration_since(period_end));
        }
    }
}

impl Dummy {
    pub fn new(config: WrapperConfig) -> Self {
        Self { config }
    }
}
