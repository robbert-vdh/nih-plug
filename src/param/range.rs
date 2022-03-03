//! Different ranges for numeric parameters.

/// A distribution for a floating point parameter's range. All range endpoints are inclusive.
#[derive(Debug)]
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
}

/// A distribution for an integer parameter's range. All range endpoints are inclusive. Only linear
/// ranges are supported for integers since hosts expect discrete parameters to have a fixed step
/// size.
#[derive(Debug)]
pub enum IntRange {
    /// The values are uniformly distributed between `min` and `max`.
    Linear { min: i32, max: i32 },
}

impl Default for FloatRange {
    fn default() -> Self {
        Self::Linear { min: 0.0, max: 1.0 }
    }
}

impl Default for IntRange {
    fn default() -> Self {
        Self::Linear { min: 0, max: 1 }
    }
}

impl FloatRange {
    /// Calculate a skew factor for [`FloatRange::Skewed`] and [`FloatRange::SymmetricalSkewed`].
    /// Positive values make the end of the range wider while negative make the start of the range
    /// wider.
    pub fn skew_factor(factor: f32) -> f32 {
        2.0f32.powf(factor)
    }

    /// Normalize a plain, unnormalized value. Will be clamped to the bounds of the range if the
    /// normalized value exceeds `[0, 1]`.
    pub fn normalize(&self, plain: f32) -> f32 {
        match &self {
            FloatRange::Linear { min, max } => (plain - min) / (max - min),
            FloatRange::Skewed { min, max, factor } => ((plain - min) / (max - min)).powf(*factor),
            FloatRange::SymmetricalSkewed {
                min,
                max,
                factor,
                center,
            } => {
                // There's probably a much faster equivalent way to write this. Also, I have no clue
                // how I managed to implement this correctly on the first try.
                let unscaled_proportion = (plain - min) / (max - min);
                let center_proportion = (center - min) / (max - min);
                if unscaled_proportion > center_proportion {
                    // The part above the center gets normalized to a [0, 1] range, skewed, and then
                    // unnormalized and scaled back to the original [center_proportion, 1] range
                    let scaled_proportion = (unscaled_proportion - center_proportion)
                        * (1.0 - center_proportion).recip();
                    (scaled_proportion.powf(*factor) * 0.5) + 0.5
                } else {
                    // The part below the center gets scaled, inverted (so the range is [0, 1] where
                    // 0 corresponds to the center proportion and 1 corresponds to the orignal
                    // normalized 0 value), skewed, inverted back again, and then scaled back to the
                    // original range
                    let inverted_scaled_proportion =
                        (center_proportion - unscaled_proportion) * (center_proportion).recip();
                    (1.0 - inverted_scaled_proportion.powf(*factor)) * 0.5
                }
            }
        }
        .clamp(0.0, 1.0)
    }

    /// Unnormalize a normalized value. Will be clamped to `[0, 1]` if the plain, unnormalized value
    /// would exceed that range.
    pub fn unnormalize(&self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match &self {
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
        }
    }

    /// Snap a vlue to a step size, clamping to the minimum and maximum value of the range.
    pub fn snap_to_step(&self, value: f32, step_size: f32) -> f32 {
        let (min, max) = match &self {
            FloatRange::Linear { min, max } => (min, max),
            FloatRange::Skewed { min, max, .. } => (min, max),
            FloatRange::SymmetricalSkewed { min, max, .. } => (min, max),
        };

        ((value / step_size).round() * step_size).clamp(*min, *max)
    }
}

impl IntRange {
    /// Normalize a plain, unnormalized value. Will be clamped to the bounds of the range if the
    /// normalized value exceeds `[0, 1]`.
    pub fn normalize(&self, plain: i32) -> f32 {
        match &self {
            IntRange::Linear { min, max } => (plain - min) as f32 / (max - min) as f32,
        }
        .clamp(0.0, 1.0)
    }

    /// Unnormalize a normalized value. Will be clamped to `[0, 1]` if the plain, unnormalized value
    /// would exceed that range.
    pub fn unnormalize(&self, normalized: f32) -> i32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match &self {
            IntRange::Linear { min, max } => (normalized * (max - min) as f32).round() as i32 + min,
        }
    }

    /// The number of steps in this range, if it is stepped. Used for the host's generic UI.
    pub fn step_count(&self) -> Option<usize> {
        match self {
            IntRange::Linear { min, max } => Some((max - min) as usize),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_linear_float_range() -> FloatRange {
        FloatRange::Linear {
            min: 10.0,
            max: 20.0,
        }
    }

    fn make_linear_int_range() -> IntRange {
        IntRange::Linear { min: -10, max: 10 }
    }

    fn make_skewed_float_range(factor: f32) -> FloatRange {
        FloatRange::Skewed {
            min: 10.0,
            max: 20.0,
            factor,
        }
    }

    fn make_symmetrical_skewed_float_range(factor: f32) -> FloatRange {
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
}
