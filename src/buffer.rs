//! Adapters and utilities for working with audio buffers.

use std::marker::PhantomData;

mod blocks;
mod samples;

pub use blocks::{Block, BlockChannelsIter, BlocksIter};
pub use samples::{ChannelSamples, ChannelSamplesIter, SamplesIter};

/// The audio buffers used during processing. This contains the output audio output buffers with the
/// inputs already copied to the outputs. You can either use the iterator adapters to conveniently
/// and efficiently iterate over the samples, or you can do your own thing using the raw audio
/// buffers.
///
/// TODO: This lifetime makes zero sense because you're going to need unsafe lifetime casts to use
///       this either way. Maybe just get rid of it in favor for raw pointers.
#[derive(Default)]
pub struct Buffer<'a> {
    /// The number of samples contained within `output_slices`. This needs to be stored separately
    /// to be able to handle 0 channel IO for MIDI-only plugins.
    num_samples: usize,

    /// Contains slices for the plugin's outputs. You can't directly create a nested slice from a
    /// pointer to pointers, so this needs to be preallocated in the setup call and kept around
    /// between process calls. And because storing a reference to this means a) that you need a lot
    /// of lifetime annotations everywhere and b) that at some point you need unsound lifetime casts
    /// because this `Buffers` either cannot have the same lifetime as the separately stored output
    /// buffers, and it also cannot be stored in a field next to it because that would mean
    /// containing mutable references to data stored in a mutex.
    output_slices: Vec<&'a mut [f32]>,
}

impl<'a> Buffer<'a> {
    /// Returns the number of samples per channel in this buffer.
    #[inline]
    pub fn samples(&self) -> usize {
        self.num_samples
    }

    /// Returns the number of channels in this buffer.
    #[inline]
    pub fn channels(&self) -> usize {
        self.output_slices.len()
    }

    /// Returns true if this buffer does not contain any samples.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.num_samples == 0
    }

    /// Obtain the raw audio buffers.
    #[inline]
    pub fn as_slice(&mut self) -> &mut [&'a mut [f32]] {
        &mut self.output_slices
    }

    /// The same as [`as_slice()`][Self::as_slice()], but for a non-mutable reference. This is
    /// usually not needed.
    #[inline]
    pub fn as_slice_immutable(&self) -> &[&'a mut [f32]] {
        &self.output_slices
    }

    /// Iterate over the samples, returning a channel iterator for each sample.
    #[inline]
    pub fn iter_samples<'slice>(&'slice mut self) -> SamplesIter<'slice, 'a> {
        SamplesIter {
            buffers: self.output_slices.as_mut_slice(),
            current_sample: 0,
            samples_end: self.samples(),
            _marker: PhantomData,
        }
    }

    /// Iterate over the buffer in blocks with the specified maximum size. The ideal maximum block
    /// size depends on the plugin in question, but 64 or 128 samples works for most plugins. Since
    /// the buffer's total size may not be cleanly divisible by the maximum size, the returned
    /// buffers may have any size in `[1, max_block_size]`. This is useful when using algorithms
    /// that work on entire blocks of audio, like those that would otherwise need to perform
    /// expensive per-sample branching or that can use per-sample SIMD as opposed to per-channel
    /// SIMD.
    ///
    /// The parameter smoothers can also produce smoothed values for an entire block using
    /// [`Smoother::next_block()`][crate::prelude::Smoother::next_block()].
    ///
    /// You can use this to obtain block-slices from a buffer so you can pass them to a library:
    ///
    /// ```ignore
    /// for block in buffer.iter_blocks(128) {
    ///     let mut block_channels = block.into_iter();
    ///     let stereo_slice = &[
    ///         block_channels.next().unwrap(),
    ///         block_channels.next().unwrap(),
    ///     ];
    ///
    ///     // Do something cool with `stereo_slice`
    /// }
    /// ````
    #[inline]
    pub fn iter_blocks<'slice>(&'slice mut self, max_block_size: usize) -> BlocksIter<'slice, 'a> {
        BlocksIter {
            buffers: self.output_slices.as_mut_slice(),
            max_block_size,
            current_block_start: 0,
            _marker: PhantomData,
        }
    }

    /// Set the slices in the raw output slice vector. This vector needs to be resized to match the
    /// number of output channels during the plugin's initialization. Then during audio processing,
    /// these slices should be updated to point to the plugin's audio buffers. The `num_samples`
    /// argument should match the length of the inner slices.
    ///
    /// # Safety
    ///
    /// The stored slices must point to live data when this object is passed to the plugins' process
    /// function. The rest of this object also assumes all channel lengths are equal. Panics will
    /// likely occur if this is not the case.
    pub unsafe fn set_slices(
        &mut self,
        num_samples: usize,
        update: impl FnOnce(&mut Vec<&'a mut [f32]>),
    ) {
        self.num_samples = num_samples;
        update(&mut self.output_slices);

        #[cfg(debug_assertions)]
        for slice in &self.output_slices {
            nih_debug_assert_eq!(slice.len(), num_samples);
        }
    }
}

#[cfg(any(miri, test))]
mod miri {
    use super::*;

    #[test]
    fn repeated_access() {
        let mut real_buffers = vec![vec![0.0; 512]; 2];
        let mut buffer = Buffer::default();
        unsafe {
            buffer.set_slices(512, |output_slices| {
                let (first_channel, other_channels) = real_buffers.split_at_mut(1);
                *output_slices = vec![&mut first_channel[0], &mut other_channels[0]];
            })
        };

        for samples in buffer.iter_samples() {
            for sample in samples {
                *sample += 0.001;
            }
        }

        for mut samples in buffer.iter_samples() {
            for _ in 0..2 {
                for sample in samples.iter_mut() {
                    *sample += 0.001;
                }
            }
        }

        assert_eq!(real_buffers[0][0], 0.003);
    }

    #[test]
    fn repeated_slices() {
        let mut real_buffers = vec![vec![0.0; 512]; 2];
        let mut buffer = Buffer::default();
        unsafe {
            buffer.set_slices(512, |output_slices| {
                let (first_channel, other_channels) = real_buffers.split_at_mut(1);
                *output_slices = vec![&mut first_channel[0], &mut other_channels[0]];
            })
        };

        // These iterators should not alias
        let mut blocks = buffer.iter_blocks(16);
        let (_block1_offset, block1) = blocks.next().unwrap();
        let (_block2_offset, block2) = blocks.next().unwrap();
        for channel in block1 {
            for sample in channel.iter_mut() {
                *sample += 0.001;
            }
        }
        for channel in block2 {
            for sample in channel.iter_mut() {
                *sample += 0.001;
            }
        }

        for i in 0..32 {
            assert_eq!(real_buffers[0][i], 0.001);
        }
        for i in 32..48 {
            assert_eq!(real_buffers[0][i], 0.0);
        }
    }
}
