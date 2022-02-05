// nih-plug: plugins, but rewritten in Rust
// Copyright (C) 2022 Robbert van der Helm
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
pub struct Smoother<T> {
    /// The kind of snoothing that needs to be applied, if any.
    style: SmoothingStyle,
    /// The number of steps of smoothing left to take.
    steps_left: u32,
    /// The amount we should adjust the current value each sample to be able to reach the target in
    /// the specified tiem frame. This is also a floating point number to keep the smoothing
    /// uniform.
    step_size: f32,
    /// The value for the current sample. Always stored as floating point for obvious reasons.
    current: f32,
    /// The value we're smoothing towards
    target: T,
}

impl<T: Default> Default for Smoother<T> {
    fn default() -> Self {
        Self {
            style: SmoothingStyle::None,
            steps_left: 0,
            step_size: Default::default(),
            current: 0.0,
            target: Default::default(),
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

    /// Whether calling [Self::next] will yield a new value or an old value. Useful if you need to
    /// recompute something wheenver this parameter changes.
    pub fn is_smoothing(&self) -> bool {
        self.steps_left > 0
    }
}

// These are not iterators for the sole reason that this will always yield a value, and needing to
// unwrap all of those options is not going to be very fun.
impl Smoother<f32> {
    /// Reset the smoother the specified value.
    pub fn reset(&mut self, value: f32) {
        self.target = value;
        self.current = value;
        self.steps_left = 0;
    }

    /// Set the target value.
    pub fn set_target(&mut self, sample_rate: f32, target: f32) {
        self.target = target;
        self.steps_left = match self.style {
            SmoothingStyle::None => 1,
            SmoothingStyle::Linear(time) | SmoothingStyle::Logarithmic(time) => {
                (sample_rate * time / 1000.0).round() as u32
            }
        };
        self.step_size = match self.style {
            SmoothingStyle::None => 0.0,
            SmoothingStyle::Linear(_) => (self.target - self.current) / self.steps_left as f32,
            SmoothingStyle::Logarithmic(_) => {
                // We need to solve `current * (step_size ^ steps_left) = target` for
                // `step_size`
                nih_debug_assert_ne!(self.current, 0.0);
                (self.target / self.current).powf((self.steps_left as f32).recip())
            }
        };
    }

    // Yes, Clippy, like I said, this was intentional
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        if self.steps_left > 1 {
            // The number of steps usually won't fit exactly, so make sure we don't do weird things
            // with overshoots or undershoots
            self.steps_left -= 1;
            if self.steps_left == 0 {
                self.current = self.target;
            } else {
                match &self.style {
                    SmoothingStyle::None => self.current = self.target,
                    SmoothingStyle::Linear(_) => self.current += self.step_size,
                    SmoothingStyle::Logarithmic(_) => self.current *= self.step_size,
                };
            }

            self.current
        } else {
            self.target
        }
    }
}

impl Smoother<i32> {
    /// Reset the smoother the specified value.
    pub fn reset(&mut self, value: i32) {
        self.target = value;
        self.current = value as f32;
        self.steps_left = 0;
    }

    pub fn set_target(&mut self, sample_rate: f32, target: i32) {
        self.target = target;
        self.steps_left = match self.style {
            SmoothingStyle::None => 1,
            SmoothingStyle::Linear(time) | SmoothingStyle::Logarithmic(time) => {
                (sample_rate * time / 1000.0).round() as u32
            }
        };
        self.step_size = match self.style {
            SmoothingStyle::None => 0.0,
            SmoothingStyle::Linear(_) => {
                (self.target as f32 - self.current) / self.steps_left as f32
            }
            SmoothingStyle::Logarithmic(_) => {
                nih_debug_assert_ne!(self.current, 0.0);
                (self.target as f32 / self.current).powf((self.steps_left as f32).recip())
            }
        };
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> i32 {
        if self.steps_left > 1 {
            self.steps_left -= 1;
            if self.steps_left == 0 {
                self.current = self.target as f32;
            } else {
                match &self.style {
                    SmoothingStyle::None => self.current = self.target as f32,
                    SmoothingStyle::Linear(_) => self.current += self.step_size,
                    SmoothingStyle::Logarithmic(_) => self.current *= self.step_size,
                };
            }

            self.current.round() as i32
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
            dbg!(smoother.next());
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
            dbg!(smoother.next());
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
            dbg!(smoother.next());
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
            dbg!(smoother.next());
        }
        assert_ne!(smoother.next(), 20);
        assert_eq!(smoother.next(), 20);
    }
}
