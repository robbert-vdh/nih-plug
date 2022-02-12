//! Convenience functions for formatting and parsing parameter values in common formats.

use std::sync::Arc;

/// Round an `f32` value to always have a specific number of decimal digits.
pub fn f32_rounded(digits: usize) -> Option<Arc<dyn Fn(f32) -> String + Send + Sync>> {
    Some(Arc::new(move |x| format!("{:.digits$}", x)))
}
