use std::marker::PhantomData;

#[cfg(feature = "simd")]
use std::simd::{LaneCount, Simd, SupportedLaneCount};

// TODO: Does adding `#[inline]` to the .next() functions make any difference?

/// The audio buffers used during processing. This contains the output audio output buffers with the
/// inputs already copied to the outputs. You can either use the iterator adapters to conveniently
/// and efficiently iterate over the samples, or you can do your own thing using the raw audio
/// buffers.
#[derive(Default)]
pub struct Buffer<'a> {
    /// Contains slices for the plugin's outputs. You can't directly create a nested slice form
    /// apointer to pointers, so this needs to be preallocated in the setup call and kept around
    /// between process calls. And because storing a reference to this means a) that you need a lot
    /// of lifetime annotations everywhere and b) that at some point you need unsound lifetime casts
    /// because this `Buffers` either cannot have the same lifetime as the separately stored output
    /// buffers, and it also cannot be stored in a field next to it because that would mean
    /// containing mutable references to data stored in a mutex.
    output_slices: Vec<&'a mut [f32]>,
}

// Per-sample per-channel iterators

/// An iterator over all samples in the buffer, yielding iterators over each channel for every
/// sample. This iteration order offers good cache locality for per-sample access.
pub struct SamplesIter<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_sample: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

/// Can construct iterators over actual iterator over the channel data for a sample, yielded by
/// [Samples]. Can be turned into an iterator, or [Channels::iter_mut()] can be used to iterate over
/// the channel data multiple times, or more efficiently you can use [Channels::get_unchecked_mut()]
/// to do the same thing.
pub struct Channels<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_sample: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

/// The actual iterator over the channel data for a sample, yielded by [Channels].
pub struct ChannelsIter<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_sample: usize,
    pub(self) current_channel: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

// Per-block per-channel per-sample iterators

/// An iterator over all samples in the buffer, slicing over the sample-dimension with a maximum
/// size of [Self::max_block_size]. See [Buffer::iter_blocks()]. Yields both the block and the
/// offset from the start of the buffer.
pub struct BlocksIter<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) max_block_size: usize,
    pub(self) current_block_start: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

/// A block yielded by [BlocksIter]. Can be iterated over once or multiple times, and also supports
/// direct access to the block's samples if needed.
pub struct Block<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_block_start: usize,
    pub(self) current_block_end: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

/// An iterator over all channels in a block yielded by [Block]. Analogous to [ChannelsIter] but for
/// blocks.
pub struct BlockChannelsIter<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_block_start: usize,
    pub(self) current_block_end: usize,
    pub(self) current_channel: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

impl<'slice, 'sample> Iterator for SamplesIter<'slice, 'sample> {
    type Item = Channels<'slice, 'sample>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_sample < unsafe { (*self.buffers)[0].len() } {
            let channels = Channels {
                buffers: self.buffers,
                current_sample: self.current_sample,
                _marker: self._marker,
            };

            self.current_sample += 1;

            Some(channels)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = unsafe { (*self.buffers)[0].len() } - self.current_sample;
        (remaining, Some(remaining))
    }
}

impl<'slice, 'sample> Iterator for BlockChannelsIter<'slice, 'sample> {
    type Item = &'sample mut [f32];

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_channel < unsafe { (*self.buffers).len() } {
            // SAFETY: These bounds have already been checked
            // SAFETY: It is also not possible to have multiple mutable references to the same
            //         sample at the same time
            let slice = unsafe {
                (*self.buffers)
                    .get_unchecked_mut(self.current_channel)
                    .get_unchecked_mut(self.current_block_start..self.current_block_end)
            };

            self.current_channel += 1;

            Some(slice)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = unsafe { (*self.buffers).len() } - self.current_channel;
        (remaining, Some(remaining))
    }
}

impl<'slice, 'sample> Iterator for BlocksIter<'slice, 'sample> {
    type Item = (usize, Block<'slice, 'sample>);

    fn next(&mut self) -> Option<Self::Item> {
        let buffer_len = unsafe { (*self.buffers)[0].len() };
        if self.current_block_start < buffer_len {
            let current_block_start = self.current_block_start;
            let current_block_end =
                (self.current_block_start + self.max_block_size).min(buffer_len);
            let block = Block {
                buffers: self.buffers,
                current_block_start,
                current_block_end,
                _marker: self._marker,
            };

            self.current_block_start += self.max_block_size;

            Some((current_block_start, block))
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = ((unsafe { (*self.buffers)[0].len() } - self.current_block_start) as f32
            / self.max_block_size as f32)
            .ceil() as usize;
        (remaining, Some(remaining))
    }
}

impl<'slice, 'sample> Iterator for ChannelsIter<'slice, 'sample> {
    type Item = &'sample mut f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_channel < unsafe { (*self.buffers).len() } {
            // SAFETY: These bounds have already been checked
            // SAFETY: It is also not possible to have multiple mutable references to the same
            // sample at the same time
            let sample = unsafe {
                (*self.buffers)
                    .get_unchecked_mut(self.current_channel)
                    .get_unchecked_mut(self.current_sample)
            };

            self.current_channel += 1;

            Some(sample)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = unsafe { (*self.buffers).len() } - self.current_channel;
        (remaining, Some(remaining))
    }
}

impl<'slice, 'sample> IntoIterator for Channels<'slice, 'sample> {
    type Item = &'sample mut f32;
    type IntoIter = ChannelsIter<'slice, 'sample>;

    fn into_iter(self) -> Self::IntoIter {
        ChannelsIter {
            buffers: self.buffers,
            current_sample: self.current_sample,
            current_channel: 0,
            _marker: self._marker,
        }
    }
}

impl<'slice, 'sample> IntoIterator for Block<'slice, 'sample> {
    type Item = &'sample mut [f32];
    type IntoIter = BlockChannelsIter<'slice, 'sample>;

    fn into_iter(self) -> Self::IntoIter {
        BlockChannelsIter {
            buffers: self.buffers,
            current_block_start: self.current_block_start,
            current_block_end: self.current_block_end,
            current_channel: 0,
            _marker: self._marker,
        }
    }
}

impl ExactSizeIterator for SamplesIter<'_, '_> {}
impl ExactSizeIterator for ChannelsIter<'_, '_> {}
impl ExactSizeIterator for BlocksIter<'_, '_> {}
impl ExactSizeIterator for BlockChannelsIter<'_, '_> {}

impl<'a> Buffer<'a> {
    /// Returns true if this buffer does not contain any samples.
    pub fn is_empty(&self) -> bool {
        self.output_slices.is_empty() || self.output_slices[0].is_empty()
    }

    /// Obtain the raw audio buffers.
    pub fn as_slice(&mut self) -> &mut [&'a mut [f32]] {
        &mut self.output_slices
    }

    /// Iterate over the samples, returning a channel iterator for each sample.
    pub fn iter_mut<'slice>(&'slice mut self) -> SamplesIter<'slice, 'a> {
        SamplesIter {
            buffers: self.output_slices.as_mut_slice(),
            current_sample: 0,
            _marker: PhantomData,
        }
    }

    /// Iterate over the buffer in blocks with the specified maximum size. The ideal maximum block
    /// size depends on the plugin in question, but 64 or 128 samples works for most plugins. Since
    /// the buffer's total size may not be cleanly divisble by the maximum size, the returned
    /// buffers may have any size in `[1, max_block_size]`. This is useful when using algorithms
    /// that work on entire blocks of audio, like those that would otherwise need to perform
    /// expensive per-sample branching or that can use per-sample SIMD as opposed to per-channel
    /// SIMD.
    ///
    /// The parameter smoothers can also produce smoothed values for an entire block using
    /// [crate::Smoother::next_block()]. Before using this, you will need to call
    /// [crate::Plugin::initialize_block_smoothers()] with the same `max_block_size` in your
    /// initialization function first.
    ///
    /// You can use this to obtain block-slices from a buffer so you can pass them to a libraryq:
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
    pub fn iter_blocks<'slice>(&'slice mut self, max_block_size: usize) -> BlocksIter<'slice, 'a> {
        BlocksIter {
            buffers: self.output_slices.as_mut_slice(),
            max_block_size,
            current_block_start: 0,
            _marker: PhantomData,
        }
    }

    /// Access the raw output slice vector. This needs to be resized to match the number of output
    /// channels during the plugin's initialization. Then during audio processing, these slices
    /// should be updated to point to the plugin's audio buffers.
    ///
    /// # Safety
    ///
    /// The stored slices must point to live data when this object is passed to the plugins' process
    /// function. The rest of this object also assumes all channel lengths are equal. Panics will
    /// likely occur if this is not the case.
    pub unsafe fn with_raw_vec(&mut self, update: impl FnOnce(&mut Vec<&'a mut [f32]>)) {
        update(&mut self.output_slices);
    }
}

impl<'slice, 'sample> Channels<'slice, 'sample> {
    /// Get the number of channels.
    pub fn len(&self) -> usize {
        unsafe { (*self.buffers).len() }
    }

    /// A resetting iterator. This lets you iterate over the same channels multiple times. Otherwise
    /// you don't need to use this function as [Channels] already implements [Iterator].
    pub fn iter_mut(&mut self) -> ChannelsIter<'slice, 'sample> {
        ChannelsIter {
            buffers: self.buffers,
            current_sample: self.current_sample,
            current_channel: 0,
            _marker: self._marker,
        }
    }

    /// Access a sample by index. Useful when you would otherwise iterate over this 'Channels'
    /// iterator multiple times.
    #[inline]
    pub fn get_mut(&mut self, channel_index: usize) -> Option<&mut f32> {
        // SAFETY: The sample bound has already been checked
        unsafe {
            Some(
                (*self.buffers)
                    .get_mut(channel_index)?
                    .get_unchecked_mut(self.current_sample),
            )
        }
    }

    /// The same as [Self::get_mut], but without any bounds checking.
    ///
    /// # Safety
    ///
    /// `channel_index` must be in the range `0..Self::len()`.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, channel_index: usize) -> &mut f32 {
        (*self.buffers)
            .get_unchecked_mut(channel_index)
            .get_unchecked_mut(self.current_sample)
    }

    /// Get a SIMD vector containing the channel data for this buffer. If `LANES > channels.len()`
    /// then this will be padded with zeroes. If `LANES < channels.len()` then this won't contain
    /// all values.
    #[cfg(feature = "simd")]
    #[inline]
    pub fn to_simd<const LANES: usize>(&self) -> Simd<f32, LANES>
    where
        LaneCount<LANES>: SupportedLaneCount,
    {
        let used_lanes = self.len().max(LANES);
        let mut values = [0.0; LANES];
        for (channel_idx, value) in values.iter_mut().enumerate().take(used_lanes) {
            *value = unsafe {
                *(*self.buffers)
                    .get_unchecked(channel_idx)
                    .get_unchecked(self.current_sample)
            };
        }

        Simd::from_array(values)
    }

    /// Get a SIMD vector containing the channel data for this buffer. Will always read exactly
    /// `LANES` channels.
    ///
    /// # Safety
    ///
    /// Undefined behavior if `LANES > channels.len()`.
    #[cfg(feature = "simd")]
    #[inline]
    pub unsafe fn to_simd_unchecked<const LANES: usize>(&self) -> Simd<f32, LANES>
    where
        LaneCount<LANES>: SupportedLaneCount,
    {
        let mut values = [0.0; LANES];
        for (channel_idx, value) in values.iter_mut().enumerate() {
            *value = *(*self.buffers)
                .get_unchecked(channel_idx)
                .get_unchecked(self.current_sample);
        }

        Simd::from_array(values)
    }

    /// Write data from a SIMD vector to this sample's channel data. This takes the padding added by
    /// [Self::to_simd()] into account.
    #[cfg(feature = "simd")]
    #[allow(clippy::wrong_self_convention)]
    #[inline]
    pub fn from_simd<const LANES: usize>(&mut self, vector: Simd<f32, LANES>)
    where
        LaneCount<LANES>: SupportedLaneCount,
    {
        let used_lanes = self.len().max(LANES);
        let values = vector.to_array();
        for (channel_idx, value) in values.into_iter().enumerate().take(used_lanes) {
            *unsafe {
                (*self.buffers)
                    .get_unchecked_mut(channel_idx)
                    .get_unchecked_mut(self.current_sample)
            } = value;
        }
    }

    /// Write data from a SIMD vector to this sample's channel data. This assumes `LANES` matches
    /// exactly with the number of channels in the buffer.
    ///
    /// # Safety
    ///
    /// Undefined behavior if `LANES > channels.len()`.
    #[cfg(feature = "simd")]
    #[allow(clippy::wrong_self_convention)]
    #[inline]
    pub unsafe fn from_simd_unchecked<const LANES: usize>(&mut self, vector: Simd<f32, LANES>)
    where
        LaneCount<LANES>: SupportedLaneCount,
    {
        let values = vector.to_array();
        for (channel_idx, value) in values.into_iter().enumerate() {
            *(*self.buffers)
                .get_unchecked_mut(channel_idx)
                .get_unchecked_mut(self.current_sample) = value;
        }
    }
}

impl<'slice, 'sample> Block<'slice, 'sample> {
    /// Get the number of samples (not channels) in the block.
    pub fn len(&self) -> usize {
        self.current_block_end - self.current_block_start
    }

    /// A resetting iterator. This lets you iterate over the same block multiple times. Otherwise
    /// you don't need to use this function as [Block] already implements [Iterator]. You can also
    /// use the direct accessor functions on this block instead.
    pub fn iter_mut(&mut self) -> BlockChannelsIter<'slice, 'sample> {
        BlockChannelsIter {
            buffers: self.buffers,
            current_block_start: self.current_block_start,
            current_block_end: self.current_block_end,
            current_channel: 0,
            _marker: self._marker,
        }
    }

    /// Access a channel by index. Useful when you would otherwise iterate over this [Block]
    /// multiple times.
    #[inline]
    pub fn get_mut(&mut self, channel_index: usize) -> Option<&mut [f32]> {
        // SAFETY: The block bound has already been checked
        unsafe {
            Some(
                (*self.buffers)
                    .get_mut(channel_index)?
                    .get_unchecked_mut(self.current_block_start..self.current_block_end),
            )
        }
    }

    /// The same as [Self::get_mut], but without any bounds checking.
    ///
    /// # Safety
    ///
    /// `channel_index` must be in the range `0..Self::len()`.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, channel_index: usize) -> &mut [f32] {
        (*self.buffers)
            .get_unchecked_mut(channel_index)
            .get_unchecked_mut(self.current_block_start..self.current_block_end)
    }

    /// Get a SIMD vector containing the channel data for a specific sample in this block. If `LANES
    /// > channels.len()` then this will be padded with zeroes. If `LANES < channels.len()` then
    /// this won't contain all values.
    ///
    /// Returns a `None` value if `sample_index` is out of bounds.
    #[cfg(feature = "simd")]
    #[inline]
    pub fn to_simd<const LANES: usize>(&self, sample_index: usize) -> Option<Simd<f32, LANES>>
    where
        LaneCount<LANES>: SupportedLaneCount,
    {
        if sample_index > self.len() {
            return None;
        }

        let used_lanes = self.len().max(LANES);
        let mut values = [0.0; LANES];
        for (channel_idx, value) in values.iter_mut().enumerate().take(used_lanes) {
            *value = unsafe {
                *(*self.buffers)
                    .get_unchecked(channel_idx)
                    .get_unchecked(self.current_block_start + sample_index)
            };
        }

        Some(Simd::from_array(values))
    }

    /// Get a SIMD vector containing the channel data for a specific sample in this block. Will
    /// always read exactly `LANES` channels, and does not perform bounds checks on `sample_index`.
    ///
    /// # Safety
    ///
    /// Undefined behavior if `LANES > block.len()` or if `sample_index > block.len()`.
    #[cfg(feature = "simd")]
    #[inline]
    pub unsafe fn to_simd_unchecked<const LANES: usize>(
        &self,
        sample_index: usize,
    ) -> Simd<f32, LANES>
    where
        LaneCount<LANES>: SupportedLaneCount,
    {
        let mut values = [0.0; LANES];
        for (channel_idx, value) in values.iter_mut().enumerate() {
            *value = *(*self.buffers)
                .get_unchecked(channel_idx)
                .get_unchecked(self.current_block_start + sample_index);
        }

        Simd::from_array(values)
    }

    /// Write data from a SIMD vector to this sample's channel data for a specific sample in this
    /// block. This takes the padding added by [Self::to_simd()] into account.
    ///
    /// Returns `false` if `sample_index` is out of bounds.
    #[cfg(feature = "simd")]
    #[allow(clippy::wrong_self_convention)]
    #[inline]
    pub fn from_simd<const LANES: usize>(
        &mut self,
        sample_index: usize,
        vector: Simd<f32, LANES>,
    ) -> bool
    where
        LaneCount<LANES>: SupportedLaneCount,
    {
        if sample_index > self.len() {
            return false;
        }

        let used_lanes = self.len().max(LANES);
        let values = vector.to_array();
        for (channel_idx, value) in values.into_iter().enumerate().take(used_lanes) {
            *unsafe {
                (*self.buffers)
                    .get_unchecked_mut(channel_idx)
                    .get_unchecked_mut(self.current_block_start + sample_index)
            } = value;
        }

        true
    }

    /// Write data from a SIMD vector to this sample's channel data for a specific sample in this
    /// block.. This assumes `LANES` matches exactly with the number of channels in the buffer, and
    /// does not perform bounds checks on `sample_index`.
    ///
    /// # Safety
    ///
    /// Undefined behavior if `LANES > block.len()` or if `sample_index > block.len()`.
    #[cfg(feature = "simd")]
    #[allow(clippy::wrong_self_convention)]
    #[inline]
    pub unsafe fn from_simd_unchecked<const LANES: usize>(
        &mut self,
        sample_index: usize,
        vector: Simd<f32, LANES>,
    ) where
        LaneCount<LANES>: SupportedLaneCount,
    {
        let values = vector.to_array();
        for (channel_idx, value) in values.into_iter().enumerate() {
            *(*self.buffers)
                .get_unchecked_mut(channel_idx)
                .get_unchecked_mut(self.current_block_start + sample_index) = value;
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
            buffer.with_raw_vec(|output_slices| {
                let (first_channel, other_channels) = real_buffers.split_at_mut(1);
                *output_slices = vec![&mut first_channel[0], &mut other_channels[0]];
            })
        };

        for samples in buffer.iter_mut() {
            for sample in samples {
                *sample += 0.001;
            }
        }

        for mut samples in buffer.iter_mut() {
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
            buffer.with_raw_vec(|output_slices| {
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
