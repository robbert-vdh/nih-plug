//! Utilities for creating these widgets.

use crate::Rectangle;

/// Remap a `[0, 1]` value to an x-coordinate within this rectangle. The value will be clamped to
/// `[0, 1]` if it isn't already in that range.
pub fn remap_rect_x(rect: &Rectangle, t: f32) -> f32 {
    rect.x + (rect.width * t.clamp(0.0, 1.0))
}

/// Remap a `[0, 1]` value to a y-coordinate within this rectangle. The value will be clamped to
/// `[0, 1]` if it isn't already in that range.
pub fn remap_rect_y(rect: &Rectangle, t: f32) -> f32 {
    rect.y + (rect.height * t.clamp(0.0, 1.0))
}
