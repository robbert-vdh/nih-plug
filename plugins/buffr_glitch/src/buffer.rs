// Buffr Glitch: a MIDI-controlled buffer repeater
// Copyright (C) 2022-2024 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use nih_plug::prelude::*;

use crate::MAX_OCTAVE_SHIFT;

/// A super simple ring buffer abstraction that records audio into a buffer until it is full, and
/// then starts looping the already recorded audio. The recording starts when pressing a key so
/// transients are preserved correctly. This needs to be able to store at least the number of
/// samples that correspond to the period size of MIDI note 0.
#[derive(Debug, Default)]
pub struct RingBuffer {
    sample_rate: f32,

    /// When a key is pressed, `next_sample_pos` is set to 0 and the incoming audio is recorded into
    /// this buffer until `next_sample_pos` wraps back around to the start of the ring buffer. At
    /// that point the incoming audio is replaced by the previously recorded audio. These buffers
    /// are resized to match the length/frequency of the audio being played back.
    audio_buffers: Vec<Vec<f32>>,
    /// The current playback position in `playback_buffers`.
    next_sample_pos: usize,
    /// The length of the crossfade, in samples. After the first this additional samples are
    /// recorded and faded back into the buffer.
    crossfade_length: usize,
    /// See [`BufferStatus`].
    buffer_status: BufferStatus,
}

#[derive(Debug, Default, Clone, Copy)]
enum BufferStatus {
    /// The buffer has not yet been filled and all sample should be recorded into the buffer.
    #[default]
    Recording,
    /// The buffer has wrapped around, but `crossfade_length` is set to 1 or more samples. This
    /// second pass continues recording, and replaces the buffer's start with a cross faded version
    /// of that input and the existing contents.
    Crossfading,
    /// The buffer has wrapped around once and `crossfade_length` is set to 0, or it has wrapped
    /// around twice and crossfading is enabled. Samples only need to be read from the buffer, all
    /// work is done.
    Ready,
}

impl RingBuffer {
    /// Initialize or resize the buffers to fit a certain number of channels and samples. The inner
    /// buffer capacity is determined by the number of samples it takes to represent the period of
    /// MIDI note 0 at the specified sample rate, rounded up to a power of two. Make sure to call
    /// [`reset()`][Self::reset()] after this.
    pub fn resize(&mut self, num_channels: usize, sample_rate: f32) {
        nih_debug_assert!(num_channels >= 1);
        nih_debug_assert!(sample_rate > 0.0);

        // NOTE: We need to take the octave shift into account
        let lowest_note_frequency =
            util::midi_note_to_freq(0) / 2.0f32.powi(MAX_OCTAVE_SHIFT as i32);
        let loest_note_period_samples =
            (lowest_note_frequency.recip() * sample_rate).ceil() as usize;
        let buffer_len = loest_note_period_samples.next_power_of_two();

        // Used later to compute period sizes in samples based on frequencies
        self.sample_rate = sample_rate;

        self.audio_buffers.resize_with(num_channels, Vec::new);
        for buffer in self.audio_buffers.iter_mut() {
            buffer.resize(buffer_len, 0.0);
        }
    }

    /// Zero out the buffers.
    pub fn reset(&mut self) {
        // The current verion's buffers don't need to be reset since they're always initialized
        // before being used
    }

    /// Prepare the playback buffers to play back audio at the specified frequency. This resets the
    /// buffer to record the next `note_period_samples`, which are then looped until the key is
    /// released. The crossfade length is also set at this point since right now we don't record
    /// more than necessary and can't change this afterwards.
    pub fn prepare_playback(&mut self, frequency: f32, crossfade_ms: f32) {
        nih_debug_assert!(frequency > 0.0);
        nih_debug_assert!(crossfade_ms >= 0.0);
        let note_period_samples = (frequency.recip() * self.sample_rate).ceil() as usize;

        // This buffer doesn't need to be cleared since the data is not read until the entire buffer
        // has been recorded to
        nih_debug_assert!(note_period_samples <= self.audio_buffers[0].capacity());
        for buffer in self.audio_buffers.iter_mut() {
            buffer.resize(note_period_samples, 0.0);
        }

        // The buffer is filled on the first `note_period_samples` calls to `next_sample`, plus a
        // little more for the crossfade if set
        self.next_sample_pos = 0;
        self.crossfade_length =
            ((crossfade_ms * self.sample_rate).ceil() as usize).min(note_period_samples);
        self.buffer_status = BufferStatus::Recording;
    }

    /// Read or write a sample from or to the ring buffer, and return the output. On the first loop
    /// this will store the input samples into the bufffer and return the input value as is.
    /// Afterwards it will read the previously recorded data from the buffer. The read/write
    /// position is advanced whenever the last channel is written to.
    pub fn next_sample(&mut self, channel_idx: usize, input_sample: f32) -> f32 {
        match self.buffer_status {
            BufferStatus::Recording => {
                self.audio_buffers[channel_idx][self.next_sample_pos] = input_sample
            }
            BufferStatus::Crossfading if self.next_sample_pos < self.crossfade_length => {
                // This is an equal power fade between the part of the input after the first loop
                // and the buffer's existing contents. The `.max(1)` is needed to avoid NaNs with
                // crossfade lengths of 1 sample.
                let crossfade_t =
                    self.next_sample_pos as f32 / (self.crossfade_length - 1).max(1) as f32;
                let new_t = (1.0 - crossfade_t).sqrt();
                let existing_t = crossfade_t.sqrt();

                self.audio_buffers[channel_idx][self.next_sample_pos] = (input_sample * new_t)
                    + (self.audio_buffers[channel_idx][self.next_sample_pos] * existing_t);
            }
            _ => (),
        }
        let result = self.audio_buffers[channel_idx][self.next_sample_pos];

        // TODO: This can be done more efficiently, but you really won't notice the performance
        //       impact here
        if channel_idx == self.audio_buffers.len() - 1 {
            self.next_sample_pos += 1;

            if self.next_sample_pos == self.audio_buffers[0].len() {
                self.next_sample_pos = 0;

                self.buffer_status = match self.buffer_status {
                    BufferStatus::Recording if self.crossfade_length > 0 => {
                        BufferStatus::Crossfading
                    }
                    _ => BufferStatus::Ready,
                };
            }
        }

        result
    }
}
