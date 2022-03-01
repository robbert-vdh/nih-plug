use atomic_float::AtomicF32;
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use std::sync::atomic::{AtomicI32, Ordering};

use crate::buffer::Block;

/// Controls if and how parameters gets smoothed.
pub enum SmoothingStyle {
    /// No smoothing is applied. The parameter's `value` field contains the latest sample value
    /// available for the parameters.
    None,
    /// Smooth parameter changes so the current value approaches the target value at a constant
    /// rate.
    Linear(f32),
    /// Smooth parameter changes such that the rate matches the curve of a logarithmic function.
    /// This is useful for smoothing things like frequencies and decibel gain value. **The caveat is
    /// that the value may never reach 0**, or you will end up multiplying and dividing things by
    /// zero. Make sure your value ranges don't include 0.
    Logarithmic(f32),
    // TODO: Sample-accurate modes
}

/// A smoother, providing a smoothed value for each sample.
//
// TODO: We need to use atomics here so we can share the params object with the GUI. Is there a
//       better alternative to allow the process function to mutate these smoothers?
pub struct Smoother<T> {
    /// The kind of snoothing that needs to be applied, if any.
    style: SmoothingStyle,
    /// The number of steps of smoothing left to take.
    ///
    // This is a signed integer because we can skip multiple steps, which would otherwise make it
    // possible to get an underflow here.
    steps_left: AtomicI32,
    /// The amount we should adjust the current value each sample to be able to reach the target in
    /// the specified tiem frame. This is also a floating point number to keep the smoothing
    /// uniform.
    step_size: f32,
    /// The value for the current sample. Always stored as floating point for obvious reasons.
    current: AtomicF32,
    /// The value we're smoothing towards
    target: T,

    /// A dense buffer containing smoothed values for an entire block of audio. Useful when using
    /// [crate::Buffer::iter_blocks()] to process small blocks of audio multiple times.
    block_values: AtomicRefCell<Vec<T>>,
}

impl<T: Default> Default for Smoother<T> {
    fn default() -> Self {
        Self {
            style: SmoothingStyle::None,
            steps_left: AtomicI32::new(0),
            step_size: Default::default(),
            current: AtomicF32::new(0.0),
            target: Default::default(),

            block_values: AtomicRefCell::new(Vec::new()),
        }
    }
}

impl<T: Default> Smoother<T> {
    /// Use the specified style for the smoothing.
    pub fn new(style: SmoothingStyle) -> Self {
        Self {
            style,
            ..Default::default()
        }
    }

    /// Convenience function for not applying any smoothing at all. Same as `Smoother::default`.
    pub fn none() -> Self {
        Default::default()
    }

    /// Whether calling [Self::next()] will yield a new value or an old value. Useful if you need to
    /// recompute something wheenver this parameter changes.
    pub fn is_smoothing(&self) -> bool {
        self.steps_left.load(Ordering::Relaxed) > 0
    }

    /// Allocate memory to store smoothed values for an entire block of audio. Call this in
    /// [crate::Plugin::initialize()] with the same max block size you are going to pass to
    /// [crate::Buffer::iter_blocks()].
    pub fn initialize_block_smoother(&mut self, max_block_size: usize) {
        self.block_values
            .borrow_mut()
            .resize_with(max_block_size, || T::default());
    }
}

// These are not iterators for the sole reason that this will always yield a value, and needing to
// unwrap all of those options is not going to be very fun.
impl Smoother<f32> {
    /// Reset the smoother the specified value.
    pub fn reset(&mut self, value: f32) {
        self.target = value;
        self.current.store(value, Ordering::Relaxed);
        self.steps_left.store(0, Ordering::Relaxed);
    }

    /// Set the target value.
    pub fn set_target(&mut self, sample_rate: f32, target: f32) {
        self.target = target;

        let steps_left = match self.style {
            SmoothingStyle::None => 1,
            SmoothingStyle::Linear(time) | SmoothingStyle::Logarithmic(time) => {
                (sample_rate * time / 1000.0).round() as i32
            }
        };
        self.steps_left.store(steps_left, Ordering::Relaxed);

        let current = self.current.load(Ordering::Relaxed);
        self.step_size = match self.style {
            SmoothingStyle::None => 0.0,
            SmoothingStyle::Linear(_) => (self.target - current) / steps_left as f32,
            SmoothingStyle::Logarithmic(_) => {
                // We need to solve `current * (step_size ^ steps_left) = target` for
                // `step_size`
                nih_debug_assert_ne!(current, 0.0);
                (self.target / current).powf((steps_left as f32).recip())
            }
        };
    }

    /// Get the next value from this smoother. The value will be equal to the previous value once
    /// the smoothing period is over. This should be called exactly once per sample.
    // Yes, Clippy, like I said, this was intentional
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&self) -> f32 {
        self.next_step(1)
    }

    /// Produce smoothed values for an entire block of audio. Used in conjunction with
    /// [crate::Buffer::iter_blocks()]. Make sure to call
    /// [crate::Plugin::initialize_block_smoothers()] with the same maximum buffer block size as the
    /// one passed to `iter_blocks()` in your [crate::Plugin::initialize()] function first to
    /// allocate memory for the block smoothing.
    ///
    /// Returns a `None` value if the block length exceed's the allocated capacity.
    #[inline]
    pub fn next_block(&self, block: &Block) -> Option<AtomicRefMut<[f32]>> {
        let mut block_values = self.block_values.borrow_mut();
        if block_values.len() < block.len() {
            return None;
        }

        // TODO: As a small optimization we could split this up into two loops for the smoothed and
        //       unsmoothed parts. Another worthwhile optimization would be to remember if the
        //       buffer is already filled with the target value and [Self::is_smoothing()] is false.
        //       In that case we wouldn't need to do anything ehre.
        (&mut block_values[..block.len()]).fill_with(|| self.next());

        Some(AtomicRefMut::map(block_values, |values| {
            &mut values[..block.len()]
        }))
    }

    /// [Self::next()], but with the ability to skip forward in the smoother. [Self::next()] is
    /// equivalent to calling this function with a `steps` value of 1. Calling this function with a
    /// `steps` value of `n` means will cause you to skip the next `n - 1` values and return the
    /// `n`th value.
    #[inline]
    pub fn next_step(&self, steps: u32) -> f32 {
        nih_debug_assert_ne!(steps, 0);

        if self.steps_left.load(Ordering::Relaxed) > 0 {
            let current = self.current.load(Ordering::Relaxed);

            // The number of steps usually won't fit exactly, so make sure we don't end up with
            // quantization errors on overshoots or undershoots. We also need to account for the
            // possibility that we only have `n < steps` steps left.
            let old_steps_left = self.steps_left.fetch_sub(steps as i32, Ordering::Relaxed);
            let new = if old_steps_left <= steps as i32 {
                self.steps_left.store(0, Ordering::Relaxed);
                self.target
            } else {
                match &self.style {
                    SmoothingStyle::None => self.target,
                    SmoothingStyle::Linear(_) => current + (self.step_size * steps as f32),
                    SmoothingStyle::Logarithmic(_) => current * (self.step_size.powi(steps as i32)),
                }
            };
            self.current.store(new, Ordering::Relaxed);

            new
        } else {
            self.target
        }
    }
}

impl Smoother<i32> {
    /// Reset the smoother the specified value.
    pub fn reset(&mut self, value: i32) {
        self.target = value;
        self.current.store(value as f32, Ordering::Relaxed);
        self.steps_left.store(0, Ordering::Relaxed);
    }

    pub fn set_target(&mut self, sample_rate: f32, target: i32) {
        self.target = target;

        let steps_left = match self.style {
            SmoothingStyle::None => 1,
            SmoothingStyle::Linear(time) | SmoothingStyle::Logarithmic(time) => {
                (sample_rate * time / 1000.0).round() as i32
            }
        };
        self.steps_left.store(steps_left, Ordering::Relaxed);

        let current = self.current.load(Ordering::Relaxed);
        self.step_size = match self.style {
            SmoothingStyle::None => 0.0,
            SmoothingStyle::Linear(_) => (self.target as f32 - current) / steps_left as f32,
            SmoothingStyle::Logarithmic(_) => {
                nih_debug_assert_ne!(current, 0.0);
                (self.target as f32 / current).powf((steps_left as f32).recip())
            }
        };
    }

    /// Get the next value from this smoother. The value will be equal to the previous value once
    // the smoothing period is over. This should be called exactly once per sample.
    // Yes, Clippy, like I said, this was intentional
    #[allow(clippy::should_implement_trait)]
    pub fn next(&self) -> i32 {
        self.next_step(1)
    }

    /// Produce smoothed values for an entire block of audio. Used in conjunction with
    /// [crate::Buffer::iter_blocks()]. Make sure to call
    /// [crate::Plugin::initialize_block_smoothers()] with the same maximum buffer block size as the
    /// one passed to `iter_blocks()` in your [crate::Plugin::initialize()] function first to
    /// allocate memory for the block smoothing.
    ///
    /// Returns a `None` value if the block length exceed's the allocated capacity.
    #[inline]
    pub fn next_block(&self, block: &Block) -> Option<AtomicRefMut<[i32]>> {
        let mut block_values = self.block_values.borrow_mut();
        if block_values.len() < block.len() {
            return None;
        }

        (&mut block_values[..block.len()]).fill_with(|| self.next());

        Some(AtomicRefMut::map(block_values, |values| {
            &mut values[..block.len()]
        }))
    }

    /// [Self::next()], but with the ability to skip forward in the smoother. [Self::next()] is
    /// equivalent to calling this function with a `steps` value of 1. Calling this function with a
    /// `steps` value of `n` means will cause you to skip the next `n - 1` values and return the
    /// `n`th value.
    pub fn next_step(&self, steps: u32) -> i32 {
        nih_debug_assert_ne!(steps, 0);

        if self.steps_left.load(Ordering::Relaxed) > 0 {
            let current = self.current.load(Ordering::Relaxed);

            // The number of steps usually won't fit exactly, so make sure we don't end up with
            // quantization errors on overshoots or undershoots. We also need to account for the
            // possibility that we only have `n < steps` steps left.
            let old_steps_left = self.steps_left.fetch_sub(steps as i32, Ordering::Relaxed);
            let new = if old_steps_left <= steps as i32 {
                self.steps_left.store(0, Ordering::Relaxed);
                self.target as f32
            } else {
                match &self.style {
                    SmoothingStyle::None => self.target as f32,
                    SmoothingStyle::Linear(_) => current + (self.step_size * steps as f32),
                    SmoothingStyle::Logarithmic(_) => current * self.step_size.powi(steps as i32),
                }
            };
            self.current.store(new, Ordering::Relaxed);

            new.round() as i32
        } else {
            self.target
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_f32_smoothing() {
        let mut smoother: Smoother<f32> = Smoother::new(SmoothingStyle::Linear(100.0));
        smoother.reset(10.0);
        assert_eq!(smoother.next(), 10.0);

        // Instead of testing the actual values, we'll make sure that we reach the target values at
        // the expected time.
        smoother.set_target(100.0, 20.0);
        for _ in 0..(10 - 2) {
            smoother.next();
        }
        assert_ne!(smoother.next(), 20.0);
        assert_eq!(smoother.next(), 20.0);
    }

    #[test]
    fn linear_i32_smoothing() {
        let mut smoother: Smoother<i32> = Smoother::new(SmoothingStyle::Linear(100.0));
        smoother.reset(10);
        assert_eq!(smoother.next(), 10);

        // Integers are rounded, but with these values we can still test this
        smoother.set_target(100.0, 20);
        for _ in 0..(10 - 2) {
            smoother.next();
        }
        assert_ne!(smoother.next(), 20);
        assert_eq!(smoother.next(), 20);
    }

    #[test]
    fn logarithmic_f32_smoothing() {
        let mut smoother: Smoother<f32> = Smoother::new(SmoothingStyle::Logarithmic(100.0));
        smoother.reset(10.0);
        assert_eq!(smoother.next(), 10.0);

        // Instead of testing the actual values, we'll make sure that we reach the target values at
        // the expected time.
        smoother.set_target(100.0, 20.0);
        for _ in 0..(10 - 2) {
            smoother.next();
        }
        assert_ne!(smoother.next(), 20.0);
        assert_eq!(smoother.next(), 20.0);
    }

    #[test]
    fn logarithmic_i32_smoothing() {
        let mut smoother: Smoother<i32> = Smoother::new(SmoothingStyle::Logarithmic(100.0));
        smoother.reset(10);
        assert_eq!(smoother.next(), 10);

        // Integers are rounded, but with these values we can still test this
        smoother.set_target(100.0, 20);
        for _ in 0..(10 - 2) {
            smoother.next();
        }
        assert_ne!(smoother.next(), 20);
        assert_eq!(smoother.next(), 20);
    }

    /// Same as [linear_f32_smoothing], but skipping steps instead.
    #[test]
    fn skipping_linear_f32_smoothing() {
        let mut smoother: Smoother<f32> = Smoother::new(SmoothingStyle::Linear(100.0));
        smoother.reset(10.0);
        assert_eq!(smoother.next(), 10.0);

        smoother.set_target(100.0, 20.0);
        smoother.next_step(8);
        assert_ne!(smoother.next(), 20.0);
        assert_eq!(smoother.next(), 20.0);
    }

    /// Same as [linear_i32_smoothing], but skipping steps instead.
    #[test]
    fn skipping_linear_i32_smoothing() {
        let mut smoother: Smoother<i32> = Smoother::new(SmoothingStyle::Linear(100.0));
        smoother.reset(10);
        assert_eq!(smoother.next(), 10);

        smoother.set_target(100.0, 20);
        smoother.next_step(8);
        assert_ne!(smoother.next(), 20);
        assert_eq!(smoother.next(), 20);
    }

    /// Same as [logarithmic_f32_smoothing], but skipping steps instead.
    #[test]
    fn skipping_logarithmic_f32_smoothing() {
        let mut smoother: Smoother<f32> = Smoother::new(SmoothingStyle::Logarithmic(100.0));
        smoother.reset(10.0);
        assert_eq!(smoother.next(), 10.0);

        smoother.set_target(100.0, 20.0);
        smoother.next_step(8);
        assert_ne!(smoother.next(), 20.0);
        assert_eq!(smoother.next(), 20.0);
    }

    /// Same as [logarithmic_i32_smoothing], but skipping steps instead.
    #[test]
    fn skipping_logarithmic_i32_smoothing() {
        let mut smoother: Smoother<i32> = Smoother::new(SmoothingStyle::Logarithmic(100.0));
        smoother.reset(10);
        assert_eq!(smoother.next(), 10);

        smoother.set_target(100.0, 20);
        smoother.next_step(8);
        assert_ne!(smoother.next(), 20);
        assert_eq!(smoother.next(), 20);
    }
}
