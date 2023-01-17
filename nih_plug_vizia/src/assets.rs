//! Binary assets for use with `nih_plug_vizia`. These fonts first need to be registered using their
//! associated registration function.

use vizia::prelude::*;

// This module provides a re-export and simple font wrappers around the re-exported fonts.
pub use nih_plug_assets::*;

/// The font name for Noto Sans Regular, needs to be registered using
/// [`register_noto_sans_regular()`] first.
pub const NOTO_SANS_REGULAR: &str = "Noto Sans Regular";
/// The font name for Noto Sans Regular Italic, needs to be registered using
/// [`register_noto_sans_regular_italic()`] first.
pub const NOTO_SANS_REGULAR_ITALIC: &str = "Noto Sans Regular Italic";
/// The font name for Noto Sans Thin, needs to be registered using [`register_noto_sans_thin()`]
/// first.
pub const NOTO_SANS_THIN: &str = "Noto Sans Thin";
/// The font name for Noto Sans Thin Italic, needs to be registered using
/// [`register_noto_sans_thin_italic()`] first.
pub const NOTO_SANS_THIN_ITALIC: &str = "Noto Sans Thin Italic";
/// The font name for Noto Sans Light, needs to be registered using [`register_noto_sans_light()`]
/// first.
pub const NOTO_SANS_LIGHT: &str = "Noto Sans Light";
/// The font name for Noto Sans Light Italic, needs to be registered using
/// [`register_noto_sans_light_italic()`] first.
pub const NOTO_SANS_LIGHT_ITALIC: &str = "Noto Sans Light Italic";
/// The font name for Noto Sans Bold, needs to be registered using [`register_noto_sans_bold()`]
/// first.
// NOTE: I'd expect this to be an alias for Noto Sans Regular but this is what cosmic-text thinks
//       the font is called
pub const NOTO_SANS_BOLD: &str = "Noto Sans";
/// The font name for Noto Sans Bold Italic, needs to be registered using
/// [`register_noto_sans_bold_italic()`] first.
pub const NOTO_SANS_BOLD_ITALIC: &str = "Noto Sans Italic";

pub fn register_noto_sans_regular(cx: &mut Context) {
    cx.add_fonts_mem(&[fonts::NOTO_SANS_REGULAR]);
}
pub fn register_noto_sans_regular_italic(cx: &mut Context) {
    cx.add_fonts_mem(&[fonts::NOTO_SANS_REGULAR_ITALIC]);
}
pub fn register_noto_sans_thin(cx: &mut Context) {
    cx.add_fonts_mem(&[fonts::NOTO_SANS_THIN]);
}
pub fn register_noto_sans_thin_italic(cx: &mut Context) {
    cx.add_fonts_mem(&[fonts::NOTO_SANS_THIN_ITALIC]);
}
pub fn register_noto_sans_light(cx: &mut Context) {
    cx.add_fonts_mem(&[fonts::NOTO_SANS_LIGHT]);
}
pub fn register_noto_sans_light_italic(cx: &mut Context) {
    cx.add_fonts_mem(&[fonts::NOTO_SANS_LIGHT_ITALIC]);
}
pub fn register_noto_sans_bold(cx: &mut Context) {
    cx.add_fonts_mem(&[fonts::NOTO_SANS_BOLD]);
}
pub fn register_noto_sans_bold_italic(cx: &mut Context) {
    cx.add_fonts_mem(&[fonts::NOTO_SANS_BOLD_ITALIC]);
}
