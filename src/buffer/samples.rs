//! Per-sample per-channel iterators.

use std::marker::PhantomData;

#[cfg(feature = "simd")]
use std::simd::{LaneCount, Simd, SupportedLaneCount};

/// An iterator over all samples in a buffer or block, yielding iterators over each channel for
/// every sample. This iteration order offers good cache locality for per-sample access.
pub struct SamplesIter<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(super) buffers: *mut [&'sample mut [f32]],
    pub(super) current_sample: usize,
    /// The last sample index to iterate over plus one. Would be equal to `buffers.len()` when
    /// iterating over an entire buffer, but this can also be used to iterate over smaller blocks in
    /// a similar fashion.
    pub(super) samples_end: usize,
    pub(super) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

/// Can construct iterators over actual iterator over the channel data for a sample, yielded by
/// [`SamplesIter`]. Can be turned into an iterator, or [`ChannelSamples::iter_mut()`] can be used
/// to iterate over the channel data multiple times, or more efficiently you can use
/// [`ChannelSamples::get_unchecked_mut()`] to do the same thing.
pub struct ChannelSamples<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_sample: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

/// The actual iterator over the channel data for a sample, yielded by [`ChannelSamples`].
pub struct ChannelSamplesIter<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_sample: usize,
    pub(self) current_channel: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

impl<'slice, 'sample> Iterator for SamplesIter<'slice, 'sample> {
    type Item = ChannelSamples<'slice, 'sample>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // This can iterate over both the entire buffer or over a smaller sample slice of it
        if self.current_sample < self.samples_end {
            let channels = ChannelSamples {
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

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.samples_end - self.current_sample;

        (remaining, Some(remaining))
    }
}

impl<'slice, 'sample> IntoIterator for ChannelSamples<'slice, 'sample> {
    type Item = &'sample mut f32;
    type IntoIter = ChannelSamplesIter<'slice, 'sample>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        ChannelSamplesIter {
            buffers: self.buffers,
            current_sample: self.current_sample,
            current_channel: 0,
            _marker: self._marker,
        }
    }
}

impl<'slice, 'sample> Iterator for ChannelSamplesIter<'slice, 'sample> {
    type Item = &'sample mut f32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current_channel < unsafe { (&(*self.buffers)).len() } {
            // SAFETY: These bounds have already been checked
            // SAFETY: It is also not possible to have multiple mutable references to the same
            // sample at the same time
            let sample = unsafe {
                (&mut (*self.buffers))
                    .get_unchecked_mut(self.current_channel)
                    .get_unchecked_mut(self.current_sample)
            };

            self.current_channel += 1;

            Some(sample)
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = unsafe { (&(*self.buffers)).len() } - self.current_channel;

        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for SamplesIter<'_, '_> {}
impl ExactSizeIterator for ChannelSamplesIter<'_, '_> {}

impl<'slice, 'sample> ChannelSamples<'slice, 'sample> {
    /// Get the number of channels.
    #[allow(clippy::len_without_is_empty)]
    #[inline]
    pub fn len(&self) -> usize {
        unsafe { (&(*self.buffers)).len() }
    }

    /// A resetting iterator. This lets you iterate over the same channels multiple times. Otherwise
    /// you don't need to use this function as [`ChannelSamples`] already implements
    /// [`IntoIterator`].
    #[inline]
    pub fn iter_mut(&mut self) -> ChannelSamplesIter<'slice, 'sample> {
        ChannelSamplesIter {
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
                (&mut (*self.buffers))
                    .get_mut(channel_index)?
                    .get_unchecked_mut(self.current_sample),
            )
        }
    }

    /// The same as [`get_mut()`][Self::get_mut()], but without any bounds checking.
    ///
    /// # Safety
    ///
    /// `channel_index` must be in the range `0..Self::len()`.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, channel_index: usize) -> &mut f32 {
        (&mut (*self.buffers))
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
                *(&(*self.buffers))
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
            *value = *(&(*self.buffers))
                .get_unchecked(channel_idx)
                .get_unchecked(self.current_sample);
        }

        Simd::from_array(values)
    }

    /// Write data from a SIMD vector to this sample's channel data. This takes the padding added by
    /// [`to_simd()`][Self::to_simd()] into account.
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
                (&mut (*self.buffers))
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
            *(&mut (*self.buffers))
                .get_unchecked_mut(channel_idx)
                .get_unchecked_mut(self.current_sample) = value;
        }
    }
}
