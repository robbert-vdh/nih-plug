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

//! Different ranges for numeric parameters.

/// A distribution for a parameter's range. All range endpoints are inclusive.
#[derive(Debug)]
pub enum Range<T> {
    /// The values are uniformly distributed between `min` and `max`.
    Linear { min: T, max: T },
    /// The range is skewed by a factor. Values above 1.0 will make the end of the range wider,
    /// while values between 0 and 1 will skew the range towards the start. Use [Range::skew_factor]
    /// for a more intuitively way to calculate the skew factor where positive values skew the range
    /// towards the end while negative values skew the range toward the start.
    Skewed { min: T, max: T, factor: f32 },
    /// The same as [Range::Skewed], but with the skewing happening from a central point.
    SymmetricalSkewed {
        min: T,
        max: T,
        factor: f32,
        center: T,
    },
}

impl Range<()> {
    /// Calculate a skew factor for [Range::Skewed] and [Range::SymmetricalSkewed]. Positive values
    /// make the end of the range wider while negative make the start of the range wider.
    pub fn skew_factor(factor: f32) -> f32 {
        2.0f32.powf(factor)
    }
}

/// A normalizable range for type `T`, where `self` is expected to be a type `R<T>`. Higher kinded
/// types would have made this trait definition a lot clearer.
pub(crate) trait NormalizebleRange<T> {
    /// Normalize a plain, unnormalized value. Will be clamped to the bounds of the range if the
    /// normalized value exceeds `[0, 1]`.
    fn normalize(&self, plain: T) -> f32;

    /// Unnormalize a normalized value. Will be clamped to `[0, 1]` if the plain, unnormalized value
    /// would exceed that range.
    fn unnormalize(&self, normalized: f32) -> T;
}

impl Default for Range<f32> {
    fn default() -> Self {
        Self::Linear { min: 0.0, max: 1.0 }
    }
}

impl Default for Range<i32> {
    fn default() -> Self {
        Self::Linear { min: 0, max: 1 }
    }
}

impl NormalizebleRange<f32> for Range<f32> {
    fn normalize(&self, plain: f32) -> f32 {
        match &self {
            Range::Linear { min, max } => (plain - min) / (max - min),
            Range::Skewed { min, max, factor } => ((plain - min) / (max - min)).powf(*factor),
            Range::SymmetricalSkewed {
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
                    (scaled_proportion.powf(*factor) * (1.0 - center_proportion))
                        + center_proportion
                } else {
                    // The part below the center gets scaled, inverted (so the range is [0, 1] where
                    // 0 corresponds to the center proportion and 1 corresponds to the orignal
                    // normalized 0 value), skewed, inverted back again, and then scaled back to the
                    // original range
                    let inverted_scaled_proportion =
                        (center_proportion - unscaled_proportion) * (center_proportion).recip();
                    (1.0 - inverted_scaled_proportion.powf(*factor)) * (center_proportion)
                }
            }
        }
        .clamp(0.0, 1.0)
    }

    fn unnormalize(&self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match &self {
            Range::Linear { min, max } => (normalized * (max - min)) + min,
            Range::Skewed { min, max, factor } => {
                (normalized.powf(factor.recip()) * (max - min)) + min
            }
            Range::SymmetricalSkewed {
                min,
                max,
                factor,
                center,
            } => {
                // Reconstructing the subranges works the same as with the normal skewed ranges
                let center_proportion = (center - min) / (max - min);
                let skewed_proportion = if normalized > center_proportion {
                    let scaled_proportion =
                        (normalized - center_proportion) * (1.0 - center_proportion).recip();
                    (scaled_proportion.powf(factor.recip()) * (1.0 - center_proportion))
                        + center_proportion
                } else {
                    let inverted_scaled_proportion =
                        (center_proportion - normalized) * (center_proportion).recip();
                    (1.0 - inverted_scaled_proportion.powf(factor.recip())) * (center_proportion)
                };

                (skewed_proportion * (max - min)) + min
            }
        }
    }
}

impl NormalizebleRange<i32> for Range<i32> {
    fn normalize(&self, plain: i32) -> f32 {
        match &self {
            Range::Linear { min, max } => (plain - min) as f32 / (max - min) as f32,
            Range::Skewed { min, max, factor } => {
                ((plain - min) as f32 / (max - min) as f32).powf(*factor)
            }
            Range::SymmetricalSkewed {
                min,
                max,
                factor,
                center,
            } => {
                // See the comments in the float version
                let unscaled_proportion = (plain - min) as f32 / (max - min) as f32;
                let center_proportion = (center - min) as f32 / (max - min) as f32;
                if unscaled_proportion > center_proportion {
                    let scaled_proportion = (unscaled_proportion - center_proportion)
                        * (1.0 - center_proportion).recip();
                    (scaled_proportion.powf(*factor) * (1.0 - center_proportion))
                        + center_proportion
                } else {
                    let inverted_scaled_proportion =
                        (center_proportion - unscaled_proportion) * (center_proportion).recip();
                    (1.0 - inverted_scaled_proportion.powf(*factor)) * (center_proportion)
                }
            }
        }
        .clamp(0.0, 1.0)
    }

    fn unnormalize(&self, normalized: f32) -> i32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match &self {
            Range::Linear { min, max } => (normalized * (max - min) as f32).round() as i32 + min,
            Range::Skewed { min, max, factor } => {
                (normalized.powf(factor.recip()) * (max - min) as f32).round() as i32 + min
            }
            Range::SymmetricalSkewed {
                min,
                max,
                factor,
                center,
            } => {
                let center_proportion = (center - min) as f32 / (max - min) as f32;
                let skewed_proportion = if normalized > center_proportion {
                    let scaled_proportion =
                        (normalized - center_proportion) * (1.0 - center_proportion).recip();
                    (scaled_proportion.powf(factor.recip()) * (1.0 - center_proportion))
                        + center_proportion
                } else {
                    let inverted_scaled_proportion =
                        (center_proportion - normalized) * (center_proportion).recip();
                    (1.0 - inverted_scaled_proportion.powf(factor.recip())) * (center_proportion)
                };

                (skewed_proportion * (max - min) as f32).round() as i32 + min
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_linear_float_range() -> Range<f32> {
        Range::Linear {
            min: 10.0,
            max: 20.0,
        }
    }

    fn make_linear_int_range() -> Range<i32> {
        Range::Linear { min: -10, max: 10 }
    }

    fn make_skewed_float_range(factor: f32) -> Range<f32> {
        Range::Skewed {
            min: 10.0,
            max: 20.0,
            factor,
        }
    }

    fn make_skewed_int_range(factor: f32) -> Range<i32> {
        Range::Skewed {
            min: -10,
            max: 10,
            factor,
        }
    }

    fn make_symmetrical_skewed_float_range(factor: f32) -> Range<f32> {
        Range::SymmetricalSkewed {
            min: 10.0,
            max: 20.0,
            factor,
            center: 12.5,
        }
    }

    fn make_symmetrical_skewed_int_range(factor: f32) -> Range<i32> {
        Range::SymmetricalSkewed {
            min: -10,
            max: 10,
            factor,
            center: -3,
        }
    }

    mod linear {
        use super::super::*;
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
        use super::super::*;
        use super::*;

        #[test]
        fn range_normalize_float() {
            let range = make_skewed_float_range(Range::skew_factor(-2.0));
            assert_eq!(range.normalize(17.5), 0.9306048591020996);
        }

        #[test]
        fn range_normalize_int() {
            let range = make_skewed_int_range(Range::skew_factor(-2.0));
            assert_eq!(range.normalize(-5), 0.7071067811865476);
        }

        #[test]
        fn range_unnormalize_float() {
            let range = make_skewed_float_range(Range::skew_factor(-2.0));
            assert_eq!(range.unnormalize(0.9306048591020996), 17.5);
        }

        #[test]
        fn range_unnormalize_int() {
            let range = make_skewed_int_range(Range::skew_factor(-2.0));
            assert_eq!(range.unnormalize(0.7071067811865476), -5);
        }

        #[test]
        fn range_normalize_linear_equiv_float() {
            let linear_range = make_linear_float_range();
            let skewed_range = make_skewed_float_range(1.0);
            assert_eq!(linear_range.normalize(17.5), skewed_range.normalize(17.5));
        }

        #[test]
        fn range_normalize_linear_equiv_int() {
            let linear_range = make_linear_int_range();
            let skewed_range = make_skewed_int_range(1.0);
            assert_eq!(linear_range.normalize(-5), skewed_range.normalize(-5));
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

        #[test]
        fn range_unnormalize_linear_equiv_int() {
            let linear_range = make_linear_int_range();
            let skewed_range = make_skewed_int_range(1.0);
            assert_eq!(
                linear_range.unnormalize(0.25),
                skewed_range.unnormalize(0.25)
            );
        }

        #[test]
        fn range_unnormalize_linear_equiv_int_rounding() {
            let linear_range = make_linear_int_range();
            let skewed_range = make_skewed_int_range(1.0);
            assert_eq!(
                linear_range.unnormalize(0.73),
                skewed_range.unnormalize(0.73)
            );
        }
    }

    mod symmetrical_skewed {
        use super::super::*;
        use super::*;

        #[test]
        fn range_normalize_float() {
            let range = make_symmetrical_skewed_float_range(Range::skew_factor(-2.0));
            assert_eq!(range.normalize(17.5), 0.9277015);
        }

        #[test]
        fn range_normalize_int() {
            let range = make_symmetrical_skewed_int_range(Range::skew_factor(-2.0));
            assert_eq!(range.normalize(-5), 0.09411134);
        }

        #[test]
        fn range_unnormalize_float() {
            let range = make_symmetrical_skewed_float_range(Range::skew_factor(-2.0));
            assert_eq!(range.unnormalize(0.9277015), 17.5);
        }

        #[test]
        fn range_unnormalize_int() {
            let range = make_symmetrical_skewed_int_range(Range::skew_factor(-2.0));
            assert_eq!(range.unnormalize(0.09411134), -5);
        }
    }
}
