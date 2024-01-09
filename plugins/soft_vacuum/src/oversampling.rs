// Soft Vacuum: Airwindows Hard Vacuum port with oversampling
// Copyright (C) 2023-2024 Robbert van der Helm
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

use nih_plug::debug::*;

/// The kernel used in `Lanczos3Oversampler`. Specified here as a constant since it is a constant.
/// Precomputed since compile-time floating point arithmetic is still unstable.
///
/// Computed using:
///
/// ```python
/// LANCZOS_A = 3
///
/// x = np.arange(-LANCZOS_A * 2 + 1, LANCZOS_A * 2) / 2
/// np.sinc(x) * np.sinc(x / LANCZOS_A)
/// ```
///
/// Note the `+1` at the start of the range and the lack of `+1` at the (exclusive) end of the
/// range. This is because we can ommit the first and last point because they are always zero.
const LANCZOS3_UPSAMPLING_KERNEL: [f32; 11] = [
    0.02431708,
    -0.0,
    -0.13509491,
    0.0,
    0.6079271,
    1.0,
    0.6079271,
    0.0,
    -0.13509491,
    -0.0,
    0.02431708,
];

/// `LANCZOS3_UPSAMPLING_KERNEL` divided by two, used for downsampling so that upsampling followed
/// by downsampling results in unity gain.
const LANCZOS3_DOWNSAMPLING_KERNEL: [f32; 11] = [
    0.01215854,
    -0.0,
    -0.06754746,
    0.0,
    0.30396355,
    0.5,
    0.30396355,
    0.0,
    -0.06754746,
    -0.0,
    0.01215854,
];

/// The latency introduced by the two filter kernels defined above, in samples.
const LANZCOS3_KERNEL_LATENCY: usize = LANCZOS3_UPSAMPLING_KERNEL.len() / 2;

/// A barebones multi-stage linear-phase oversampler that uses the lanzcos kernel with a=3 for a
/// good approximation of a windowed sinc with only a 11 point kernel function (the kernel is
/// actually 13 points, but the outer two points are both zero can can thus be omitted). This can be
/// done much more efficiently but I was in a hurry and this is simple to implement without having
/// to look anything up.
///
/// This only handles a single audio channel. Use multiple instances for multichannel audio.
#[derive(Debug)]
pub struct Lanczos3Oversampler {
    /// The state used for each oversampling stage. Also contains stages that are not being used, so
    /// the number of stages can change without allocating. The number of currently active
    /// stages/the oversampling factor passed to [`process()`][Self::process()] determines how many
    /// of these are actually used.
    stages: Vec<Lanzcos3Stage>,

    /// The oversampler's latency. Precomputed for each possible number of active stages.
    latencies: Vec<u32>,
}

/// A single oversampling stage. Contains the ring buffers and current position in that ringbuffer
/// used for convolving the filter with the inputs in the upsampling and downsampling parts of the
/// stage.
#[derive(Debug, Clone)]
struct Lanzcos3Stage {
    /// The amount of oversampling that happens at this stage. Will be 2 for the first stage, 4 for
    /// the second stage, 8 for the third stage, and so forth. Used to calculate the stage's effect
    /// on the oversampling's latency.
    oversampling_amount: usize,

    /// These ring buffers contain `LANCZOS3_UPSAMPLING_KERNEL.len()` samples. The upsampling ring
    /// buffer contains room to delay the signal further to make sure the _total_
    /// (upsampling+downsampling) latency imposed on the signal is divisible by the stage's
    /// oversampling amount. That is needed to avoid fractional latency.
    upsampling_rb: Vec<f32>,
    upsampling_write_pos: usize,
    /// The additional delay for the upsampling needed to make this stage impose an integer amount
    /// of latency. The stage's _total_ (upsampling+downsampling) latency needs to be divisible by
    /// the stage's oversampling amount.
    additional_upsampling_latency: usize,

    /// No additional latency needs to be imposed for the downsampling, so to keep things simple
    /// this doesn't add any additional delay.
    downsampling_rb: [f32; LANCZOS3_DOWNSAMPLING_KERNEL.len()],
    downsampling_write_pos: usize,

    scratch_buffer: Vec<f32>,
}

impl Lanczos3Oversampler {
    /// Create a new oversampler that can oversample to up to the specified oversampling factor, or
    /// the 2-logarithm of the oversampling amount. 1x oversampling (aka, do nothing) = 0, 2x
    /// oversampling = 1, 4x oversampling = 3, etc. The actual amount of oversampling stages used is
    /// passed to the `process()` function, and must be set to `max_factor` or lower.
    pub fn new(maximum_block_size: usize, max_factor: usize) -> Self {
        let mut stages = Vec::with_capacity(max_factor);
        for stage in 0..max_factor {
            stages.push(Lanzcos3Stage::new(maximum_block_size, stage))
        }

        // Since the number of active oversampling stages is passed to the process function, we also
        // need to know the effective latencies of all possible oversampling settings in advance.
        let latencies = stages
            .iter()
            .map(|stage| stage.effective_latency())
            .scan(0, |total_latency, latency| {
                *total_latency += latency;
                Some(*total_latency)
            })
            .collect();

        Self { stages, latencies }
    }

    /// Reset the oversampling filters to their initial states.
    pub fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }

    /// Get the latency in samples for the given oversampling factor. Fractional latency is
    /// automatically avoided.
    ///
    /// # Panics
    ///
    /// Panics if `factor > max_factor`.
    pub fn latency(&self, factor: usize) -> u32 {
        if factor == 0 {
            0
        } else {
            self.latencies[factor - 1]
        }
    }

    /// Upsample `block` using the specified oversampling factor, process the upsampled version
    /// using `f`, and then downsample it again and write the results back to `block` with a
    /// [`latency()`][Self::latency()] sample delay.
    ///
    /// # Panics
    ///
    /// Panics if `factor > max_factor`, or if `block`'s length is longer than the maximum block
    /// size.
    pub fn process(&mut self, block: &mut [f32], factor: usize, f: impl FnOnce(&mut [f32])) {
        assert!(factor <= self.stages.len());

        // This is the 1x oversampling case, this should also modify the block to be consistent
        if factor == 0 {
            f(block);
            return;
        }

        assert!(
            block.len() <= self.stages[0].scratch_buffer.len() / 2,
            "The block's size exceeds the maximum block size"
        );

        let upsampled = self.upsample_from(block, factor);
        f(upsampled);
        self.downsample_to(block, factor)
    }

    /// An upsample-only version of `process` that returns the upsampled version of the signal that
    /// would normally be passed to `process`'s callback. Useful for upsampling control signals.
    ///
    /// # Panics
    ///
    /// Panics if `factor > max_factor`, or if `block`'s length is longer than the maximum block
    /// size.
    pub fn upsample_only<'a>(&'a mut self, block: &'a mut [f32], factor: usize) -> &'a mut [f32] {
        assert!(factor <= self.stages.len());

        // This is the 1x oversampling case, this should also modify the block to be consistent
        if factor == 0 {
            return block;
        }

        assert!(
            block.len() <= self.stages[0].scratch_buffer.len() / 2,
            "The block's size exceeds the maximum block size"
        );

        self.upsample_from(block, factor)
    }

    /// Upsample `block` through `factor` oversampling stages. Returns a reference to the
    /// oversampled output stored in the last `LancZos3Stage`'s scratch buffer **with the correct
    /// length**. This is a multiple of `block`'s length, which may be shorter than the entire
    /// scratch buffer's length if `block` is shorter than the configured maximum block length.
    ///
    /// # Panics
    ///
    /// Panics if `block`'s length is longer than the maximum block size, if the number of
    /// oversampling is smaller than `factor`, or if `factor` is zero. This is already checked for
    /// in the process function.
    fn upsample_from(&mut self, block: &[f32], factor: usize) -> &mut [f32] {
        assert_ne!(factor, 0);
        assert!(factor <= self.stages.len());

        // The first stage is upsampled from `block`, and everything after that is upsampled from
        // the stage preceeding it
        self.stages[0].upsample_from(block);

        let mut previous_upsampled_block_len = block.len() * 2;
        for to_stage_idx in 1..factor {
            // This requires splitting the vector so we can borrow the from-stage immutably and the
            // to-stage mutably at the same time
            let ([.., from], [to, ..]) = self.stages.split_at_mut(to_stage_idx) else {
                unreachable!()
            };

            to.upsample_from(&from.scratch_buffer[..previous_upsampled_block_len]);
            previous_upsampled_block_len *= 2;
        }

        &mut self.stages[factor - 1].scratch_buffer[..previous_upsampled_block_len]
    }

    /// Downsample starting from the `factor`th oversampling stage, writing the results from
    /// downsampling the first stage to `block`. `block`'s actual length is taken into account to
    /// compute the length of the oversampled blocks.
    ///
    /// # Panics
    ///
    /// Panics if `block`'s length is longer than the maximum block size, if the number of
    /// oversampling is smaller than `factor`, or if `factor` is zero. This is already checked for
    /// in the process function.
    fn downsample_to(&mut self, block: &mut [f32], factor: usize) {
        assert_ne!(factor, 0);
        assert!(factor <= self.stages.len());

        // This is the reverse of `upsample_from`. Starting from the last stage, the oversampling
        // stages are downsampled to the previous stage and then the first stage is downsampled to
        // `block`.
        let mut next_downsampled_block_len = block.len() * 2usize.pow(factor as u32 - 1);
        for to_stage_idx in (1..factor).rev() {
            // This requires splitting the vector so we can borrow the from-stage immutably and the
            // to-stage mutably at the same time
            let ([.., to], [from, ..]) = self.stages.split_at_mut(to_stage_idx) else {
                unreachable!()
            };

            from.downsample_to(&mut to.scratch_buffer[..next_downsampled_block_len]);
            next_downsampled_block_len /= 2;
        }

        // And then the first stage downsamples to `block`
        assert_eq!(next_downsampled_block_len, block.len());
        self.stages[0].downsample_to(block);
    }
}

impl Lanzcos3Stage {
    /// Create a `stage_number`th oversampling stage, where `stage_number` is this stage's
    /// zero-based index in a list of stages. Stage 0 handles the 2x oversampling, stage 1 handles
    /// the 4x oversampling, stage 2 handles the 8x oversampling, etc.. This is used to make sure
    /// the stage's effect on the total latency is always an integer amount.
    ///
    /// The maximum block size is used to allocate enough scratch space for oversampling that many
    /// samples *at the base sample rate*. The scratch buffer's size automatically takes the stage
    /// number into account.
    pub fn new(maximum_block_size: usize, stage_number: usize) -> Self {
        let oversampling_amount = 2usize.pow(stage_number as u32 + 1);

        // In theory we would only need to delay one of these, but we'll distribute the delay
        // cleanly
        assert!(LANCZOS3_UPSAMPLING_KERNEL.len() == LANCZOS3_DOWNSAMPLING_KERNEL.len());
        assert!(LANCZOS3_UPSAMPLING_KERNEL.len() % 2 == 1);

        // This is the latency of the upsampling and downsampling filter, at the base sample rate.
        // Because this stage's filtering happens at a higher sample rate (`oversampling_amount`
        // times the base sample rate), we need to make sure that the delay imposed _on this higher
        // sample rate_ results in an integer amount of latency at the base sample rate. To do that,
        // the delay needs to be divisible by `oversampling_amount`. This extra delay is only
        // applied to the upsampling part to keep the downsampling simpler.
        let uncompensated_stage_latency = LANZCOS3_KERNEL_LATENCY + LANZCOS3_KERNEL_LATENCY;

        // Say the oversampling amount is 4, then an uncompensated stage latency of 8 results in 0
        // additional samples of delay, 9 in 3, 10 in 2, 11 in 1, 12 in 0, etc. This is added to the
        // upsampling filter.
        let additional_delay_required = (-(uncompensated_stage_latency as isize))
            .rem_euclid(oversampling_amount as isize)
            as usize;

        Self {
            oversampling_amount,

            upsampling_rb: vec![0.0; LANCZOS3_UPSAMPLING_KERNEL.len() + additional_delay_required],
            upsampling_write_pos: 0,
            additional_upsampling_latency: additional_delay_required,

            downsampling_rb: [0.0; LANCZOS3_DOWNSAMPLING_KERNEL.len()],
            downsampling_write_pos: 0,

            scratch_buffer: vec![0.0; maximum_block_size * oversampling_amount],
        }
    }

    pub fn reset(&mut self) {
        // Resetting the positions is not needed, but it also doesn't hurt
        self.upsampling_rb.fill(0.0);
        self.upsampling_write_pos = 0;

        self.downsampling_rb.fill(0.0);
        self.downsampling_write_pos = 0;
    }

    /// The stage's effect on the oversampling's latency as a whole. This is already divided by the
    /// stage's oversampling amount.
    pub fn effective_latency(&self) -> u32 {
        let uncompensated_stage_latency = LANZCOS3_KERNEL_LATENCY + LANZCOS3_KERNEL_LATENCY;
        let total_stage_latency = uncompensated_stage_latency + self.additional_upsampling_latency;

        let effective_latency = total_stage_latency as f32 / self.oversampling_amount as f32;
        assert!(effective_latency.fract() == 0.0);

        effective_latency as u32
    }

    /// Upsample `block` 2x and write the results to this stage's scratch buffer.
    ///
    /// # Panics
    ///
    /// Panics if `block`'s times two exceeds the scratch buffer's size.
    pub fn upsample_from(&mut self, block: &[f32]) {
        let output_length = block.len() * 2;
        assert!(output_length <= self.scratch_buffer.len());

        // We'll first zero-stuff the input, and then run that through the lanczos halfband filter
        for (input_sample_idx, input_sample) in block.iter().enumerate() {
            let output_sample_idx = input_sample_idx * 2;
            self.scratch_buffer[output_sample_idx] = *input_sample;
            self.scratch_buffer[output_sample_idx + 1] = 0.0;
        }

        // The zero-stuffed input is now run through the lanczos filter, which is a windowed sinc
        // filter where every even tap has a value of zero. That means that if the filter is
        // centered on a non-zero sample, the output must be equal to that sample and we can thus
        // skip the convolution step entirely. Another important consideration is that we are
        // imposing an additional `self.additional_upsampling_latency` samples of delay on the input
        // to make sure the effective latency of the oversampling is always an integer amount.
        let mut direct_read_pos =
            (self.upsampling_write_pos + LANZCOS3_KERNEL_LATENCY) % self.upsampling_rb.len();
        for output_sample_idx in 0..output_length {
            // For a more intuitive description, imagine that `self.additional_upsampling_latency`
            // is 2, and `self.upsampling_write_pos` is currently 0. For an 11-tap filter (like the
            // lanczos3 kernel with the zero points removed from both ends), the situation after
            // this statement would look like this:
            //
            // [n, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
            //  ^-- self.upsampling_write_pos
            self.upsampling_rb[self.upsampling_write_pos] = self.scratch_buffer[output_sample_idx];

            // The read/write head position needs to be incremented before filtering so that the
            // just-added sample becomes the last sample in the ring buffer (if the additional
            // latency/delay is 0)
            self.upsampling_write_pos += 1;
            if self.upsampling_write_pos == self.upsampling_rb.len() {
                self.upsampling_write_pos = 0;
            }

            direct_read_pos += 1;
            if direct_read_pos == self.upsampling_rb.len() {
                direct_read_pos = 0;
            }

            // We can now read starting from the new `self.upsampling_write_pos`. This will cause
            // the output to be delayed by `self.additional_upsampling_latency` samples. The range
            // used for convolution is visualized below. It in this example it takes 2 additional
            // iterations of this loop before sample `n` is considered again. Even output samples
            // can directly be read from the ring buffer without convolution at the visualized
            // offset.
            //
            // [n, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
            //     ^--------------^---------------^
            //                    └- direct_read_position
            //
            // NOTE: 'Even samples' is considered from the perspective of a zero latency filter. In
            //       this case the evenness of the filter's latency also needs to be considered. If
            //       it's odd then the direct reading should also happen for odd indexed samples.
            self.scratch_buffer[output_sample_idx] =
                if output_sample_idx % 2 == (LANZCOS3_KERNEL_LATENCY % 2) {
                    nih_debug_assert_eq!(
                        self.upsampling_rb[(direct_read_pos + self.upsampling_rb.len() - 1)
                            % self.upsampling_rb.len()],
                        0.0
                    );
                    nih_debug_assert_eq!(
                        self.upsampling_rb[(direct_read_pos + 1) % self.upsampling_rb.len()],
                        0.0
                    );

                    self.upsampling_rb[direct_read_pos]
                } else {
                    convolve_rb(
                        &self.upsampling_rb,
                        &LANCZOS3_UPSAMPLING_KERNEL,
                        self.upsampling_write_pos,
                    )
                };
        }
    }

    /// Downsample starting from the last oversampling stage, writing the results from downsampling
    /// the first stage to `block`. `block`'s actual length is taken into account to compute the
    /// length of the oversampled blocks.
    ///
    /// # Panics
    ///
    /// Panics if `block`'s divided by two exceeds the scratch buffer's size.
    pub fn downsample_to(&mut self, block: &mut [f32]) {
        let input_length = block.len() * 2;
        assert!(input_length <= self.scratch_buffer.len());

        // The additional delay to make the latency integer has already been taken into account in
        // the upsampling part, so the downsampling is more straightforward
        for input_sample_idx in 0..input_length {
            self.downsampling_rb[self.downsampling_write_pos] =
                self.scratch_buffer[input_sample_idx];

            // The read/write head position needs to be incremented before filtering so that the
            // just-added sample becomes the last sample in the ring buffer
            self.downsampling_write_pos += 1;
            if self.downsampling_write_pos == LANCZOS3_DOWNSAMPLING_KERNEL.len() {
                self.downsampling_write_pos = 0;
            }

            // Because downsampling by a factor of two is filtering followed by decimation (where
            // you take every even sample), we only need to compute the filtered output for the even
            // samples. This is similar to how we only need to filter half the samples in the
            // upsampling step.
            if input_sample_idx % 2 == 0 {
                let output_sample_idx = input_sample_idx / 2;
                block[output_sample_idx] = convolve_rb(
                    &self.downsampling_rb,
                    // NOTE: This is `LANCZOS3_UPSAMPLING_KERNEL`, but with a factor two gain
                    //       decrease to compensate for the 2x gain increase that happened during
                    //       the upsampling
                    &LANCZOS3_DOWNSAMPLING_KERNEL,
                    self.downsampling_write_pos,
                )
            }
        }
    }
}

/// Convolve `input_ring_buffer` with `kernel`, with `input_ring_buffer` rotated so that it starts
/// at `ring_buffer_pos` and then wraps back around to the start.
///
/// # Panics
///
/// Assumes `input_ring_buffer` and `kernel` have the same length. May panic if they don't.
fn convolve_rb(input_ring_buffer: &[f32], kernel: &[f32], ring_buffer_pos: usize) -> f32 {
    let mut total = 0.0;

    nih_debug_assert!(input_ring_buffer.len() >= kernel.len());

    // This is straightforward convolution. Could be implemented much more efficiently, but for our
    // 11-tap filter this works fine
    let num_samples_until_wraparound =
        (input_ring_buffer.len() - ring_buffer_pos).min(kernel.len());
    for (read_pos_offset, kernel_sample) in kernel
        .iter()
        .rev()
        .take(num_samples_until_wraparound)
        .enumerate()
    {
        total += kernel_sample * input_ring_buffer[ring_buffer_pos + read_pos_offset];
    }

    for (read_pos, kernel_sample) in kernel
        .iter()
        .rev()
        // Needs to happen before the `enumerate`
        .skip(num_samples_until_wraparound)
        .enumerate()
    {
        total += kernel_sample * input_ring_buffer[read_pos];
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    mod convolve_rb {
        use super::*;

        #[test]
        fn test_with_wrap() {
            let input_rb = [1.0, 2.0, -3.0, 4.0];
            let kernel = [1.0, 2.0, -0.0, -1.0];
            let input_pos = 2;

            // This should be `(-3.0 * -1.0) + (4.0 * 0.0) + (1.0 * 2.0) + (2.0 * 1.0) = 7.0`
            let result = convolve_rb(&input_rb, &kernel, input_pos);
            assert_eq!(result, 7.0);
        }

        #[test]
        fn test_no_wrap() {
            let input_rb = [1.0, 2.0, -3.0, 4.0];
            let kernel = [1.0, 2.0, 0.0, -1.0];
            let input_pos = 0;

            // This should be `(1.0 * -1.0) + (2.0 * 0.0) + (-3.0 * 2.0) + (4.0 * 1.0) = 7.0`
            let result = convolve_rb(&input_rb, &kernel, input_pos);
            assert_eq!(result, -3.0);
        }
    }

    mod oversampling {
        use super::*;

        fn argmax(iter: impl IntoIterator<Item = f32>) -> usize {
            iter.into_iter()
                .enumerate()
                .max_by(|(_, value_a), (_, value_b)| value_a.total_cmp(value_b))
                .unwrap()
                .0
        }

        /// Makes sure that the reported latency is correct and is (more or less) an integer value
        fn test_latency(oversampling_factor: usize) {
            let mut delta_impulse = [0.0f32; 64];
            delta_impulse[0] = 1.0;

            let mut oversampler =
                Lanczos3Oversampler::new(delta_impulse.len(), oversampling_factor);

            let reported_latency = oversampler.latency(oversampling_factor) as usize;
            assert!(
                delta_impulse.len() > reported_latency,
                "The delta impulse array is too small to test the latency at oversampling factor \
                 {oversampling_factor}, this is an error with the test case"
            );

            oversampler.process(&mut delta_impulse, oversampling_factor, |_| ());

            let new_impulse_idx = argmax(delta_impulse);
            assert_eq!(new_impulse_idx, reported_latency);

            // The latency should also not be fractional
            assert!(delta_impulse[new_impulse_idx] > delta_impulse[new_impulse_idx - 1]);
            assert!(delta_impulse[new_impulse_idx] > delta_impulse[new_impulse_idx + 1]);
        }

        /// Checks whether the output matches the input when compensating for the latency. Also
        /// applies a gain offset to make sure the process callback actually works.
        fn test_sine_output(oversampling_factor: usize) {
            // The gain applied to the oversampled version
            const GAIN: f32 = 2.0;
            // As a fraction of the sampling frequency
            const FREQUENCY: f32 = 0.125;

            let mut input = [0.0f32; 128];
            for (i, sample) in input.iter_mut().enumerate() {
                *sample = (i as f32 * (FREQUENCY * 2.0 * std::f32::consts::PI)).sin();
            }

            let mut output = input;
            let mut oversampler = Lanczos3Oversampler::new(output.len(), oversampling_factor);
            oversampler.process(&mut output, oversampling_factor, |upsampled| {
                for sample in upsampled {
                    *sample *= GAIN;
                }
            });

            let reported_latency = oversampler.latency(oversampling_factor) as usize;
            for (input_sample_idx, input_sample) in input
                .into_iter()
                .enumerate()
                .take(input.len() - reported_latency)
            {
                let output_sample_idx = input_sample_idx + reported_latency;
                let output_sample = output[output_sample_idx];

                // There can be quite a big difference between the input and output thanks to the
                // filter's ringing
                approx::assert_relative_eq!(input_sample * GAIN, output_sample, epsilon = 0.1);
            }
        }

        #[test]
        fn latency_2x() {
            test_latency(1);
        }

        #[test]
        fn latency_4x() {
            test_latency(2);
        }

        #[test]
        fn latency_8x() {
            test_latency(3);
        }

        #[test]
        fn latency_16x() {
            test_latency(4);
        }

        #[test]
        fn sine_output_2x() {
            test_sine_output(1);
        }

        #[test]
        fn sine_output_4x() {
            test_sine_output(2);
        }

        #[test]
        fn sine_output_8x() {
            test_sine_output(3);
        }

        #[test]
        fn sine_output_16x() {
            test_sine_output(4);
        }
    }
}
