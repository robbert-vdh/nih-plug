//! Binary assets for use with `nih_plug_vizia`. These fonts first need to be registered by calling
//! [`nih_plug_vizia::assets::register_fonts()`][register_fonts()].

use vizia::prelude::*;

// This module provides a re-export and simple font wrappers around the re-exported fonts.
pub use nih_plug_assets::*;

/// Register the fonts from this module so they can be used with VIZIA. This is automatically called
/// for you when using [`create_vizia_editor()`][super::create_vizia_editor()].
pub fn register_fonts(cx: &mut Context) {
    cx.add_font_mem(NOTO_SANS_REGULAR, fonts::NOTO_SANS_REGULAR);
    cx.add_font_mem(NOTO_SANS_REGULAR_ITALIC, fonts::NOTO_SANS_REGULAR_ITALIC);
    cx.add_font_mem(NOTO_SANS_THIN, fonts::NOTO_SANS_THIN);
    cx.add_font_mem(NOTO_SANS_THIN_ITALIC, fonts::NOTO_SANS_THIN_ITALIC);
    cx.add_font_mem(NOTO_SANS_LIGHT, fonts::NOTO_SANS_LIGHT);
    cx.add_font_mem(NOTO_SANS_LIGHT_ITALIC, fonts::NOTO_SANS_LIGHT_ITALIC);
    cx.add_font_mem(NOTO_SANS_BOLD, fonts::NOTO_SANS_BOLD);
    cx.add_font_mem(NOTO_SANS_BOLD_ITALIC, fonts::NOTO_SANS_BOLD_ITALIC);
}

pub const NOTO_SANS_REGULAR: &str = "Noto Sans Regular";
pub const NOTO_SANS_REGULAR_ITALIC: &str = "Noto Sans Regular Italic";
pub const NOTO_SANS_THIN: &str = "Noto Sans Thin";
pub const NOTO_SANS_THIN_ITALIC: &str = "Noto Sans Thin Italic";
pub const NOTO_SANS_LIGHT: &str = "Noto Sans Light";
pub const NOTO_SANS_LIGHT_ITALIC: &str = "Noto Sans Light Italic";
pub const NOTO_SANS_BOLD: &str = "Noto Sans Bold";
pub const NOTO_SANS_BOLD_ITALIC: &str = "Noto Sans Bold Italic";
