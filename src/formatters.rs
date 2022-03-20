//! Convenience functions for formatting and parsing parameter values in common formats.

use std::sync::Arc;

/// Round an `f32` value to always have a specific number of decimal digits.
pub fn f32_rounded(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| format!("{:.digits$}", x))
}

/// Turn an `f32` value from linear to dBFS (reference value 1)
pub fn f32_lin_to_db(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| format!("{:.digits$}", x.log10() * 20.0))
}

/// Turn an `f32` value from dBFS (reference value 1) to linear
pub fn f32_db_to_lin(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| format!("{:.digits$}", 10f32.powf(x / 20.0)))
}

/// Round an `f32` value and divide it by 1000 when it gets over 1000
pub fn f32_hertz_then_khz(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| {
        if x < 1000.0 {
            format!("{:.digits$}", x)
        } else {
            let digits = digits + 1;
            format!("{:.digits$}", x / 1000.0)
        }
    })
}

/// Format a `[0, 1]` number as a percentage. Does not include the percent sign, you should specify
/// this as the parameter's unit.
pub fn f32_percentage(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| format!("{:.digits$}", value * 100.0))
}

/// Parse a `[0, 100]` percentage to a `[0, 1]` number. Handles the percentage unit for you. Used in
/// conjunction with [`f32_percentage`].
pub fn from_f32_percentage() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        string
            .trim_end_matches(&[' ', '%'])
            .parse()
            .ok()
            .map(|x: f32| x / 100.0)
    })
}

/// Format an order/power of two. Useful in conjunction with [`from_power_of_two()`] to limit
/// integer parameter ranges to be only powers of two.
pub fn i32_power_of_two() -> Arc<dyn Fn(i32) -> String + Send + Sync> {
    Arc::new(|value| format!("{}", 1 << value))
}

/// Parse a parameter input string to a power of two. Useful in conjunction with [`power_of_two()`]
/// to limit integer parameter ranges to be only powers of two.
pub fn from_i32_power_of_two() -> Arc<dyn Fn(&str) -> Option<i32> + Send + Sync> {
    Arc::new(|string| string.parse().ok().map(|n: i32| (n as f32).log2() as i32))
}
