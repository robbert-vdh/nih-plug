// Buffr Glitch: a MIDI-controlled buffer repeater
// Copyright (C) 2022-2023 Robbert van der Helm
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

use crate::{NormalizationMode, MAX_OCTAVE_SHIFT};

/// A super simple ring buffer abstraction that records audio into a recording ring buffer, and then
/// copies audio to a playback buffer when a note is pressed so audio can be repeated while still
/// recording audio for further key presses. This needs to be able to store at least the number of
/// samples that correspond to the period size of MIDI note 0.
#[derive(Debug, Default)]
pub struct RingBuffer {
    sample_rate: f32,

    /// Sample ring buffers indexed by channel and sample index. These are always recorded to.
    recording_buffers: Vec<Vec<f32>>,
    /// The positions within the sample buffers the next sample should be written to. Since all
    /// channels will be written to in lockstep we only need a single value here. It's incremented
    /// when writing a sample for the last channel.
    next_write_pos: usize,

    /// When a key is pressed, audio gets copied from `recording_buffers` to these buffers so it can
    /// be played back without interrupting the recording process. These buffers are resized to
    /// match the length of the audio being played back.
    playback_buffers: Vec<Vec<f32>>,
    /// The current playback position in `playback_buffers`.
    playback_buffer_pos: usize,
}

impl RingBuffer {
    /// Initialize or resize the buffers to fit a certain number of channels and samples. The inner
    /// buffer capacity is determined by the number of samples it takes to represent the period of
    /// MIDI note 0 at the specified sample rate, rounded up to a power of two. Make sure to call
    /// [`reset()`][Self::reset()] after this.
    pub fn resize(&mut self, num_channels: usize, sample_rate: f32) {
        // NOTE: We need to take the octave shift into account
        let lowest_note_frequency =
            util::midi_note_to_freq(0) / 2.0f32.powi(MAX_OCTAVE_SHIFT as i32);
        let loest_note_period_samples =
            (lowest_note_frequency.recip() * sample_rate).ceil() as usize;
        let buffer_len = loest_note_period_samples.next_power_of_two();

        // Used later to compute period sizes in samples based on frequencies
        self.sample_rate = sample_rate;

        self.recording_buffers.resize_with(num_channels, Vec::new);
        for buffer in self.recording_buffers.iter_mut() {
            buffer.resize(buffer_len, 0.0);
        }

        self.playback_buffers.resize_with(num_channels, Vec::new);
        for buffer in self.playback_buffers.iter_mut() {
            buffer.resize(buffer_len, 0.0);
            // We need to reserve capacity for the playback buffers, but they're initially empty
            buffer.resize(0, 0.0);
        }
    }

    /// Zero out the buffers.
    pub fn reset(&mut self) {
        for buffer in self.recording_buffers.iter_mut() {
            buffer.fill(0.0);
        }
        self.next_write_pos = 0;

        // The playback buffers don't need to be reset since they're always initialized before being
        // used
    }

    /// Push a sample to the buffer. The write position is advanced whenever the last channel is
    /// written to.
    pub fn push(&mut self, channel_idx: usize, sample: f32) {
        self.recording_buffers[channel_idx][self.next_write_pos] = sample;

        // TODO: This can be done more efficiently, but you really won't notice the performance
        //       impact here
        if channel_idx == self.recording_buffers.len() - 1 {
            self.next_write_pos += 1;

            if self.next_write_pos == self.recording_buffers[0].len() {
                self.next_write_pos = 0;
            }
        }
    }

    /// Prepare the playback buffers to play back audio at the specified frequency. This copies
    /// audio from the ring buffers to the playback buffers.
    pub fn prepare_playback(&mut self, frequency: f32, normalization_mode: NormalizationMode) {
        let note_period_samples = (frequency.recip() * self.sample_rate).ceil() as usize;

        // We'll copy the last `note_period_samples` samples from the recording ring buffers to the
        // playback buffers
        nih_debug_assert!(note_period_samples <= self.playback_buffers[0].capacity());
        for (playback_buffer, recording_buffer) in self
            .playback_buffers
            .iter_mut()
            .zip(self.recording_buffers.iter())
        {
            playback_buffer.resize(note_period_samples, 0.0);

            // Keep in mind we'll need to go `note_period_samples` samples backwards in the
            // recording buffer
            let copy_num_from_start = usize::min(note_period_samples, self.next_write_pos);
            let copy_num_from_end = note_period_samples - copy_num_from_start;
            playback_buffer[0..copy_num_from_end]
                .copy_from_slice(&recording_buffer[recording_buffer.len() - copy_num_from_end..]);
            playback_buffer[copy_num_from_end..].copy_from_slice(
                &recording_buffer[self.next_write_pos - copy_num_from_start..self.next_write_pos],
            );
        }

        // The playback buffer is normalized as necessary. This prevents small grains from being
        // either way quieter or way louder than the origianl audio.
        match normalization_mode {
            NormalizationMode::None => (),
            NormalizationMode::Auto => {
                // Prevent this from causing divisions by zero or making very loud clicks when audio
                // playback has just started
                let playback_rms = calculate_rms(&self.playback_buffers);
                if playback_rms > 0.001 {
                    let recording_rms = calculate_rms(&self.recording_buffers);
                    let normalization_factor = recording_rms / playback_rms;

                    for buffer in self.playback_buffers.iter_mut() {
                        for sample in buffer.iter_mut() {
                            *sample *= normalization_factor;
                        }
                    }
                }
            }
        }

        // Reading from the buffer should always start at the beginning
        self.playback_buffer_pos = 0;
    }

    /// Return a sample from the playback buffer. The playback position is advanced whenever the
    /// last channel is written to. When the playback position reaches the end of the buffer it
    /// wraps around.
    pub fn next_playback_sample(&mut self, channel_idx: usize) -> f32 {
        let sample = self.playback_buffers[channel_idx][self.playback_buffer_pos];

        // TODO: Same as the above
        if channel_idx == self.playback_buffers.len() - 1 {
            self.playback_buffer_pos += 1;

            if self.playback_buffer_pos == self.playback_buffers[0].len() {
                self.playback_buffer_pos = 0;
            }
        }

        sample
    }
}

/// Get the RMS value of an entire buffer. This is used for (automatic) normalization.
///
/// # Panics
///
/// This will panic of `buffers` is empty.
fn calculate_rms(buffers: &[Vec<f32>]) -> f32 {
    nih_debug_assert_ne!(buffers.len(), 0);

    let sum_of_squares: f32 = buffers
        .iter()
        .map(|buffer| buffer.iter().map(|sample| (sample * sample)).sum::<f32>())
        .sum();
    let num_samples = buffers.len() * buffers[0].len();

    (sum_of_squares / num_samples as f32).sqrt()
}
