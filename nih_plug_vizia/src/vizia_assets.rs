//! Registration functions for Vizia's built-in fonts. These are not enabled by default in
//! `nih_plug_vizia` to save on binary size.

use vizia::prelude::*;

// This module provides a re-export and simple font wrappers around the re-exported fonts.
pub use vizia::fonts;

/// The font name for the Roboto font family. Comes in regular, bold, and italic variations.
/// Register the variations you want to use with [`register_roboto()`], [`register_roboto_bold()`],
/// and [`register_roboto_italic()`] first. Use the font weight and font style properties to select
/// a specific variation.
pub const ROBOTO: &str = "Roboto";
/// The font name for the icon font (tabler-icons), needs to be registered using
/// [`register_tabler_icons()`] first.
pub const TABLER_ICONS: &str = "tabler-icons";

pub fn register_roboto(cx: &mut Context) {
    cx.add_font_mem(fonts::ROBOTO_REGULAR);
}
pub fn register_roboto_bold(cx: &mut Context) {
    cx.add_font_mem(fonts::ROBOTO_BOLD);
}
pub fn register_roboto_italic(cx: &mut Context) {
    cx.add_font_mem(fonts::ROBOTO_ITALIC);
}
pub fn register_tabler_icons(cx: &mut Context) {
    cx.add_font_mem(fonts::TABLER_ICONS);
}
