//! Utilities for writing VIZIA widgets.

use vizia::prelude::*;

/// An extension trait for [`Modifiers`] that adds platform-independent getters.
pub trait ModifiersExt {
    /// Returns true if the Command (on macOS) or Ctrl (on any other platform) key is pressed.
    fn command(&self) -> bool;

    /// Returns true if the Alt (or Option on macOS) key is pressed.
    fn alt(&self) -> bool;

    /// Returns true if the Shift key is pressed.
    fn shift(&self) -> bool;
}

impl ModifiersExt for Modifiers {
    fn command(&self) -> bool {
        #[cfg(target_os = "macos")]
        let result = self.contains(Modifiers::LOGO);

        #[cfg(not(target_os = "macos"))]
        let result = self.contains(Modifiers::CTRL);

        result
    }

    fn alt(&self) -> bool {
        self.contains(Modifiers::ALT)
    }

    fn shift(&self) -> bool {
        self.contains(Modifiers::SHIFT)
    }
}

/// Remap a `[0, 1]` value to an x-coordinate within the current entity's bounding box. The value
/// will be clamped to `[0, 1]` if it isn't already in that range. This ignores the border width.
pub fn remap_current_entity_x_t(cx: &EventContext, t: f32) -> f32 {
    let border_width = cx.border_width();
    let x_pos = cx.cache.get_posx(cx.current()) + border_width;
    let width = cx.cache.get_width(cx.current()) - (border_width * 2.0);
    x_pos + (width * t.clamp(0.0, 1.0))
}

/// Remap a `[0, 1]` value to a y-coordinate within the current entity's bounding box. The value
/// will be clamped to `[0, 1]` if it isn't already in that range. This ignores the border width.
///
/// # Note
///
/// As y-coordinates increase from top to bottom, this value is likely the inverse of what you want.
/// 0.0 is the bottom of the enitty and 1.0 corresponds to the top of the entity.
pub fn remap_current_entity_y_t(cx: &EventContext, t: f32) -> f32 {
    let border_width = cx.border_width();
    let y_pos = cx.cache.get_posy(cx.current()) + border_width;
    let height = cx.cache.get_height(cx.current()) - (border_width * 2.0);
    y_pos + (height * t.clamp(0.0, 1.0))
}

/// Remap an x-coordinate to a `[0, 1]` value within the current entity's bounding box. The value
/// will be clamped to `[0, 1]` if it isn't already in that range. This ignores the border width.
pub fn remap_current_entity_x_coordinate(cx: &EventContext, x_coord: f32) -> f32 {
    let border_width = cx.border_width();
    let x_pos = cx.cache.get_posx(cx.current()) + border_width;
    let width = cx.cache.get_width(cx.current()) - (border_width * 2.0);
    ((x_coord - x_pos) / width).clamp(0.0, 1.0)
}

/// Remap an y-coordinate to a `[0, 1]` value within the current entity's bounding box. The value
/// will be clamped to `[0, 1]` if it isn't already in that range. This ignores the border width.
///
/// # Note
///
/// As y-coordinates increase from top to bottom, this value is likely the inverse of what you want.
/// 0.0 is the bottom of the enitty and 1.0 corresponds to the top of the entity.
pub fn remap_current_entity_y_coordinate(cx: &EventContext, y_coord: f32) -> f32 {
    let border_width = cx.border_width();
    let y_pos = cx.cache.get_posy(cx.current()) + border_width;
    let height = cx.cache.get_height(cx.current()) - (border_width * 2.0);
    ((y_coord - y_pos) / height).clamp(0.0, 1.0)
}
