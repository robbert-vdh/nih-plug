// Buffr Glitch: a MIDI-controlled buffer repeater
// Copyright (C) 2022 Robbert van der Helm
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

use nih_plug::prelude::util;

/// A super simple ring buffer abstraction to store the last received audio. This needs to be able
/// to store at least the number of samples that correspond to the period size of MIDI note 0.
#[derive(Debug, Default)]
pub struct RingBuffer {
    /// Sample buffers indexed by channel and sample index.
    buffers: Vec<Vec<f32>>,
    /// The positions within the sample buffers the next sample should be written to. Since all
    /// channels will be written to in lockstep we only need a single value here. It's incremented
    /// when writing a sample for the last channel.
    next_write_pos: usize,
}

impl RingBuffer {
    /// Initialize or resize the buffers to fit a certain number of channels and samples. The inner
    /// buffer capacity is determined by the number of samples it takes to represent the period of
    /// MIDI note 0 at the specified sample rate, rounded up to a power of two. Make sure to call
    /// [`reset()`][Self::reset()] after this.
    pub fn resize(&mut self, num_channels: usize, sample_rate: f32) {
        let note_frequency = util::midi_note_to_freq(0);
        let note_period_samples = (note_frequency.recip() * sample_rate).ceil() as usize;
        let buffer_len = note_period_samples.next_power_of_two();

        self.buffers.resize_with(num_channels, Vec::new);
        for buffer in self.buffers.iter_mut() {
            buffer.resize(buffer_len, 0.0);
        }
    }

    /// Zero out the buffers.
    pub fn reset(&mut self) {
        for buffer in self.buffers.iter_mut() {
            buffer.fill(0.0);
        }

        self.next_write_pos = 0;
    }

    /// Push a sample to the buffer. The write position is advanced whenever the last channel is
    /// written to.
    pub fn push(&mut self, channel_idx: usize, sample: f32) {
        self.buffers[channel_idx][self.next_write_pos] = sample;

        // TODO: This can be done more efficiently, but you really won't notice the performance
        //       impact here
        if channel_idx == self.buffers.len() - 1 {
            self.next_write_pos += 1;

            if self.next_write_pos == self.buffers[0].len() {
                self.next_write_pos = 0;
            }
        }
    }
}
