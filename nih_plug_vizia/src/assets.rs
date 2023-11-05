//! Binary assets for use with `nih_plug_vizia`. These fonts first need to be registered using their
//! associated registration function.

use vizia::prelude::*;

// This module provides a re-export and simple font wrappers around the re-exported fonts.
pub use nih_plug_assets::*;

/// The font name for the Noto Sans font family. Comes in regular, thin, light and bold versions,
/// with italic variations for each. Register the variations you want to use with
/// [`register_noto_sans_regular()`], [`register_noto_sans_regular_italic()`],
/// [`register_noto_sans_thin()`], [`register_noto_sans_thin_italic()`],
/// [`register_noto_sans_light()`], [`register_noto_sans_light_italic()`],
/// [`register_noto_sans_bold()`], and [`register_noto_sans_bold_italic()`], Use the font weight and
/// font style properties to select a specific variation.
pub const NOTO_SANS: &str = "Noto Sans";

pub fn register_noto_sans_regular(cx: &mut Context) {
    cx.add_font_mem(fonts::NOTO_SANS_REGULAR);
}
pub fn register_noto_sans_regular_italic(cx: &mut Context) {
    cx.add_font_mem(fonts::NOTO_SANS_REGULAR_ITALIC);
}
pub fn register_noto_sans_thin(cx: &mut Context) {
    cx.add_font_mem(fonts::NOTO_SANS_THIN);
}
pub fn register_noto_sans_thin_italic(cx: &mut Context) {
    cx.add_font_mem(fonts::NOTO_SANS_THIN_ITALIC);
}
pub fn register_noto_sans_light(cx: &mut Context) {
    cx.add_font_mem(fonts::NOTO_SANS_LIGHT);
}
pub fn register_noto_sans_light_italic(cx: &mut Context) {
    cx.add_font_mem(fonts::NOTO_SANS_LIGHT_ITALIC);
}
pub fn register_noto_sans_bold(cx: &mut Context) {
    cx.add_font_mem(fonts::NOTO_SANS_BOLD);
}
pub fn register_noto_sans_bold_italic(cx: &mut Context) {
    cx.add_font_mem(fonts::NOTO_SANS_BOLD_ITALIC);
}
