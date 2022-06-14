use std::time::{Duration, Instant};

use crate::buffer::Buffer;

use super::config::WrapperConfig;

/// An audio+MIDI backend for the standalone wrapper.
pub trait Backend: 'static + Send + Sync {
    /// Start processing audio and MIDI on this thread. The process callback will be called whenever
    /// there's a new block of audio to be processed. The process callback receives the audio
    /// buffers for the wrapped plugin's outputs. Any inputs will have already been copied to this
    /// buffer. This will block until the process callback returns `false`.
    ///
    /// TODO: MIDI
    fn run(&mut self, cb: impl FnMut(&mut Buffer) -> bool);
}

// /// Uses JACK audio and MIDI.
// pub struct Jack {
//     // TODO
// }

/// This backend doesn't input or output any audio or MIDI. It only exists so the standalone
/// application can continue to run even when there is no audio backend available. This can be
/// useful for testing plugin GUIs.
pub struct Dummy {
    config: WrapperConfig,
}

// TODO: Add a JACK backend
// impl Backend for Jack {
//     fn run(&mut self, cb: impl FnMut(&mut Buffer) -> bool) {
//         todo!()
//     }
// }

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
