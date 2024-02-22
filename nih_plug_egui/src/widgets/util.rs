//! Utilities for creating these widgets.

use egui_baseview::egui::{self, Color32};

/// Additively modify the hue, saturation, and lightness [0, 1] values of a color.
pub fn add_hsv(color: Color32, h: f32, s: f32, v: f32) -> Color32 {
    let mut hsv = egui::epaint::Hsva::from(color);
    hsv.h += h;
    hsv.s += s;
    hsv.v += v;
    hsv.into()
}

/// Multiplicatively modify the hue, saturation, and lightness [0, 1] values of a color.
pub fn scale_hsv(color: Color32, h: f32, s: f32, v: f32) -> Color32 {
    let mut hsv = egui::epaint::Hsva::from(color);
    hsv.h *= h;
    hsv.s *= s;
    hsv.v *= v;
    hsv.into()
}
