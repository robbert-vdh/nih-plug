//! Binary assets for use with `nih_plug_iced`.

use crate::Font;

// This module provides a re-export and simple font wrappers around the re-exported fonts.
pub use nih_plug_assets::*;

pub const NOTO_SANS_REGULAR: Font = Font::External {
    name: "Noto Sans Regular",
    bytes: fonts::NOTO_SANS_REGULAR,
};

pub const NOTO_SANS_REGULAR_ITALIC: Font = Font::External {
    name: "Noto Sans Regular Italic",
    bytes: fonts::NOTO_SANS_REGULAR_ITALIC,
};

pub const NOTO_SANS_THIN: Font = Font::External {
    name: "Noto Sans Thin",
    bytes: fonts::NOTO_SANS_THIN,
};

pub const NOTO_SANS_THIN_ITALIC: Font = Font::External {
    name: "Noto Sans Thin Italic",
    bytes: fonts::NOTO_SANS_THIN_ITALIC,
};

pub const NOTO_SANS_LIGHT: Font = Font::External {
    name: "Noto Sans Light",
    bytes: fonts::NOTO_SANS_LIGHT,
};

pub const NOTO_SANS_LIGHT_ITALIC: Font = Font::External {
    name: "Noto Sans Light Italic",
    bytes: fonts::NOTO_SANS_LIGHT_ITALIC,
};

pub const NOTO_SANS_BOLD: Font = Font::External {
    name: "Noto Sans Bold",
    bytes: fonts::NOTO_SANS_BOLD,
};

pub const NOTO_SANS_BOLD_ITALIC: Font = Font::External {
    name: "Noto Sans Bold Italic",
    bytes: fonts::NOTO_SANS_BOLD_ITALIC,
};
