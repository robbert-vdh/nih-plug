//! Utilities for buffering audio, likely used as part of a short-term Fourier transform.

use std::cmp;

use crate::buffer::{Block, Buffer};

/// Some buffer that can be used with the [`StftHelper`].
pub trait StftInput {
    /// The number of samples in this input.
    fn num_samples(&self) -> usize;

    /// The number of channels in this input.
    fn num_channels(&self) -> usize;

    /// Index the buffer without any bounds checks.
    unsafe fn get_sample_unchecked(&self, channel: usize, sample_idx: usize) -> f32;
}

/// The same as [`StftInput`], but with support for writing results back to the buffer
pub trait StftInputMut: StftInput {
    /// Get a mutable reference to a sample in the buffer without any bounds checks.
    unsafe fn get_sample_unchecked_mut(&mut self, channel: usize, sample_idx: usize) -> &mut f32;
}

/// Process the input buffer in equal sized blocks, running a callback on each block to transform
/// the block and then writing back the results from the previous block to the buffer. This
/// introduces latency equal to the size of the block.
///
/// Additional inputs can be processed by setting the `NUM_SIDECHAIN_INPUTS` constant. These buffers
/// will not be written to, so they are purely used for analysis. These sidechain inputs will have
/// the same number of channels as the main input.
///
/// TODO: Better name?
/// TODO: We may need something like this purely for analysis, e.g. for showing spectrums in a GUI.
///       Figure out the cleanest way to adapt this for the non-processing use case.
pub struct StftHelper<const NUM_SIDECHAIN_INPUTS: usize = 0> {
    // These ring buffers store the input samples and the already processed output produced by
    // adding overlapping windows. Whenever we reach a new overlapping window, we'll write the
    // already calculated outputs to the main buffer passed to the process function and then process
    // a new block.
    main_input_ring_buffers: Vec<Vec<f32>>,
    main_output_ring_buffers: Vec<Vec<f32>>,
    sidechain_ring_buffers: [Vec<Vec<f32>>; NUM_SIDECHAIN_INPUTS],

    /// Results from the ring buffers are copied to this scratch buffer before being passed to the
    /// plugin. Needed to handle overlap.
    scratch_buffer: Vec<f32>,
    /// If padding is used, then this will contain the previous iteration's values from the padding
    /// values in `scratch_buffer` (`scratch_buffer[(scratch_buffer.len() - padding -
    /// 1)..scratch_buffer.len()]`). This is then added to the ring buffer in the next iteration.
    padding_buffers: Vec<Vec<f32>>,

    /// The current position in our ring buffers. Whenever this wraps around to 0, we'll process
    /// a block.
    current_pos: usize,
    /// If padding is used, then this much extra capacity has been added to the buffers.
    padding: usize,
}

/// Marker struct for the version without sidechaining.
struct NoSidechain;

impl StftInput for Buffer<'_> {
    #[inline]
    fn num_samples(&self) -> usize {
        self.samples()
    }

    #[inline]
    fn num_channels(&self) -> usize {
        self.channels()
    }

    #[inline]
    unsafe fn get_sample_unchecked(&self, channel: usize, sample_idx: usize) -> f32 {
        *self
            .as_slice_immutable()
            .get_unchecked(channel)
            .get_unchecked(sample_idx)
    }
}

impl StftInputMut for Buffer<'_> {
    #[inline]
    unsafe fn get_sample_unchecked_mut(&mut self, channel: usize, sample_idx: usize) -> &mut f32 {
        self.as_slice()
            .get_unchecked_mut(channel)
            .get_unchecked_mut(sample_idx)
    }
}

impl StftInput for Block<'_, '_> {
    #[inline]
    fn num_samples(&self) -> usize {
        self.samples()
    }

    #[inline]
    fn num_channels(&self) -> usize {
        self.channels()
    }

    #[inline]
    unsafe fn get_sample_unchecked(&self, channel: usize, sample_idx: usize) -> f32 {
        *self.get_unchecked(channel).get_unchecked(sample_idx)
    }
}

impl StftInputMut for Block<'_, '_> {
    #[inline]
    unsafe fn get_sample_unchecked_mut(&mut self, channel: usize, sample_idx: usize) -> &mut f32 {
        self.get_unchecked_mut(channel)
            .get_unchecked_mut(sample_idx)
    }
}

impl StftInput for [&[f32]] {
    #[inline]
    fn num_samples(&self) -> usize {
        if self.is_empty() {
            0
        } else {
            self[0].len()
        }
    }

    #[inline]
    fn num_channels(&self) -> usize {
        self.len()
    }

    #[inline]
    unsafe fn get_sample_unchecked(&self, channel: usize, sample_idx: usize) -> f32 {
        *self.get_unchecked(channel).get_unchecked(sample_idx)
    }
}

impl StftInput for [&mut [f32]] {
    #[inline]
    fn num_samples(&self) -> usize {
        if self.is_empty() {
            0
        } else {
            self[0].len()
        }
    }

    #[inline]
    fn num_channels(&self) -> usize {
        self.len()
    }

    #[inline]
    unsafe fn get_sample_unchecked(&self, channel: usize, sample_idx: usize) -> f32 {
        *self.get_unchecked(channel).get_unchecked(sample_idx)
    }
}

impl StftInputMut for [&mut [f32]] {
    #[inline]
    unsafe fn get_sample_unchecked_mut(&mut self, channel: usize, sample_idx: usize) -> &mut f32 {
        self.get_unchecked_mut(channel)
            .get_unchecked_mut(sample_idx)
    }
}

impl StftInput for NoSidechain {
    fn num_samples(&self) -> usize {
        0
    }

    fn num_channels(&self) -> usize {
        0
    }

    unsafe fn get_sample_unchecked(&self, _channel: usize, _sample_idx: usize) -> f32 {
        0.0
    }
}

impl<const NUM_SIDECHAIN_INPUTS: usize> StftHelper<NUM_SIDECHAIN_INPUTS> {
    /// Initialize the [`StftHelper`] for [`Buffer`]s with the specified number of channels and the
    /// given maximum block size. When the option is set, then every yielded sample buffer will have
    /// this many zero samples appended at the end of the block. Call
    /// [`set_block_size()`][Self::set_block_size()] afterwards if you do not need the full capacity
    /// upfront. If the padding option is non zero, then all yielded blocks will have that many
    /// zeroes added to the end of it and the results stored in the padding area will be added to
    /// the outputs in the next iteration(s). You may also change how much padding is added with
    /// [`set_padding()`][Self::set_padding()].
    ///
    /// # Panics
    ///
    /// Panics if `num_channels == 0 || max_block_size == 0`.
    pub fn new(num_channels: usize, max_block_size: usize, max_padding: usize) -> Self {
        assert_ne!(num_channels, 0);
        assert_ne!(max_block_size, 0);

        Self {
            main_input_ring_buffers: vec![vec![0.0; max_block_size]; num_channels],
            main_output_ring_buffers: vec![vec![0.0; max_block_size]; num_channels],
            // Kinda hacky way to initialize an array of non-copy types
            sidechain_ring_buffers: [(); NUM_SIDECHAIN_INPUTS]
                .map(|_| vec![vec![0.0; max_block_size]; num_channels]),

            // When padding is used this scratch buffer will have a bunch of zeroes added to it
            // after copying a block of audio to it
            scratch_buffer: vec![0.0; max_block_size + max_padding],
            padding_buffers: vec![vec![0.0; max_padding]; num_channels],

            current_pos: 0,
            padding: max_padding,
        }
    }

    /// Change the current block size. This will clear the buffers, causing the next block to output
    /// silence.
    ///
    /// # Panics
    ///
    /// Will panic if `block_size > max_block_size`.
    pub fn set_block_size(&mut self, block_size: usize) {
        assert!(block_size <= self.main_input_ring_buffers[0].capacity());

        self.update_buffers(block_size);
    }

    /// Change the current padding amount. This will clear the buffers, causing the next block to
    /// output silence.
    ///
    /// # Panics
    ///
    /// Will panic if `padding > max_padding`.
    pub fn set_padding(&mut self, padding: usize) {
        assert!(padding <= self.padding_buffers[0].capacity());

        self.padding = padding;
        self.update_buffers(self.main_input_ring_buffers[0].len());
    }

    /// The number of channels this `StftHelper` was configured for
    pub fn num_channels(&self) -> usize {
        self.main_input_ring_buffers.len()
    }

    /// The maximum block size supported by this instance.
    pub fn max_block_size(&self) -> usize {
        self.main_input_ring_buffers.capacity()
    }

    /// The maximum amount of padding supported by this instance.
    pub fn max_padding(&self) -> usize {
        self.padding_buffers[0].capacity()
    }

    /// The amount of latency introduced when processing audio through this [`StftHelper`].
    pub fn latency_samples(&self) -> u32 {
        self.main_input_ring_buffers[0].len() as u32
    }

    /// Process the audio in `main_buffer` in small overlapping blocks, adding up the results for
    /// the main buffer so they can eventually be written back to the host one block later. This
    /// means that this function will introduce one block of latency. This can be compensated by
    /// calling [`InitContext::set_latency()`][`crate::prelude::InitContext::set_latency_samples()`]
    /// in your plugin's initialization function.
    ///
    /// If a padding value was specified in [`new()`][Self::new()], then the yielded blocks will
    /// have that many zeroes appended at the end of them. The padding values will be added to the
    /// next block before `process_cb()` is called.
    ///
    /// Since there are a couple different ways to do it, any window functions needs to be applied
    /// in the callbacks. Check the [`nih_plug::util::window`][crate::util::window] module for more information.
    ///
    /// For efficiency's sake this function will reuse the same vector for all calls to
    /// `process_cb`. This means you can only access a single channel's worth of windowed data at a
    /// time. The arguments to that function are `process_cb(channel_idx, real_fft_buffer)`.
    /// `real_fft_buffer` will be a slice of `block_size` real valued samples. This can be passed
    /// directly to an FFT algorithm.
    ///
    /// # Panics
    ///
    /// Panics if `main_buffer` or the buffers in `sidechain_buffers` do not have the same number of
    /// channels as this [`StftHelper`], or if the sidechain buffers do not contain the same number of
    /// samples as the main buffer.
    ///
    /// TODO: Add more useful ways to do STFT and other buffered operations. I just went with this
    ///       approach because it's what I needed myself, but generic combinators like this could
    ///       also be useful for other operations.
    pub fn process_overlap_add<M, F>(
        &mut self,
        main_buffer: &mut M,
        overlap_times: usize,
        mut process_cb: F,
    ) where
        M: StftInputMut,
        F: FnMut(usize, &mut [f32]),
    {
        self.process_overlap_add_sidechain(
            main_buffer,
            [&NoSidechain; NUM_SIDECHAIN_INPUTS],
            overlap_times,
            |channel_idx, sidechain_idx, real_fft_scratch_buffer| {
                if sidechain_idx.is_none() {
                    process_cb(channel_idx, real_fft_scratch_buffer);
                }
            },
        );
    }

    /// The same as [`process_overlap_add()`][Self::process_overlap_add()], but with sidechain
    /// inputs that can be analyzed before the main input gets processed.
    ///
    /// The extra argument in the process function is `sidechain_buffer_idx`, which will be `None`
    /// for the main buffer.
    pub fn process_overlap_add_sidechain<M, S, F>(
        &mut self,
        main_buffer: &mut M,
        sidechain_buffers: [&S; NUM_SIDECHAIN_INPUTS],
        overlap_times: usize,
        mut process_cb: F,
    ) where
        M: StftInputMut,
        S: StftInput,
        F: FnMut(usize, Option<usize>, &mut [f32]),
    {
        assert_eq!(
            main_buffer.num_channels(),
            self.main_input_ring_buffers.len()
        );
        assert!(overlap_times > 0);

        // We'll copy samples from `*_buffer` into `*_ring_buffers` while simultaneously copying
        // already processed samples from `main_ring_buffers` in into `main_buffer`
        let main_buffer_len = main_buffer.num_samples();
        let num_channels = main_buffer.num_channels();
        let block_size = self.main_input_ring_buffers[0].len();
        let window_interval = (block_size / overlap_times) as i32;
        let mut already_processed_samples = 0;
        while already_processed_samples < main_buffer_len {
            let remaining_samples = main_buffer_len - already_processed_samples;
            let samples_until_next_window = ((window_interval - self.current_pos as i32 - 1)
                .rem_euclid(window_interval)
                + 1) as usize;
            let samples_to_process = samples_until_next_window.min(remaining_samples);

            // Copy the input from `main_buffer` to the ring buffer while copying last block's
            // result from the buffer to `main_buffer`
            // TODO: This might be able to be sped up a bit with SIMD

            // For the main buffer
            for sample_offset in 0..samples_to_process {
                for channel_idx in 0..num_channels {
                    let sample = unsafe {
                        main_buffer.get_sample_unchecked_mut(
                            channel_idx,
                            already_processed_samples + sample_offset,
                        )
                    };
                    let input_ring_buffer_sample = unsafe {
                        self.main_input_ring_buffers
                            .get_unchecked_mut(channel_idx)
                            .get_unchecked_mut(self.current_pos + sample_offset)
                    };
                    let output_ring_buffer_sample = unsafe {
                        self.main_output_ring_buffers
                            .get_unchecked_mut(channel_idx)
                            .get_unchecked_mut(self.current_pos + sample_offset)
                    };
                    *input_ring_buffer_sample = *sample;
                    *sample = *output_ring_buffer_sample;
                    // Very important, or else we'll overlap-add ourselves into a feedback hell
                    *output_ring_buffer_sample = 0.0;
                }
            }

            // And for the sidechain buffers we only need to copy the inputs
            for (sidechain_buffer, sidechain_ring_buffers) in sidechain_buffers
                .iter()
                .zip(self.sidechain_ring_buffers.iter_mut())
            {
                for sample_offset in 0..samples_to_process {
                    for channel_idx in 0..num_channels {
                        let sample = unsafe {
                            sidechain_buffer.get_sample_unchecked(
                                channel_idx,
                                already_processed_samples + sample_offset,
                            )
                        };
                        let ring_buffer_sample = unsafe {
                            sidechain_ring_buffers
                                .get_unchecked_mut(channel_idx)
                                .get_unchecked_mut(self.current_pos + sample_offset)
                        };
                        *ring_buffer_sample = sample;
                    }
                }
            }

            already_processed_samples += samples_to_process;
            self.current_pos = (self.current_pos + samples_to_process) % block_size;

            // At this point we either have `already_processed_samples == main_buffer_len`, or
            // `self.current_pos % window_interval == 0`. If it's the latter, then we can process a
            // new block.
            if samples_to_process == samples_until_next_window {
                // Because we're processing in smaller windows, the input ring buffers sadly does
                // not always contain the full contiguous range we're interested in because they map
                // wrap around. Because premade FFT algorithms typically can't handle this, we'll
                // start with copying the wrapped ranges from our ring buffers to the scratch
                // buffer. Then we apply the windowing function and this it along to
                for (sidechain_idx, sidechain_ring_buffers) in
                    self.sidechain_ring_buffers.iter().enumerate()
                {
                    for (channel_idx, sidechain_ring_buffer) in
                        sidechain_ring_buffers.iter().enumerate()
                    {
                        copy_ring_to_scratch_buffer(
                            &mut self.scratch_buffer,
                            self.current_pos,
                            sidechain_ring_buffer,
                        );
                        if self.padding > 0 {
                            self.scratch_buffer[block_size..].fill(0.0);
                        }

                        process_cb(channel_idx, Some(sidechain_idx), &mut self.scratch_buffer);
                    }
                }

                for (channel_idx, ((input_ring_buffer, output_ring_buffer), padding_buffer)) in self
                    .main_input_ring_buffers
                    .iter()
                    .zip(self.main_output_ring_buffers.iter_mut())
                    .zip(self.padding_buffers.iter_mut())
                    .enumerate()
                {
                    copy_ring_to_scratch_buffer(
                        &mut self.scratch_buffer,
                        self.current_pos,
                        input_ring_buffer,
                    );
                    if self.padding > 0 {
                        self.scratch_buffer[block_size..].fill(0.0);
                    }

                    process_cb(channel_idx, None, &mut self.scratch_buffer);

                    // Add the padding from the last iteration (for this channel) to the scratch
                    // buffer before it is copied to the output ring buffer. In case the padding is
                    // longer than the block size, then this will cause everything else to be
                    // shifted to the left so it can be added in the iteration after this.
                    if self.padding > 0 {
                        let padding_to_copy = cmp::min(self.padding, block_size);
                        for (scratch_sample, padding_sample) in self.scratch_buffer
                            [..padding_to_copy]
                            .iter_mut()
                            .zip(&mut padding_buffer[..padding_to_copy])
                        {
                            *scratch_sample += *padding_sample;
                        }

                        // Any remaining padding tail should be moved towards the start of the
                        // buffer
                        padding_buffer.copy_within(padding_to_copy.., 0);

                        // And we obviously don't want this to feedback
                        padding_buffer[self.padding - padding_to_copy..].fill(0.0);
                    }

                    // The actual overlap-add part of the equation
                    add_scratch_to_ring_buffer(
                        &self.scratch_buffer,
                        self.current_pos,
                        output_ring_buffer,
                    );

                    // And the data from the padding area should be saved so it can be added to next
                    // iteration's scratch buffer. Like mentioned above, the padding can be larger
                    // than the block size so we also need to do overlap-add here.
                    if self.padding > 0 {
                        for (padding_sample, scratch_sample) in padding_buffer
                            .iter_mut()
                            .zip(&mut self.scratch_buffer[block_size..])
                        {
                            *padding_sample += *scratch_sample;
                        }
                    }
                }
            }
        }
    }

    /// Similar to [`process_overlap_add()`][Self::process_overlap_add()], but without the inverse
    /// STFT part. `buffer` will only ever be read from. This can be useful for providing FFT data
    /// for a spectrum analyzer in a plugin GUI. These is still a delay to the analysis equal to the
    /// block size.
    pub fn process_analyze_only<B, F>(
        &mut self,
        buffer: &B,
        overlap_times: usize,
        mut analyze_cb: F,
    ) where
        B: StftInput,
        F: FnMut(usize, &mut [f32]),
    {
        assert_eq!(buffer.num_channels(), self.main_input_ring_buffers.len());
        assert!(overlap_times > 0);

        // See `process_overlap_add_sidechain` for an annotated version
        let main_buffer_len = buffer.num_samples();
        let num_channels = buffer.num_channels();
        let block_size = self.main_input_ring_buffers[0].len();
        let window_interval = (block_size / overlap_times) as i32;
        let mut already_processed_samples = 0;
        while already_processed_samples < main_buffer_len {
            let remaining_samples = main_buffer_len - already_processed_samples;
            let samples_until_next_window = ((window_interval - self.current_pos as i32 - 1)
                .rem_euclid(window_interval)
                + 1) as usize;
            let samples_to_process = samples_until_next_window.min(remaining_samples);

            for sample_offset in 0..samples_to_process {
                for channel_idx in 0..num_channels {
                    let sample = unsafe {
                        buffer.get_sample_unchecked(
                            channel_idx,
                            already_processed_samples + sample_offset,
                        )
                    };
                    let input_ring_buffer_sample = unsafe {
                        self.main_input_ring_buffers
                            .get_unchecked_mut(channel_idx)
                            .get_unchecked_mut(self.current_pos + sample_offset)
                    };
                    *input_ring_buffer_sample = sample;
                }
            }

            already_processed_samples += samples_to_process;
            self.current_pos = (self.current_pos + samples_to_process) % block_size;

            if samples_to_process == samples_until_next_window {
                for (channel_idx, input_ring_buffer) in
                    self.main_input_ring_buffers.iter().enumerate()
                {
                    copy_ring_to_scratch_buffer(
                        &mut self.scratch_buffer,
                        self.current_pos,
                        input_ring_buffer,
                    );
                    if self.padding > 0 {
                        self.scratch_buffer[block_size..].fill(0.0);
                    }

                    analyze_cb(channel_idx, &mut self.scratch_buffer);
                }
            }
        }
    }

    fn update_buffers(&mut self, block_size: usize) {
        for main_ring_buffer in &mut self.main_input_ring_buffers {
            main_ring_buffer.resize(block_size, 0.0);
            main_ring_buffer.fill(0.0);
        }
        for main_ring_buffer in &mut self.main_output_ring_buffers {
            main_ring_buffer.resize(block_size, 0.0);
            main_ring_buffer.fill(0.0);
        }
        for sidechain_ring_buffers in &mut self.sidechain_ring_buffers {
            for sidechain_ring_buffer in sidechain_ring_buffers {
                sidechain_ring_buffer.resize(block_size, 0.0);
                sidechain_ring_buffer.fill(0.0);
            }
        }
        self.scratch_buffer.resize(block_size + self.padding, 0.0);
        self.scratch_buffer.fill(0.0);

        for padding_buffer in &mut self.padding_buffers {
            // In case this changed since the last call, like in `set_padding()`
            padding_buffer.resize(self.padding, 0.0);
            padding_buffer.fill(0.0);
        }

        self.current_pos = 0;
    }
}

/// Copy data from the the specified ring buffer (borrowed from `self`) to the scratch buffers at
/// the current position. This is a free function because you cannot pass an immutable reference to
/// a field from `&self` to a `&mut self` method.
#[inline]
fn copy_ring_to_scratch_buffer(
    scratch_buffer: &mut [f32],
    current_pos: usize,
    ring_buffer: &[f32],
) {
    let block_size = ring_buffer.len();
    let num_copy_before_wrap = block_size - current_pos;
    scratch_buffer[0..num_copy_before_wrap].copy_from_slice(&ring_buffer[current_pos..block_size]);
    scratch_buffer[num_copy_before_wrap..block_size].copy_from_slice(&ring_buffer[0..current_pos]);
}

/// Add data from the scratch buffer to the specified ring buffer. When writing samples from this
/// ring buffer back to the host's outputs they must be cleared to prevent infinite feedback.
#[inline]
fn add_scratch_to_ring_buffer(scratch_buffer: &[f32], current_pos: usize, ring_buffer: &mut [f32]) {
    // TODO: This could also use some SIMD
    let block_size = ring_buffer.len();
    let num_copy_before_wrap = block_size - current_pos;
    for (scratch_sample, ring_sample) in scratch_buffer[0..num_copy_before_wrap]
        .iter()
        .zip(&mut ring_buffer[current_pos..block_size])
    {
        *ring_sample += *scratch_sample;
    }
    for (scratch_sample, ring_sample) in scratch_buffer[num_copy_before_wrap..block_size]
        .iter()
        .zip(&mut ring_buffer[0..current_pos])
    {
        *ring_sample += *scratch_sample;
    }
}
