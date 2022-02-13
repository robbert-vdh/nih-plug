use std::marker::PhantomData;

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

impl<'a> Buffer<'a> {
    /// Returns true if this buffer does not contain any samples.
    pub fn is_empty(&self) -> bool {
        self.output_slices.is_empty() || self.output_slices[0].is_empty()
    }

    /// Obtain the raw audio buffers.
    pub fn as_raw(&mut self) -> &mut [&'a mut [f32]] {
        &mut self.output_slices
    }

    /// Iterate over the samples, returning a channel iterator for each sample.
    pub fn iter_mut(&mut self) -> Samples<'a, 'a> {
        Samples {
            buffers: self.output_slices.as_mut_slice(),
            current_sample: 0,
            _marker: PhantomData,
        }
    }

    /// Access the raw output slice vector. This neds to be resized to match the number of output
    /// channels during the plugin's initialization. Then during audio processing, these slices
    /// should be updated to point to the plugin's audio buffers.
    ///
    /// # Safety
    ///
    /// The stored slices must point to live data when this object is passed to the plugins' process
    /// function. The rest of this object also assumes all channel lengths are equal. Panics will
    /// likely occur if this is not the case.
    pub unsafe fn as_raw_vec(&mut self) -> &mut Vec<&'a mut [f32]> {
        &mut self.output_slices
    }
}

/// An iterator over all samples in the buffer, yielding iterators over each channel for every
/// sample. This iteration order offers good cache locality for per-sample access.
pub struct Samples<'buffer: 'sample, 'sample> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_sample: usize,
    pub(self) _marker: PhantomData<&'buffer mut [&'sample mut [f32]]>,
}

impl<'buffer, 'sample> Iterator for Samples<'buffer, 'sample> {
    type Item = Channels<'buffer, 'sample>;

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

impl ExactSizeIterator for Samples<'_, '_> {}

/// Can construct iterators over actual iterator over the channel data for a sample, yielded by
/// [Samples].
pub struct Channels<'buffer: 'sample, 'sample> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_sample: usize,
    pub(self) _marker: PhantomData<&'buffer mut [&'sample mut [f32]]>,
}

/// The actual iterator over the channel data for a sample, yielded by [Channels].
pub struct ChannelsIter<'buffer: 'sample, 'sample> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_sample: usize,
    pub(self) current_channel: usize,
    pub(self) _marker: PhantomData<&'buffer mut [&'sample mut [f32]]>,
}

impl<'buffer, 'sample> IntoIterator for Channels<'buffer, 'sample> {
    type Item = &'sample mut f32;
    type IntoIter = ChannelsIter<'buffer, 'sample>;

    fn into_iter(self) -> Self::IntoIter {
        ChannelsIter {
            buffers: self.buffers,
            current_sample: self.current_sample,
            current_channel: 0,
            _marker: self._marker,
        }
    }
}

impl<'buffer, 'sample> Iterator for ChannelsIter<'buffer, 'sample> {
    type Item = &'sample mut f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_channel < unsafe { (*self.buffers).len() } {
            // SAFETY: These bounds have already been checked
            let sample = unsafe {
                (*self.buffers)
                    .get_unchecked_mut(self.current_channel)
                    .get_unchecked_mut(self.current_sample)
            };
            // SAFETY: It is not possible to have multiple mutable references to the same sample at
            // the same time
            let sample: &'sample mut f32 = unsafe { &mut *(sample as *mut f32) };

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

impl ExactSizeIterator for ChannelsIter<'_, '_> {}

impl<'buffer, 'sample> Channels<'buffer, 'sample> {
    /// Get the number of channels.
    pub fn len(&self) -> usize {
        unsafe { (*self.buffers).len() }
    }

    /// A resetting iterator. This lets you iterate over the same channels multiple times. Otherwise
    /// you don't need to use this function as [Channels] already implements [Iterator].
    pub fn iter_mut(&mut self) -> ChannelsIter<'buffer, 'sample> {
        // SAFETY: No two [ChannelIters] can exist at a time
        let buffers: *mut [&'sample mut [f32]] = unsafe { std::mem::transmute(self.buffers) };

        ChannelsIter {
            buffers,
            current_sample: self.current_sample,
            current_channel: 0,
            _marker: PhantomData,
        }
    }

    /// Access a sample by index. Useful when you would otehrwise iterate over this 'Channels'
    /// iterator multiple times.
    #[inline]
    pub fn get_mut(&mut self, channel_index: usize) -> Option<&mut f32> {
        // SAFETY: The channel bound has already been checked
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
}
