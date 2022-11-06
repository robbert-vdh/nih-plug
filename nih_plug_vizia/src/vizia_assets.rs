//! Registration functions for Vizia's built-in fonts. These are not enabled by default in
//! `nih_plug_vizia` to save on binary size.

use vizia::prelude::*;

// This module provides a re-export and simple font wrappers around the re-exported fonts.
pub use vizia::fonts;

/// The font name for the Roboto (Regular) font, needs to be registered using [`register_roboto()`]
/// first.
pub const ROBOTO: &str = "roboto";
/// The font name for the Roboto Bold font, needs to be registered using [`register_roboto_bold()`]
/// first.
pub const ROBOTO_BOLD: &str = "roboto-bold";
/// The font name for the icon font (Entypo), needs to be registered using [`register_icons()`]
/// first.
pub const ICONS: &str = "icons";
/// The font name for the emoji font (Open Sans Eomji), needs to be registered using
/// [`register_emoji()`] first.
pub const EMOJI: &str = "emoji";
/// The font name for the arabic font (Amiri Regular), needs to be registered using
/// [`register_arabic()`] first.
pub const ARABIC: &str = "arabic";
/// The font name for the material font (Material Icons), needs to be registered using
/// [`register_material()`] first.
pub const MATERIAL: &str = "material";

pub fn register_roboto(cx: &mut Context) {
    cx.add_font_mem(ROBOTO, fonts::ROBOTO_REGULAR);
}
pub fn register_roboto_bold(cx: &mut Context) {
    cx.add_font_mem(ROBOTO_BOLD, fonts::ROBOTO_BOLD);
}
pub fn register_icons(cx: &mut Context) {
    cx.add_font_mem(ICONS, fonts::ENTYPO);
}
pub fn register_emoji(cx: &mut Context) {
    cx.add_font_mem(EMOJI, fonts::OPEN_SANS_EMOJI);
}
pub fn register_arabic(cx: &mut Context) {
    cx.add_font_mem(ARABIC, fonts::AMIRI_REGULAR);
}
pub fn register_material(cx: &mut Context) {
    cx.add_font_mem(MATERIAL, fonts::MATERIAL_ICONS_REGULAR);
}
