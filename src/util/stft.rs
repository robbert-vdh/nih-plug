//! Utilities for buffering audio, likely used as part of a short-term Fourier transform.

use super::window::multiply_with_window;
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

    /// The current position in our ring buffers. Whenever this wraps around to 0, we'll process
    /// a block.
    current_pos: usize,
}

/// Marker struct for the version wtihout sidechaining.
struct NoSidechain;

impl StftInput for Buffer<'_> {
    #[inline]
    fn num_samples(&self) -> usize {
        self.len()
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
        self.len()
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
    /// given maximum block size. Call [`set_block_size()`][`Self::set_block_size()`] afterwards if
    /// you do not need the full capacity upfront.
    ///
    /// # Panics
    ///
    /// Panics if `num_channels == 0 || max_block_size == 0`.
    pub fn new(num_channels: usize, max_block_size: usize) -> Self {
        assert_ne!(num_channels, 0);
        assert_ne!(max_block_size, 0);

        Self {
            main_input_ring_buffers: vec![vec![0.0; max_block_size]; num_channels],
            main_output_ring_buffers: vec![vec![0.0; max_block_size]; num_channels],
            // Kinda hacky way to initialize an array of non-copy types
            sidechain_ring_buffers: [(); NUM_SIDECHAIN_INPUTS]
                .map(|_| vec![vec![0.0; max_block_size]; num_channels]),

            scratch_buffer: vec![0.0; max_block_size],

            current_pos: 0,
        }
    }

    /// Change the current block size. This will clear the buffers, causing the next block to output
    /// silence.
    ///
    /// # Panics
    ///
    /// WIll panic if `block_size > max_block_size`.
    pub fn set_block_size(&mut self, block_size: usize) {
        assert!(block_size <= self.main_input_ring_buffers[0].capacity());

        for main_ring_buffer in &mut self.main_input_ring_buffers {
            main_ring_buffer.resize(block_size, 0.0);
            main_ring_buffer.fill(0.0);
        }
        for main_ring_buffer in &mut self.main_output_ring_buffers {
            main_ring_buffer.resize(block_size, 0.0);
            main_ring_buffer.fill(0.0);
        }
        self.scratch_buffer.resize(block_size, 0.0);
        self.scratch_buffer.fill(0.0);
        for sidechain_ring_buffers in &mut self.sidechain_ring_buffers {
            for sidechain_ring_buffer in sidechain_ring_buffers {
                sidechain_ring_buffer.resize(block_size, 0.0);
                sidechain_ring_buffer.fill(0.0);
            }
        }

        self.current_pos = 0;
    }

    /// The amount of latency introduced when processing audio throug hthis [`StftHelper`].
    pub fn latency_samples(&self) -> u32 {
        self.main_input_ring_buffers[0].len() as u32
    }

    /// Process the audio in `main_buffer` in small overlapping blocks with a window function
    /// applied, adding up the results for the main buffer so they can be written back to the host.
    /// The window overlap amount is compensated automatically when adding up these samples.
    /// Whenever a new block is available, `process_cb()` gets called with a new audio block of the
    /// specified size with the windowing function already applied. The summed reults will then be
    /// written back to `main_buffer` exactly one block later, which means that this function will
    /// introduce one block of latency. This can be compensated by calling
    /// [`ProcessContext::set_latency()`][`crate::prelude::ProcessContext::set_latency_samples()`]
    /// in your plugin's initialization function.
    ///
    /// This function does not apply any gain compensation for the windowing. You will need to do
    /// that yoruself depending on your window function and the amount of overlap.
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
    /// channels as this [`StftHelper`], if the sidechain buffers do not contain the same number of
    /// samples as the main buffer, or if the window function does not match the block size.
    ///
    /// TODO: Add more useful ways to do STFT and other buffered operations. I just went with this
    ///       approach because it's what I needed myself, but generic combinators like this could
    ///       also be useful for other operations.
    pub fn process_overlap_add<M, F>(
        &mut self,
        main_buffer: &mut M,
        window_function: &[f32],
        overlap_times: usize,
        mut process_cb: F,
    ) where
        M: StftInputMut,
        F: FnMut(usize, &mut [f32]),
    {
        self.process_overlap_add_sidechain(
            main_buffer,
            [&NoSidechain; NUM_SIDECHAIN_INPUTS],
            window_function,
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
        window_function: &[f32],
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
        assert_eq!(window_function.len(), self.main_input_ring_buffers[0].len());
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
                        multiply_with_window(&mut self.scratch_buffer, window_function);
                        process_cb(channel_idx, Some(sidechain_idx), &mut self.scratch_buffer);
                    }
                }

                for (channel_idx, (input_ring_buffer, output_ring_buffer)) in self
                    .main_input_ring_buffers
                    .iter()
                    .zip(self.main_output_ring_buffers.iter_mut())
                    .enumerate()
                {
                    copy_ring_to_scratch_buffer(
                        &mut self.scratch_buffer,
                        self.current_pos,
                        input_ring_buffer,
                    );
                    multiply_with_window(&mut self.scratch_buffer, window_function);
                    process_cb(channel_idx, None, &mut self.scratch_buffer);

                    // The actual overlap-add part of the equation
                    multiply_with_window(&mut self.scratch_buffer, window_function);
                    add_scratch_to_ring_buffer(
                        &self.scratch_buffer,
                        self.current_pos,
                        output_ring_buffer,
                    );
                }
            }
        }
    }

    /// Similar to [`process_overlap_add()`][Self::process_overlap_add()], but without the inverse
    /// STFT part. `buffer` will only ever be read from. This can be useful for providing FFT data
    /// for a spectrum analyzer in a plugin GUI. These is still a delay to the analysis equal to the
    /// blcok size.
    pub fn process_analyze_only<B, F>(
        &mut self,
        buffer: &B,
        window_function: &[f32],
        overlap_times: usize,
        mut analyze_cb: F,
    ) where
        B: StftInput,
        F: FnMut(usize, &mut [f32]),
    {
        assert_eq!(buffer.num_channels(), self.main_input_ring_buffers.len());
        assert_eq!(window_function.len(), self.main_input_ring_buffers[0].len());
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
                    multiply_with_window(&mut self.scratch_buffer, window_function);
                    analyze_cb(channel_idx, &mut self.scratch_buffer);
                }
            }
        }
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
