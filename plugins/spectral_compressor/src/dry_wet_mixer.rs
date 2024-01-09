// Spectral Compressor: an FFT based compressor
// Copyright (C) 2021-2024 Robbert van der Helm
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

use nih_plug::prelude::Buffer;

/// A simple dry-wet mixer with latency compensation that operates on entire buffers.
pub struct DryWetMixer {
    /// The delay line for the latency compensation. This is indexed by `[channel_idx][sample_idx]`,
    /// with the size set to the maximum latency plus the maximum block size rounded up to the next
    /// power of two.
    delay_line: Vec<Vec<f32>>,
    /// The position in the inner delay line buffer where the next samples should be written from.
    /// This is incremented after writing. When reading the data for mixing the dry signal back in,
    /// the starting read position is determined by subtracting the buffer's length from this
    /// position and then subtracting the latency.
    next_write_position: usize,
}

/// The mixing style for the [`DryWetMixer`].
#[derive(Debug, Clone, Copy)]
#[allow(unused)]
pub enum MixingStyle {
    Linear,
    EqualPower,
}

impl DryWetMixer {
    /// Set up the mixer for the given parameters.
    pub fn new(num_channels: usize, max_block_size: usize, max_latency: usize) -> Self {
        // TODO: This could be more efficient if we don't use the entire buffer when the actual
        //       latency is lower than the maximum latency, but that's an optimization for later
        let delay_line_len = (max_block_size + max_latency).next_power_of_two();

        DryWetMixer {
            delay_line: vec![vec![0.0; delay_line_len]; num_channels],
            next_write_position: 0,
        }
    }

    /// Resize the internal buffers to fit new parameters.
    pub fn resize(&mut self, num_channels: usize, max_block_size: usize, max_latency: usize) {
        let delay_line_len = (max_block_size + max_latency).next_power_of_two();

        self.delay_line.resize_with(num_channels, Vec::new);
        for buffer in &mut self.delay_line {
            buffer.resize(delay_line_len, 0.0);
            buffer.fill(0.0);
        }
        self.next_write_position = 0;
    }

    /// Clear out the buffers.
    pub fn reset(&mut self) {
        for buffer in &mut self.delay_line {
            buffer.fill(0.0);
        }
        self.next_write_position = 0;
    }

    /// Write the dry signal into the buffer. This should be called at the start of the process
    /// function.
    ///
    /// # Panics
    ///
    /// Panics if the buffer is larger than the maximum block size or if the channel counts don't
    /// match.
    pub fn write_dry(&mut self, buffer: &Buffer) {
        if buffer.channels() == 0 {
            return;
        }

        assert_eq!(buffer.channels(), self.delay_line.len());
        let delay_line_len = self.delay_line[0].len();
        assert!(buffer.samples() <= delay_line_len);

        let num_samples_before_wrap = buffer
            .samples()
            .min(delay_line_len - self.next_write_position);
        let num_samples_after_wrap = buffer.samples() - num_samples_before_wrap;

        for (buffer_channel, delay_line) in buffer
            .as_slice_immutable()
            .iter()
            .zip(self.delay_line.iter_mut())
        {
            delay_line
                [self.next_write_position..self.next_write_position + num_samples_before_wrap]
                .copy_from_slice(&buffer_channel[..num_samples_before_wrap]);
            delay_line[..num_samples_after_wrap]
                .copy_from_slice(&buffer_channel[num_samples_before_wrap..]);
        }

        self.next_write_position = (self.next_write_position + buffer.samples()) % delay_line_len;
    }

    /// Mix the dry signal into the buffer. The ratio is a `[0, 1]` integer where 0 results in an
    /// all-dry signal, and 1 results in an all-wet signal. This should be called at the start of
    /// the process function.
    ///
    /// # Panics
    ///
    /// Panics if the buffer is larger than the maximum block size, if the latency is larger than
    /// the maximum latency, or if the channel counts don't match.
    pub fn mix_in_dry(
        &mut self,
        buffer: &mut Buffer,
        ratio: f32,
        style: MixingStyle,
        latency: usize,
    ) {
        if buffer.channels() == 0 {
            return;
        }

        let ratio = ratio.clamp(0.0, 1.0);
        if ratio == 1.0 {
            return;
        }
        let (wet_t, dry_t) = match style {
            MixingStyle::Linear => (ratio, 1.0 - ratio),
            MixingStyle::EqualPower => (ratio.sqrt(), (1.0 - ratio).sqrt()),
        };

        assert_eq!(buffer.channels(), self.delay_line.len());
        let delay_line_len = self.delay_line[0].len();
        assert!(buffer.samples() + latency <= delay_line_len);

        let read_position =
            (self.next_write_position + delay_line_len - buffer.samples() - latency)
                % delay_line_len;
        let num_samples_before_wrap = buffer.samples().min(delay_line_len - read_position);
        let num_samples_after_wrap = buffer.samples() - num_samples_before_wrap;

        for (buffer_channel, delay_line) in buffer.as_slice().iter_mut().zip(self.delay_line.iter())
        {
            if ratio == 0.0 {
                buffer_channel[..num_samples_before_wrap].copy_from_slice(
                    &delay_line[read_position..read_position + num_samples_before_wrap],
                );
                buffer_channel[num_samples_before_wrap..]
                    .copy_from_slice(&delay_line[..num_samples_after_wrap]);
            } else {
                for (buffer_sample, delay_sample) in buffer_channel[..num_samples_before_wrap]
                    .iter_mut()
                    .zip(&delay_line[read_position..read_position + num_samples_before_wrap])
                {
                    *buffer_sample = (*buffer_sample * wet_t) + (delay_sample * dry_t);
                }
                for (buffer_sample, delay_sample) in buffer_channel[num_samples_before_wrap..]
                    .iter_mut()
                    .zip(&delay_line[..num_samples_after_wrap])
                {
                    *buffer_sample = (*buffer_sample * wet_t) + (delay_sample * dry_t);
                }
            }
        }
    }
}
