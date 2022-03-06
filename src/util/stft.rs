//! Utilities for buffering audio, likely used as part of a short-term Fourier transform.

use std::mem;

use crate::buffer::Buffer;

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
    // These ring buffers store both the input samples and the already processed output. Whenever we
    // wrap around,we'll write the already calculated outputs to the main buffer passed to the
    // process function and process a new block.
    main_ring_buffers: Vec<Vec<f32>>,
    sidechain_ring_buffers: [Vec<Vec<f32>>; NUM_SIDECHAIN_INPUTS],

    // To make this more convenient, we'll provide slices into the above buffers to the block
    // process callback
    main_block_buffer: Buffer<'static>,
    sidechain_block_buffers: [Buffer<'static>; NUM_SIDECHAIN_INPUTS],

    /// The current position in our ring buffers. Whenever this wraps around to 0, we'll process
    /// a block.
    current_pos: usize,
}

impl<const NUM_SIDECHAIN_INPUTS: usize> StftHelper<NUM_SIDECHAIN_INPUTS> {
    /// Initialize the [`StftHelper`] for [`Buffer`]s with the specified number of channels and the
    /// given maximum block size. Call [`set_block_size()`][`Self::set_block_size()`] afterwards if
    /// you do not need the full capacity upfront.
    pub fn new(num_channels: usize, max_block_size: usize) -> Self {
        nih_debug_assert_ne!(num_channels, 0);
        nih_debug_assert_ne!(max_block_size, 0);

        let mut helper = Self {
            main_ring_buffers: vec![vec![0.0; max_block_size]; num_channels],
            // Kinda hacky way to initialize an array of non-copy types
            sidechain_ring_buffers: [(); NUM_SIDECHAIN_INPUTS]
                .map(|_| vec![vec![0.0; max_block_size]; num_channels]),

            main_block_buffer: Buffer::default(),
            sidechain_block_buffers: [(); NUM_SIDECHAIN_INPUTS].map(|_| Buffer::default()),

            current_pos: 0,
        };

        // Preallocate the output slices. We'll point them to the ring buffers at the start of the
        // process call.
        unsafe {
            helper.main_block_buffer.with_raw_vec(|main_block_slices| {
                main_block_slices.resize_with(num_channels, || &mut [])
            });
            for sidechain_block_buffer in &mut helper.sidechain_block_buffers {
                sidechain_block_buffer.with_raw_vec(|main_block_slices| {
                    main_block_slices.resize_with(num_channels, || &mut [])
                });
            }
        };

        helper
    }

    /// Change the current block size. This will clear the buffers, causing the next block to output
    /// silence.
    ///
    /// # Panics
    ///
    /// WIll panic if `block_size > max_block_size`.
    pub fn set_block_size(&mut self, block_size: usize) {
        assert!(block_size <= self.main_ring_buffers[0].capacity());

        for main_ring_buffer in &mut self.main_ring_buffers {
            main_ring_buffer.resize(block_size, 0.0);
            main_ring_buffer.fill(0.0);
        }
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
        self.main_ring_buffers[0].len() as u32
    }

    /// Process the audio in `main_buffer` and in any sidechain buffers in small blocks. Whenever a
    /// new block is available, `process_cb()` gets called with a new audio block of the specified
    /// side. The results written to the buffer will then be written back to `main_buffer` exactly
    /// one block later, which means that this function will introduce one block of latency. This
    /// can be compensated by calling
    /// [`ProcessContext::set_latency()`][`crate::prelude::ProcessContext::set_latency()`] in your
    /// plugin's initialization function.
    ///
    /// # Panics
    ///
    /// Panics if `main_buffer` or the buffers in `sidechain_buffers` do not have the same number of
    /// channels as this [`StftHelper`].
    ///
    /// TODO: Maybe introduce a trait here so this can be used with things that aren't whole buffers
    /// TODO: And also introduce that aforementioned read-only process function (`analyze()?`)
    pub fn process<F>(
        &mut self,
        main_buffer: &mut Buffer,
        sidechain_buffers: [&Buffer; NUM_SIDECHAIN_INPUTS],
        mut process_cb: F,
    ) where
        F: FnMut(&mut Buffer, &[Buffer; NUM_SIDECHAIN_INPUTS]),
    {
        assert_eq!(main_buffer.channels(), self.main_ring_buffers.len());

        // Since the `StftHelper` object may move in between process calls, we need to make sure
        // that these slices point to our ring buffers at the start of each call
        unsafe {
            self.main_block_buffer.with_raw_vec(|main_block_slices| {
                assert_eq!(main_block_slices.len(), self.main_ring_buffers.len());
                for (channel_idx, channel_slice) in main_block_slices.iter_mut().enumerate() {
                    // SAFETY: This is equivalent to splitting on each channel, and these block
                    //         slices will only be used here as part of the callback when the ring
                    //         buffers are not mutably borrwed
                    *channel_slice =
                        &mut *(self.main_ring_buffers[channel_idx].as_mut_slice() as *mut _);
                }
            });
            for (sidechain_block_buffer, sidechain_ring_buffer) in self
                .sidechain_block_buffers
                .iter_mut()
                .zip(self.sidechain_ring_buffers.iter_mut())
            {
                sidechain_block_buffer.with_raw_vec(|sidechain_block_slices| {
                    assert_eq!(sidechain_block_slices.len(), sidechain_ring_buffer.len());
                    for (channel_idx, channel_slice) in
                        sidechain_block_slices.iter_mut().enumerate()
                    {
                        *channel_slice =
                            &mut *(sidechain_ring_buffer[channel_idx].as_mut_slice() as *mut _);
                    }
                });
            }
        };

        // We'll copy samples from `*_buffer` into `*_ring_buffers` while simultaneously copying
        // already processed samples from `main_ring_buffers` in into `main_buffer`
        let main_buffer_len = main_buffer.len();
        let num_channels = main_buffer.channels();
        let block_len = self.main_ring_buffers[0].len();
        let mut already_processed_samples = 0;
        while already_processed_samples < main_buffer_len {
            let remaining_samples = main_buffer_len - already_processed_samples;
            let samples_until_next_block = block_len - self.current_pos;
            let samples_to_process = samples_until_next_block.min(remaining_samples);

            // Copy the input from `main_buffer` to the ring buffer while copying last block's
            // result from the buffer to `main_buffer`
            // TODO: This might be able to be sped up a bit with SIMD
            {
                // For the main buffer
                let main_buffer = main_buffer.as_slice();
                for sample_offset in 0..samples_to_process {
                    for channel_idx in 0..num_channels {
                        let sample = unsafe {
                            main_buffer
                                .get_unchecked_mut(channel_idx)
                                .get_unchecked_mut(already_processed_samples + sample_offset)
                        };
                        let ring_buffer_sample = unsafe {
                            self.main_ring_buffers
                                .get_unchecked_mut(channel_idx)
                                .get_unchecked_mut(self.current_pos + sample_offset)
                        };
                        mem::swap(sample, ring_buffer_sample);
                    }
                }

                // And for the sidechain buffers we only need to copy the inputs
                for (sidechain_buffer, sidechain_ring_buffers) in sidechain_buffers
                    .iter()
                    .zip(self.sidechain_ring_buffers.iter_mut())
                {
                    let sidechain_buffer = sidechain_buffer.as_slice_immutable();
                    for sample_offset in 0..samples_to_process {
                        for channel_idx in 0..num_channels {
                            let sample = unsafe {
                                sidechain_buffer
                                    .get_unchecked(channel_idx)
                                    .get_unchecked(already_processed_samples + sample_offset)
                            };
                            let ring_buffer_sample = unsafe {
                                sidechain_ring_buffers
                                    .get_unchecked_mut(channel_idx)
                                    .get_unchecked_mut(self.current_pos + sample_offset)
                            };
                            *ring_buffer_sample = *sample;
                        }
                    }
                }
            }

            already_processed_samples += samples_to_process;
            self.current_pos += samples_to_process;

            // At this point we either have `already_processed_samples == main_buffer_len`, or
            // `self.current_pos == block_len`. If it's the latter, then we can process a new block.
            if self.current_pos == block_len {
                process_cb(&mut self.main_block_buffer, &self.sidechain_block_buffers);

                self.current_pos = 0;
            }
        }
    }
}
