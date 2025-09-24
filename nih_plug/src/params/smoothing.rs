//! Utilities to handle smoothing parameter changes over time.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

// Re-exported here because it's sued in `SmoothingStyle`.
pub use atomic_float::AtomicF32;

/// Controls if and how parameters gets smoothed.
#[derive(Debug, Clone)]
pub enum SmoothingStyle {
    /// Wraps another smoothing style to create a multi-rate oversampling-aware smoother for a
    /// parameter that's used in an oversampled part of the plugin. The `Arc<AtomicF32>` indicates
    /// the oversampling amount, where `1.0` means no oversampling. This value can change at
    /// runtime, and it effectively scales the sample rate when computing new smoothing coefficients
    /// when the parameter's value changes.
    OversamplingAware(Arc<AtomicF32>, &'static SmoothingStyle),

    /// No smoothing is applied. The parameter's `value` field contains the latest sample value
    /// available for the parameters.
    None,
    /// Smooth parameter changes so the current value approaches the target value at a constant
    /// rate. The target value will be reached in exactly this many milliseconds.
    Linear(f32),
    /// Smooth parameter changes such that the rate matches the curve of a logarithmic function,
    /// starting out slow and then constantly increasing the slope until the value is reached. The
    /// target value will be reached in exactly this many milliseconds. This is useful for smoothing
    /// things like frequencies and decibel gain value. **The caveat is that the value may never
    /// reach 0**, or you will end up multiplying and dividing things by zero. Make sure your value
    /// ranges don't include 0.
    Logarithmic(f32),
    /// Smooth parameter changes such that the rate matches the curve of an exponential function,
    /// starting out fast and then tapering off until the end. This is a single-pole IIR filter
    /// under the hood, while the other smoothing options are FIR filters. This means that the exact
    /// value would never be reached. Instead, this reaches 99.99% of the value target value in the
    /// specified number of milliseconds, and it then snaps to the target value in the last step.
    /// This results in a smoother transition, with the caveat being that there will be a tiny jump
    /// at the end. Unlike the `Logarithmic` option, this does support crossing the zero value.
    Exponential(f32),
}

/// A smoother, providing a smoothed value for each sample.
//
// TODO: We need to use atomics here so we can share the params object with the GUI. Is there a
//       better alternative to allow the process function to mutate these smoothers?
#[derive(Debug)]
pub struct Smoother<T: Smoothable> {
    /// The kind of snoothing that needs to be applied, if any.
    pub style: SmoothingStyle,
    /// The number of steps of smoothing left to take.
    ///
    // This is a signed integer because we can skip multiple steps, which would otherwise make it
    // possible to get an underflow here.
    steps_left: AtomicI32,
    /// The amount we should adjust the current value each sample to be able to reach the target in
    /// the specified tiem frame. This is also a floating point number to keep the smoothing
    /// uniform.
    ///
    /// In the case of the `Exponential` smoothing style this is the coefficient `x` that the
    /// previous sample is multiplied by.
    step_size: AtomicF32,
    /// The value for the current sample. Always stored as floating point for obvious reasons.
    current: AtomicF32,
    /// The value we're smoothing towards
    target: T::Atomic,
}

/// An iterator that continuously produces smoothed values. Can be used as an alternative to the
/// block-based smoothing API. Since the iterator itself is infinite, you can use
/// [`Smoother::is_smoothing()`] and [`Smoother::steps_left()`] to get information on the current
/// smoothing status.
pub struct SmootherIter<'a, T: Smoothable> {
    smoother: &'a Smoother<T>,
}

impl SmoothingStyle {
    /// Compute the number of steps to reach the target value based on the sample rate and this
    /// smoothing style's duration.
    #[inline]
    pub fn num_steps(&self, sample_rate: f32) -> u32 {
        nih_debug_assert!(sample_rate > 0.0);

        match self {
            Self::OversamplingAware(oversampling_times, style) => {
                style.num_steps(sample_rate * oversampling_times.load(Ordering::Relaxed))
            }

            Self::None => 1,
            Self::Linear(time) | Self::Logarithmic(time) | Self::Exponential(time) => {
                nih_debug_assert!(*time >= 0.0);
                (sample_rate * time / 1000.0).round() as u32
            }
        }
    }

    /// Compute the step size for this smoother. `num_steps` can be obtained using
    /// [`SmoothingStyle::num_steps()`]. Check the source code of the [`SmoothingStyle::next()`] and
    /// [`SmoothingStyle::next_step()`] functions for details on how these values should be used.
    #[inline]
    pub fn step_size(&self, start: f32, target: f32, num_steps: u32) -> f32 {
        nih_debug_assert!(num_steps >= 1);

        match self {
            Self::OversamplingAware(_, style) => style.step_size(start, target, num_steps),

            Self::None => 0.0,
            Self::Linear(_) => (target - start) / (num_steps as f32),
            Self::Logarithmic(_) => {
                // We need to solve `start * (step_size ^ num_steps) = target` for `step_size`
                nih_debug_assert_ne!(start, 0.0);
                ((target / start) as f64).powf((num_steps as f64).recip()) as f32
            }
            // In this case the step size value is the coefficient the current value will be
            // multiplied by, while the target value is multiplied by one minus the coefficient. This
            // reaches 99.99% of the target value after `num_steps`. The smoother will snap to the
            // target value after that point.
            Self::Exponential(_) => 0.0001f64.powf((num_steps as f64).recip()) as f32,
        }
    }

    /// Compute the next value from `current` leading up to `target` using the `step_size` computed
    /// using [`SmoothingStyle::step_size()`]. Depending on the smoothing style this function may
    /// never completely reach `target`, so you will need to snap to `target` yourself after
    /// computing the target number of steps.
    ///
    /// See the docstring on the [`SmoothingStyle::next_step()`] function for the formulas used.
    #[inline]
    pub fn next(&self, current: f32, target: f32, step_size: f32) -> f32 {
        match self {
            Self::OversamplingAware(_, style) => style.next(current, target, step_size),

            Self::None => target,
            Self::Linear(_) => current + step_size,
            Self::Logarithmic(_) => current * step_size,
            Self::Exponential(_) => (current * step_size) + (target * (1.0 - step_size)),
        }
    }

    /// The same as [`next()`][Self::next()], but with the option to take more than one step at a
    /// time. Calling `next_step()` with step count `n` gives the same result as applying `next()`
    /// `n` times to a value, but is more efficient to compute. `next_step()` with 1 step is
    /// equivalent to `step()`.
    ///
    /// See the docstring on the [`SmoothingStyle::next_step()`] function for the formulas used.
    #[inline]
    pub fn next_step(&self, current: f32, target: f32, step_size: f32, steps: u32) -> f32 {
        nih_debug_assert!(steps >= 1);

        match self {
            Self::OversamplingAware(_, style) => style.next_step(current, target, step_size, steps),

            Self::None => target,
            Self::Linear(_) => current + (step_size * steps as f32),
            Self::Logarithmic(_) => current * (step_size.powi(steps as i32)),
            Self::Exponential(_) => {
                // This is the same as calculating `current = (current * step_size) +
                // (target * (1 - step_size))` in a loop since the target value won't change
                let coefficient = step_size.powi(steps as i32);
                (current * coefficient) + (target * (1.0 - coefficient))
            }
        }
    }
}

/// A type that can be smoothed. This exists just to avoid duplicate explicit implementations for
/// the smoothers.
pub trait Smoothable: Default + Clone + Copy {
    /// The atomic representation of `Self`.
    type Atomic: Default;

    fn to_f32(self) -> f32;
    fn from_f32(value: f32) -> Self;

    fn atomic_new(value: Self) -> Self::Atomic;
    /// A relaxed atomic load.
    fn atomic_load(this: &Self::Atomic) -> Self;
    /// A relaxed atomic store.
    fn atomic_store(this: &Self::Atomic, value: Self);
}

impl<T: Smoothable> Default for Smoother<T> {
    fn default() -> Self {
        Self {
            style: SmoothingStyle::None,
            steps_left: AtomicI32::new(0),
            step_size: Default::default(),
            current: AtomicF32::new(0.0),
            target: Default::default(),
        }
    }
}

impl<T: Smoothable> Iterator for SmootherIter<'_, T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        Some(self.smoother.next())
    }
}

impl<T: Smoothable> Clone for Smoother<T> {
    fn clone(&self) -> Self {
        // We can't derive clone because of the atomics, but these atomics are only here to allow
        // Send+Sync interior mutability
        Self {
            style: self.style.clone(),
            steps_left: AtomicI32::new(self.steps_left.load(Ordering::Relaxed)),
            step_size: AtomicF32::new(self.step_size.load(Ordering::Relaxed)),
            current: AtomicF32::new(self.current.load(Ordering::Relaxed)),
            target: T::atomic_new(T::atomic_load(&self.target)),
        }
    }
}

impl<T: Smoothable> Smoother<T> {
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

    /// The number of steps left until calling [`next()`][Self::next()] will stop yielding new
    /// values.
    #[inline]
    pub fn steps_left(&self) -> i32 {
        self.steps_left.load(Ordering::Relaxed)
    }

    /// Whether calling [`next()`][Self::next()] will yield a new value or an old value. Useful if
    /// you need to recompute something whenever this parameter changes.
    #[inline]
    pub fn is_smoothing(&self) -> bool {
        self.steps_left() > 0
    }

    /// Produce an iterator that yields smoothed values. These are not iterators already for the
    /// sole reason that this will always yield a value, and needing to unwrap all of those options
    /// is not going to be very fun.
    #[inline]
    pub fn iter(&self) -> SmootherIter<'_, T> {
        SmootherIter { smoother: self }
    }

    /// Reset the smoother the specified value.
    pub fn reset(&self, value: T) {
        T::atomic_store(&self.target, value);
        self.current.store(value.to_f32(), Ordering::Relaxed);
        self.steps_left.store(0, Ordering::Relaxed);
    }

    /// Set the target value.
    pub fn set_target(&self, sample_rate: f32, target: T) {
        T::atomic_store(&self.target, target);

        let steps_left = self.style.num_steps(sample_rate) as i32;
        self.steps_left.store(steps_left, Ordering::Relaxed);

        let current = self.current.load(Ordering::Relaxed);
        let target_f32 = target.to_f32();
        self.step_size.store(
            if steps_left > 0 {
                self.style.step_size(current, target_f32, steps_left as u32)
            } else {
                0.0
            },
            Ordering::Relaxed,
        );
    }

    /// Get the next value from this smoother. The value will be equal to the previous value once
    /// the smoothing period is over. This should be called exactly once per sample.
    // Yes, Clippy, like I said, this was intentional
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&self) -> T {
        let target = T::atomic_load(&self.target);

        // NOTE: This used to be implemented in terms of `next_step()`, but this is more efficient
        //       for the common use case of single steps
        if self.steps_left.load(Ordering::Relaxed) > 0 {
            let current = self.current.load(Ordering::Relaxed);
            let target_f32 = target.to_f32();
            let step_size = self.step_size.load(Ordering::Relaxed);

            // The number of steps usually won't fit exactly, so make sure we don't end up with
            // quantization errors on overshoots or undershoots. We also need to account for the
            // possibility that we only have `n < steps` steps left. This is especially important
            // for the `Exponential` smoothing style, since that won't reach the target value
            // exactly.
            let old_steps_left = self.steps_left.fetch_sub(1, Ordering::Relaxed);
            let new = if old_steps_left == 1 {
                self.steps_left.store(0, Ordering::Relaxed);
                target_f32
            } else {
                self.style.next(current, target_f32, step_size)
            };
            self.current.store(new, Ordering::Relaxed);

            T::from_f32(new)
        } else {
            target
        }
    }

    /// [`next()`][Self::next()], but with the ability to skip forward in the smoother.
    /// [`next()`][Self::next()] is equivalent to calling this function with a `steps` value of 1.
    /// Calling this function with a `steps` value of `n` means will cause you to skip the next `n -
    /// 1` values and return the `n`th value.
    #[inline]
    pub fn next_step(&self, steps: u32) -> T {
        nih_debug_assert_ne!(steps, 0);

        let target = T::atomic_load(&self.target);

        if self.steps_left.load(Ordering::Relaxed) > 0 {
            let current = self.current.load(Ordering::Relaxed);
            let target_f32 = target.to_f32();
            let step_size = self.step_size.load(Ordering::Relaxed);

            // The number of steps usually won't fit exactly, so make sure we don't end up with
            // quantization errors on overshoots or undershoots. We also need to account for the
            // possibility that we only have `n < steps` steps left. This is especially important
            // for the `Exponential` smoothing style, since that won't reach the target value
            // exactly.
            let old_steps_left = self.steps_left.fetch_sub(steps as i32, Ordering::Relaxed);
            let new = if old_steps_left <= steps as i32 {
                self.steps_left.store(0, Ordering::Relaxed);
                target_f32
            } else {
                self.style.next_step(current, target_f32, step_size, steps)
            };
            self.current.store(new, Ordering::Relaxed);

            T::from_f32(new)
        } else {
            target
        }
    }

    /// Get previous value returned by this smoother. This may be useful to save some boilerplate
    /// when [`is_smoothing()`][Self::is_smoothing()] is used to determine whether an expensive
    /// calculation should take place, and [`next()`][Self::next()] gets called as part of that
    /// calculation.
    pub fn previous_value(&self) -> T {
        T::from_f32(self.current.load(Ordering::Relaxed))
    }

    /// Produce smoothed values for an entire block of audio. This is useful when iterating the same
    /// block of audio multiple times. For instance when summing voices for a synthesizer.
    /// `block_values[..block_len]` will be filled with the smoothed values. This is simply a
    /// convenient function for [`next_block_exact()`][Self::next_block_exact()] when iterating over
    /// variable length blocks with a known maximum size.
    ///
    /// # Panics
    ///
    /// Panics if `block_len > block_values.len()`.
    pub fn next_block(&self, block_values: &mut [T], block_len: usize) {
        self.next_block_exact(&mut block_values[..block_len])
    }

    /// The same as [`next_block()`][Self::next_block()], but filling the entire slice.
    pub fn next_block_exact(&self, block_values: &mut [T]) {
        let target = T::atomic_load(&self.target);

        // `self.next()` will yield the current value if the parameter is no longer smoothing, but
        // it's a bit of a waste to continuously call that if only the first couple or none of the
        // values in `block_values` would require smoothing and the rest don't. Instead, we'll just
        // smooth the values as necessary, and then reuse the target value for the rest of the
        // block.
        let steps_left = self.steps_left.load(Ordering::Relaxed) as usize;
        let num_smoothed_values = block_values.len().min(steps_left);
        if num_smoothed_values > 0 {
            let mut current = self.current.load(Ordering::Relaxed);
            let target_f32 = target.to_f32();
            let step_size = self.step_size.load(Ordering::Relaxed);

            if num_smoothed_values == steps_left {
                // This is the same as calling `next()` `num_smoothed_values` times, but with some
                // conditionals optimized out
                block_values[..num_smoothed_values - 1].fill_with(|| {
                    current = self.style.next(current, target_f32, step_size);
                    T::from_f32(current)
                });

                // In `next()` the last step snaps the value to the target value, so we'll do the
                // same thing here
                current = target_f32.to_f32();
                block_values[num_smoothed_values - 1] = target;
            } else {
                block_values[..num_smoothed_values].fill_with(|| {
                    current = self.style.next(current, target_f32, step_size);
                    T::from_f32(current)
                });
            }

            block_values[num_smoothed_values..].fill(target);

            self.current.store(current, Ordering::Relaxed);
            self.steps_left
                .fetch_sub(num_smoothed_values as i32, Ordering::Relaxed);
        } else {
            block_values.fill(target);
        }
    }

    /// The same as [`next_block()`][Self::next_block()], but with a function applied to each
    /// produced value. The mapping function takes an index in the block and a floating point
    /// representation of the smoother's current value. This allows the modulation to be consistent
    /// during smoothing. Additionally, the mapping function is always called even if the smoothing
    /// is finished.
    pub fn next_block_mapped(
        &self,
        block_values: &mut [T],
        block_len: usize,
        f: impl FnMut(usize, f32) -> T,
    ) {
        self.next_block_exact_mapped(&mut block_values[..block_len], f)
    }

    /// The same as [`next_block_exact()`][Self::next_block()], but with a function applied to each
    /// produced value. Useful when applying modulation to a smoothed parameter.
    pub fn next_block_exact_mapped(
        &self,
        block_values: &mut [T],
        mut f: impl FnMut(usize, f32) -> T,
    ) {
        // This works exactly the same as `next_block_exact()`, except for the addition of the
        // mapping function
        let target_f32 = T::atomic_load(&self.target).to_f32();

        let steps_left = self.steps_left.load(Ordering::Relaxed) as usize;
        let num_smoothed_values = block_values.len().min(steps_left);
        if num_smoothed_values > 0 {
            let mut current = self.current.load(Ordering::Relaxed);
            let step_size = self.step_size.load(Ordering::Relaxed);

            // See `next_block_exact()` for more details
            if num_smoothed_values == steps_left {
                for (idx, value) in block_values
                    .iter_mut()
                    .enumerate()
                    .take(num_smoothed_values - 1)
                {
                    current = self.style.next(current, target_f32, step_size);
                    *value = f(idx, current);
                }

                current = target_f32.to_f32();
                block_values[num_smoothed_values - 1] = f(num_smoothed_values - 1, target_f32);
            } else {
                for (idx, value) in block_values
                    .iter_mut()
                    .enumerate()
                    .take(num_smoothed_values)
                {
                    current = self.style.next(current, target_f32, step_size);
                    *value = f(idx, current);
                }
            }

            for (idx, value) in block_values
                .iter_mut()
                .enumerate()
                .skip(num_smoothed_values)
            {
                *value = f(idx, target_f32);
            }

            self.current.store(current, Ordering::Relaxed);
            self.steps_left
                .fetch_sub(num_smoothed_values as i32, Ordering::Relaxed);
        } else {
            for (idx, value) in block_values.iter_mut().enumerate() {
                *value = f(idx, target_f32);
            }
        }
    }
}

impl Smoothable for f32 {
    type Atomic = AtomicF32;

    #[inline]
    fn to_f32(self) -> f32 {
        self
    }

    #[inline]
    fn from_f32(value: f32) -> Self {
        value
    }

    #[inline]
    fn atomic_new(value: Self) -> Self::Atomic {
        AtomicF32::new(value)
    }

    #[inline]
    fn atomic_load(this: &Self::Atomic) -> Self {
        this.load(Ordering::Relaxed)
    }

    #[inline]
    fn atomic_store(this: &Self::Atomic, value: Self) {
        this.store(value, Ordering::Relaxed)
    }
}

impl Smoothable for i32 {
    type Atomic = AtomicI32;

    #[inline]
    fn to_f32(self) -> f32 {
        self as f32
    }

    #[inline]
    fn from_f32(value: f32) -> Self {
        value.round() as i32
    }

    #[inline]
    fn atomic_new(value: Self) -> Self::Atomic {
        AtomicI32::new(value)
    }

    #[inline]
    fn atomic_load(this: &Self::Atomic) -> Self {
        this.load(Ordering::Relaxed)
    }

    #[inline]
    fn atomic_store(this: &Self::Atomic, value: Self) {
        this.store(value, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applying `next()` `n` times should be the same as `next_step()` for `n` steps.
    #[test]
    fn linear_f32_next_equivalence() {
        let style = SmoothingStyle::Linear(100.0);

        let mut current = 0.4;
        let target = 0.8;
        let steps = 15;
        let step_size = style.step_size(current, target, steps);

        let expected_result = style.next_step(current, target, step_size, steps);
        for _ in 0..steps {
            current = style.next(current, target, step_size);
        }

        approx::assert_relative_eq!(current, expected_result, epsilon = 1e-5);
    }

    #[test]
    fn logarithmic_f32_next_equivalence() {
        let style = SmoothingStyle::Logarithmic(100.0);

        let mut current = 0.4;
        let target = 0.8;
        let steps = 15;
        let step_size = style.step_size(current, target, steps);

        let expected_result = style.next_step(current, target, step_size, steps);
        for _ in 0..steps {
            current = style.next(current, target, step_size);
        }

        approx::assert_relative_eq!(current, expected_result, epsilon = 1e-5);
    }

    #[test]
    fn exponential_f32_next_equivalence() {
        let style = SmoothingStyle::Exponential(100.0);

        let mut current = 0.4;
        let target = 0.8;
        let steps = 15;
        let step_size = style.step_size(current, target, steps);

        let expected_result = style.next_step(current, target, step_size, steps);
        for _ in 0..steps {
            current = style.next(current, target, step_size);
        }

        approx::assert_relative_eq!(current, expected_result, epsilon = 1e-5);
    }

    #[test]
    fn linear_f32_smoothing() {
        let smoother: Smoother<f32> = Smoother::new(SmoothingStyle::Linear(100.0));
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
        let smoother: Smoother<i32> = Smoother::new(SmoothingStyle::Linear(100.0));
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
        let smoother: Smoother<f32> = Smoother::new(SmoothingStyle::Logarithmic(100.0));
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
        let smoother: Smoother<i32> = Smoother::new(SmoothingStyle::Logarithmic(100.0));
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

    /// Same as [`linear_f32_smoothing`], but skipping steps instead.
    #[test]
    fn skipping_linear_f32_smoothing() {
        let smoother: Smoother<f32> = Smoother::new(SmoothingStyle::Linear(100.0));
        smoother.reset(10.0);
        assert_eq!(smoother.next(), 10.0);

        smoother.set_target(100.0, 20.0);
        smoother.next_step(8);
        assert_ne!(smoother.next(), 20.0);
        assert_eq!(smoother.next(), 20.0);
    }

    /// Same as [`linear_i32_smoothing`], but skipping steps instead.
    #[test]
    fn skipping_linear_i32_smoothing() {
        let smoother: Smoother<i32> = Smoother::new(SmoothingStyle::Linear(100.0));
        smoother.reset(10);
        assert_eq!(smoother.next(), 10);

        smoother.set_target(100.0, 20);
        smoother.next_step(8);
        assert_ne!(smoother.next(), 20);
        assert_eq!(smoother.next(), 20);
    }

    /// Same as [`logarithmic_f32_smoothing`], but skipping steps instead.
    #[test]
    fn skipping_logarithmic_f32_smoothing() {
        let smoother: Smoother<f32> = Smoother::new(SmoothingStyle::Logarithmic(100.0));
        smoother.reset(10.0);
        assert_eq!(smoother.next(), 10.0);

        smoother.set_target(100.0, 20.0);
        smoother.next_step(8);
        assert_ne!(smoother.next(), 20.0);
        assert_eq!(smoother.next(), 20.0);
    }

    /// Same as [`logarithmic_i32_smoothing`], but skipping steps instead.
    #[test]
    fn skipping_logarithmic_i32_smoothing() {
        let smoother: Smoother<i32> = Smoother::new(SmoothingStyle::Logarithmic(100.0));
        smoother.reset(10);
        assert_eq!(smoother.next(), 10);

        smoother.set_target(100.0, 20);
        smoother.next_step(8);
        assert_ne!(smoother.next(), 20);
        assert_eq!(smoother.next(), 20);
    }

    // TODO: Tests for the exponential smoothing
}
