//! Utilities for creating these widgets.

use crate::Rectangle;

/// Remap a `[0, 1]` value to an x-coordinate within this rectangle. The value will be clamped to
/// `[0, 1]` if it isn't already in that range.
pub fn remap_rect_x_t(rect: &Rectangle, t: f32) -> f32 {
    rect.x + (rect.width * t.clamp(0.0, 1.0))
}

/// Remap a `[0, 1]` value to a y-coordinate within this rectangle. The value will be clamped to
/// `[0, 1]` if it isn't already in that range.
pub fn remap_rect_y_t(rect: &Rectangle, t: f32) -> f32 {
    rect.y + (rect.height * t.clamp(0.0, 1.0))
}

/// Remap an x-coordinate to a `[0, 1]` value within this rectangle. The value will be clamped to
/// `[0, 1]` if it isn't already in that range.
pub fn remap_rect_x_coordinate(rect: &Rectangle, x_coord: f32) -> f32 {
    ((x_coord - rect.x) / rect.width).clamp(0.0, 1.0)
}

/// Remap a y-coordinate to a `[0, 1]` value within this rectangle. The value will be clamped to
/// `[0, 1]` if it isn't already in that range.
pub fn remap_rect_y_coordinate(rect: &Rectangle, y_coord: f32) -> f32 {
    ((y_coord - rect.y) / rect.height).clamp(0.0, 1.0)
}
