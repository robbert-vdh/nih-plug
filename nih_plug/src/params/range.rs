//! Different ranges for numeric parameters.

use crate::util;

/// A distribution for a floating point parameter's range. All range endpoints are inclusive.
#[derive(Debug, Clone, Copy)]
pub enum FloatRange {
    /// The values are uniformly distributed between `min` and `max`.
    Linear { min: f32, max: f32 },
    /// The range is skewed by a factor. Values above 1.0 will make the end of the range wider,
    /// while values between 0 and 1 will skew the range towards the start. Use
    /// [`FloatRange::skew_factor()`] for a more intuitively way to calculate the skew factor where
    /// positive values skew the range towards the end while negative values skew the range toward
    /// the start.
    Skewed { min: f32, max: f32, factor: f32 },
    /// The same as [`FloatRange::Skewed`], but with the skewing happening from a central point.
    /// This central point is rescaled to be at 50% of the parameter's range for convenience of use.
    /// Git blame this comment to find a version that doesn't do this.
    SymmetricalSkewed {
        min: f32,
        max: f32,
        factor: f32,
        center: f32,
    },
    /// A reversed range that goes from high to low instead of from low to high.
    Reversed(&'static FloatRange),
}

/// A distribution for an integer parameter's range. All range endpoints are inclusive. Only linear
/// ranges are supported for integers since hosts expect discrete parameters to have a fixed step
/// size.
#[derive(Debug, Clone, Copy)]
pub enum IntRange {
    /// The values are uniformly distributed between `min` and `max`.
    Linear { min: i32, max: i32 },
    /// A reversed range that goes from high to low instead of from low to high.
    Reversed(&'static IntRange),
}

impl FloatRange {
    /// Calculate a skew factor for [`FloatRange::Skewed`] and [`FloatRange::SymmetricalSkewed`].
    /// Positive values make the end of the range wider while negative make the start of the range
    /// wider.
    pub fn skew_factor(factor: f32) -> f32 {
        2.0f32.powf(factor)
    }

    /// Calculate a skew factor for [`FloatRange::Skewed`] that makes a linear gain parameter range
    /// appear as if it was linear when formatted as decibels.
    pub fn gain_skew_factor(min_db: f32, max_db: f32) -> f32 {
        nih_debug_assert!(min_db < max_db);

        let min_gain = util::db_to_gain(min_db);
        let max_gain = util::db_to_gain(max_db);
        let middle_db = (max_db + min_db) / 2.0;
        let middle_gain = util::db_to_gain(middle_db);

        // Check the Skewed equation in the normalized function below, we need to solve the factor
        // such that the a normalized value of 0.5 resolves to the middle of the range
        0.5f32.log((middle_gain - min_gain) / (max_gain - min_gain))
    }

    /// Normalize a plain, unnormalized value. Will be clamped to the bounds of the range if the
    /// normalized value exceeds `[0, 1]`.
    pub fn normalize(&self, plain: f32) -> f32 {
        match self {
            FloatRange::Linear { min, max } => (plain.clamp(*min, *max) - min) / (max - min),
            FloatRange::Skewed { min, max, factor } => {
                ((plain.clamp(*min, *max) - min) / (max - min)).powf(*factor)
            }
            FloatRange::SymmetricalSkewed {
                min,
                max,
                factor,
                center,
            } => {
                // There's probably a much faster equivalent way to write this. Also, I have no clue
                // how I managed to implement this correctly on the first try.
                let unscaled_proportion = (plain.clamp(*min, *max) - min) / (max - min);
                let center_proportion = (center - min) / (max - min);
                if unscaled_proportion > center_proportion {
                    // The part above the center gets normalized to a [0, 1] range, skewed, and then
                    // unnormalized and scaled back to the original [center_proportion, 1] range
                    let scaled_proportion = (unscaled_proportion - center_proportion)
                        * (1.0 - center_proportion).recip();
                    (scaled_proportion.powf(*factor) * 0.5) + 0.5
                } else {
                    // The part below the center gets scaled, inverted (so the range is [0, 1] where
                    // 0 corresponds to the center proportion and 1 corresponds to the original
                    // normalized 0 value), skewed, inverted back again, and then scaled back to the
                    // original range
                    let inverted_scaled_proportion =
                        (center_proportion - unscaled_proportion) * (center_proportion).recip();
                    (1.0 - inverted_scaled_proportion.powf(*factor)) * 0.5
                }
            }
            FloatRange::Reversed(range) => 1.0 - range.normalize(plain),
        }
    }

    /// Unnormalize a normalized value. Will be clamped to `[0, 1]` if the plain, unnormalized value
    /// would exceed that range.
    pub fn unnormalize(&self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match self {
            FloatRange::Linear { min, max } => (normalized * (max - min)) + min,
            FloatRange::Skewed { min, max, factor } => {
                (normalized.powf(factor.recip()) * (max - min)) + min
            }
            FloatRange::SymmetricalSkewed {
                min,
                max,
                factor,
                center,
            } => {
                // Reconstructing the subranges works the same as with the normal skewed ranges
                let center_proportion = (center - min) / (max - min);
                let skewed_proportion = if normalized > 0.5 {
                    let scaled_proportion = (normalized - 0.5) * 2.0;
                    (scaled_proportion.powf(factor.recip()) * (1.0 - center_proportion))
                        + center_proportion
                } else {
                    let inverted_scaled_proportion = (0.5 - normalized) * 2.0;
                    (1.0 - inverted_scaled_proportion.powf(factor.recip())) * center_proportion
                };

                (skewed_proportion * (max - min)) + min
            }
            FloatRange::Reversed(range) => range.unnormalize(1.0 - normalized),
        }
    }

    /// The range's previous discrete step from a certain value with a certain step size. If the
    /// step size is not set, then the normalized range is split into 50 segments instead. If
    /// `finer` is true, then this is upped to 200 segments.
    pub fn previous_step(&self, from: f32, step_size: Option<f32>, finer: bool) -> f32 {
        // This one's slightly more involved than the integer version. We'll split the normalized
        // range up into 50 segments, but if `self.step_size` would cause the range to be devided
        // into less than 50 segments then we'll use that.
        match self {
            FloatRange::Linear { min, max }
            | FloatRange::Skewed { min, max, .. }
            | FloatRange::SymmetricalSkewed { min, max, .. } => {
                let normalized_naive_step_size = if finer { 0.005 } else { 0.02 };
                let naive_step =
                    self.unnormalize(self.normalize(from) - normalized_naive_step_size);

                match step_size {
                    // Use the naive step size if it is larger than the configured step size
                    Some(step_size) if (naive_step - from).abs() > step_size => {
                        self.snap_to_step(naive_step, step_size)
                    }
                    Some(step_size) => from - step_size,
                    None => naive_step,
                }
                .clamp(*min, *max)
            }
            FloatRange::Reversed(range) => range.next_step(from, step_size, finer),
        }
    }

    /// The range's next discrete step from a certain value with a certain step size. If the step
    /// size is not set, then the normalized range is split into 100 segments instead.
    pub fn next_step(&self, from: f32, step_size: Option<f32>, finer: bool) -> f32 {
        // See above
        match self {
            FloatRange::Linear { min, max }
            | FloatRange::Skewed { min, max, .. }
            | FloatRange::SymmetricalSkewed { min, max, .. } => {
                let normalized_naive_step_size = if finer { 0.005 } else { 0.02 };
                let naive_step =
                    self.unnormalize(self.normalize(from) + normalized_naive_step_size);

                match step_size {
                    Some(step_size) if (naive_step - from).abs() > step_size => {
                        self.snap_to_step(naive_step, step_size)
                    }
                    Some(step_size) => from + step_size,
                    None => naive_step,
                }
                .clamp(*min, *max)
            }
            FloatRange::Reversed(range) => range.previous_step(from, step_size, finer),
        }
    }

    /// Snap a value to a step size, clamping to the minimum and maximum value of the range.
    pub fn snap_to_step(&self, value: f32, step_size: f32) -> f32 {
        match self {
            FloatRange::Linear { min, max }
            | FloatRange::Skewed { min, max, .. }
            | FloatRange::SymmetricalSkewed { min, max, .. } => {
                ((value / step_size).round() * step_size).clamp(*min, *max)
            }
            FloatRange::Reversed(range) => range.snap_to_step(value, step_size),
        }
    }

    /// Emits debug assertions to make sure that range minima are always less than the maxima and
    /// that they are not equal.
    pub(super) fn assert_validity(&self) {
        match self {
            FloatRange::Linear { min, max }
            | FloatRange::Skewed { min, max, .. }
            | FloatRange::SymmetricalSkewed { min, max, .. } => {
                nih_debug_assert!(
                    min < max,
                    "The range minimum ({}) needs to be less than the range maximum ({}) and they \
                     cannot be equal",
                    min,
                    max
                );
            }
            FloatRange::Reversed(range) => range.assert_validity(),
        }
    }
}

impl IntRange {
    /// Normalize a plain, unnormalized value. Will be clamped to the bounds of the range if the
    /// normalized value exceeds `[0, 1]`.
    pub fn normalize(&self, plain: i32) -> f32 {
        match self {
            IntRange::Linear { min, max } => (plain - min) as f32 / (max - min) as f32,
            IntRange::Reversed(range) => 1.0 - range.normalize(plain),
        }
        .clamp(0.0, 1.0)
    }

    /// Unnormalize a normalized value. Will be clamped to `[0, 1]` if the plain, unnormalized value
    /// would exceed that range.
    pub fn unnormalize(&self, normalized: f32) -> i32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match self {
            IntRange::Linear { min, max } => (normalized * (max - min) as f32).round() as i32 + min,
            IntRange::Reversed(range) => range.unnormalize(1.0 - normalized),
        }
    }

    /// The range's previous discrete step from a certain value.
    pub fn previous_step(&self, from: i32) -> i32 {
        match self {
            IntRange::Linear { min, max } => (from - 1).clamp(*min, *max),
            IntRange::Reversed(range) => range.next_step(from),
        }
    }

    /// The range's next discrete step from a certain value.
    pub fn next_step(&self, from: i32) -> i32 {
        match self {
            IntRange::Linear { min, max } => (from + 1).clamp(*min, *max),
            IntRange::Reversed(range) => range.previous_step(from),
        }
    }

    /// The number of steps in this range. Used for the host's generic UI.
    pub fn step_count(&self) -> usize {
        match self {
            IntRange::Linear { min, max } => (max - min) as usize,
            IntRange::Reversed(range) => range.step_count(),
        }
    }

    /// If this range is wrapped in an adapter, like `Reversed`, then return the wrapped range.
    pub fn inner_range(&self) -> Self {
        match self {
            IntRange::Linear { .. } => *self,
            IntRange::Reversed(range) => range.inner_range(),
        }
    }

    /// Emits debug assertions to make sure that range minima are always less than the maxima and
    /// that they are not equal.
    pub(super) fn assert_validity(&self) {
        match self {
            IntRange::Linear { min, max } => {
                nih_debug_assert!(
                    min < max,
                    "The range minimum ({}) needs to be less than the range maximum ({}) and they \
                     cannot be equal",
                    min,
                    max
                );
            }
            IntRange::Reversed(range) => range.assert_validity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn make_linear_float_range() -> FloatRange {
        FloatRange::Linear {
            min: 10.0,
            max: 20.0,
        }
    }

    const fn make_linear_int_range() -> IntRange {
        IntRange::Linear { min: -10, max: 10 }
    }

    const fn make_skewed_float_range(factor: f32) -> FloatRange {
        FloatRange::Skewed {
            min: 10.0,
            max: 20.0,
            factor,
        }
    }

    const fn make_symmetrical_skewed_float_range(factor: f32) -> FloatRange {
        FloatRange::SymmetricalSkewed {
            min: 10.0,
            max: 20.0,
            factor,
            center: 12.5,
        }
    }

    #[test]
    fn step_size() {
        // These are weird step sizes, but if it works here then it will work for anything
        let range = make_linear_float_range();
        // XXX: We round to decimal places when outputting, but not when snapping to steps
        assert_eq!(range.snap_to_step(13.0, 4.73), 14.190001);
    }

    #[test]
    fn step_size_clamping() {
        let range = make_linear_float_range();
        assert_eq!(range.snap_to_step(10.0, 4.73), 10.0);
        assert_eq!(range.snap_to_step(20.0, 6.73), 20.0);
    }

    mod linear {
        use super::*;

        #[test]
        fn range_normalize_float() {
            let range = make_linear_float_range();
            assert_eq!(range.normalize(17.5), 0.75);
        }

        #[test]
        fn range_normalize_int() {
            let range = make_linear_int_range();
            assert_eq!(range.normalize(-5), 0.25);
        }

        #[test]
        fn range_unnormalize_float() {
            let range = make_linear_float_range();
            assert_eq!(range.unnormalize(0.25), 12.5);
        }

        #[test]
        fn range_unnormalize_int() {
            let range = make_linear_int_range();
            assert_eq!(range.unnormalize(0.75), 5);
        }

        #[test]
        fn range_unnormalize_int_rounding() {
            let range = make_linear_int_range();
            assert_eq!(range.unnormalize(0.73), 5);
        }
    }

    mod skewed {
        use super::*;

        #[test]
        fn range_normalize_float() {
            let range = make_skewed_float_range(FloatRange::skew_factor(-2.0));
            assert_eq!(range.normalize(17.5), 0.9306049);
        }

        #[test]
        fn range_unnormalize_float() {
            let range = make_skewed_float_range(FloatRange::skew_factor(-2.0));
            assert_eq!(range.unnormalize(0.9306049), 17.5);
        }

        #[test]
        fn range_normalize_linear_equiv_float() {
            let linear_range = make_linear_float_range();
            let skewed_range = make_skewed_float_range(1.0);
            assert_eq!(linear_range.normalize(17.5), skewed_range.normalize(17.5));
        }

        #[test]
        fn range_unnormalize_linear_equiv_float() {
            let linear_range = make_linear_float_range();
            let skewed_range = make_skewed_float_range(1.0);
            assert_eq!(
                linear_range.unnormalize(0.25),
                skewed_range.unnormalize(0.25)
            );
        }
    }

    mod symmetrical_skewed {
        use super::*;

        #[test]
        fn range_normalize_float() {
            let range = make_symmetrical_skewed_float_range(FloatRange::skew_factor(-2.0));
            assert_eq!(range.normalize(17.5), 0.951801);
        }

        #[test]
        fn range_unnormalize_float() {
            let range = make_symmetrical_skewed_float_range(FloatRange::skew_factor(-2.0));
            assert_eq!(range.unnormalize(0.951801), 17.5);
        }
    }

    mod reversed_linear {
        use super::*;

        #[test]
        fn range_normalize_int() {
            const WRAPPED_RANGE: IntRange = make_linear_int_range();
            let range = IntRange::Reversed(&WRAPPED_RANGE);
            assert_eq!(range.normalize(-5), 1.0 - 0.25);
        }

        #[test]
        fn range_unnormalize_int() {
            const WRAPPED_RANGE: IntRange = make_linear_int_range();
            let range = IntRange::Reversed(&WRAPPED_RANGE);
            assert_eq!(range.unnormalize(1.0 - 0.75), 5);
        }

        #[test]
        fn range_unnormalize_int_rounding() {
            const WRAPPED_RANGE: IntRange = make_linear_int_range();
            let range = IntRange::Reversed(&WRAPPED_RANGE);
            assert_eq!(range.unnormalize(1.0 - 0.73), 5);
        }
    }

    mod reversed_skewed {
        use super::*;

        #[test]
        fn range_normalize_float() {
            const WRAPPED_RANGE: FloatRange = make_skewed_float_range(0.25);
            let range = FloatRange::Reversed(&WRAPPED_RANGE);
            assert_eq!(range.normalize(17.5), 1.0 - 0.9306049);
        }

        #[test]
        fn range_unnormalize_float() {
            const WRAPPED_RANGE: FloatRange = make_skewed_float_range(0.25);
            let range = FloatRange::Reversed(&WRAPPED_RANGE);
            assert_eq!(range.unnormalize(1.0 - 0.9306049), 17.5);
        }

        #[test]
        fn range_normalize_linear_equiv_float() {
            const WRAPPED_LINEAR_RANGE: FloatRange = make_linear_float_range();
            const WRAPPED_SKEWED_RANGE: FloatRange = make_skewed_float_range(1.0);
            let linear_range = FloatRange::Reversed(&WRAPPED_LINEAR_RANGE);
            let skewed_range = FloatRange::Reversed(&WRAPPED_SKEWED_RANGE);
            assert_eq!(linear_range.normalize(17.5), skewed_range.normalize(17.5));
        }

        #[test]
        fn range_unnormalize_linear_equiv_float() {
            const WRAPPED_LINEAR_RANGE: FloatRange = make_linear_float_range();
            const WRAPPED_SKEWED_RANGE: FloatRange = make_skewed_float_range(1.0);
            let linear_range = FloatRange::Reversed(&WRAPPED_LINEAR_RANGE);
            let skewed_range = FloatRange::Reversed(&WRAPPED_SKEWED_RANGE);
            assert_eq!(
                linear_range.unnormalize(0.25),
                skewed_range.unnormalize(0.25)
            );
        }
    }
}
