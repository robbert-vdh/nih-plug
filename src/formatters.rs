//! Convenience functions for formatting and parsing parameter values in common formats.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::util;

/// Round an `f32` value to always have a specific number of decimal digits.
pub fn f32_rounded(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| format!("{:.digits$}", value))
}

/// Format a `[0, 1]` number as a percentage. Does not include the percent sign, you should specify
/// this as the parameter's unit.
pub fn f32_percentage(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| format!("{:.digits$}", value * 100.0))
}

/// Parse a `[0, 100]` percentage to a `[0, 1]` number. Handles the percentage unit for you. Used in
/// conjunction with [`f32_percentage`].
pub fn from_f32_percentage() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        string
            .trim_end_matches(&[' ', '%'])
            .parse()
            .ok()
            .map(|x: f32| x / 100.0)
    })
}

/// Turn an `f32` value from voltage gain to decibels using the semantics described in
/// [`util::gain_to_db`]. You should use either `" dB"` or `" dBFS"` for the parameter's unit.
pub fn f32_gain_to_db(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| format!("{:.digits$}", util::gain_to_db(value)))
}

/// Parse a decibel value to a linear voltage gain ratio. Handles the `dB` or `dBFS` units for you.
/// Used in conjunction with [`f32_lin_to_db`].
pub fn from_f32_gain_to_db() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        string
            .trim_end_matches(&[' ', 'd', 'D', 'b', 'B', 'f', 'F', 's', 'S'])
            .parse()
            .ok()
            .map(util::db_to_gain)
    })
}

/// Turn an `f32` `[-1, 1]` value to a panning value where negative values are represented by
/// `[100L, 1L]`, 0 gets turned into `C`, and positive values become `[1R, 100R]` values.
pub fn f32_panning() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| match value.partial_cmp(&0.0) {
        Some(Ordering::Less) => format!("{:.0}L", value * -100.0),
        Some(Ordering::Equal) => String::from("C"),
        Some(Ordering::Greater) => format!("{:.0}R", value * 100.0),
        None => String::from("NaN"),
    })
}

/// Parse a pan value in the format of [`f32_panning`] to a linear value in the range `[-1, 1]`.
pub fn from_f32_panning() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim();
        let cleaned_string = string.trim_end_matches(&[' ', 'l', 'L']).parse().ok();
        match string.chars().last()?.to_uppercase().next()? {
            'L' => cleaned_string.map(|x: f32| x / -100.0),
            'R' => cleaned_string.map(|x: f32| x / 100.0),
            _ => Some(0.0),
        }
    })
}

/// Format a `f32` Hertz value as a rounded `Hz` below 1000 Hz, and as a rounded `kHz` value above
/// 1000 Hz. This already includes the unit.
pub fn f32_hz_then_khz(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| {
        if value < 1000.0 {
            format!("{:.digits$} Hz", value)
        } else {
            format!("{:.digits$} kHz", value / 1000.0, digits = digits.min(1))
        }
    })
}

/// Convert an input in the same format at that of [`f32_hz_then_khz`] to a Hertz value.
pub fn from_f32_hz_then_khz() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(move |string| {
        let string = string.trim();
        let cleaned_string = string
            .trim_end_matches(&[' ', 'k', 'K', 'h', 'H', 'z', 'Z'])
            .parse()
            .ok();
        match string.get(string.len() - 3..) {
            Some(unit) if unit.eq_ignore_ascii_case("khz") => cleaned_string.map(|x| x * 1000.0),
            // Even if there's no unit at all, just assume the input is in Hertz
            _ => cleaned_string,
        }
    })
}

/// Format an order/power of two. Useful in conjunction with [`from_power_of_two()`] to limit
/// integer parameter ranges to be only powers of two.
pub fn i32_power_of_two() -> Arc<dyn Fn(i32) -> String + Send + Sync> {
    Arc::new(|value| format!("{}", 1 << value))
}

/// Parse a parameter input string to a power of two. Useful in conjunction with [`power_of_two()`]
/// to limit integer parameter ranges to be only powers of two.
pub fn from_i32_power_of_two() -> Arc<dyn Fn(&str) -> Option<i32> + Send + Sync> {
    Arc::new(|string| string.parse().ok().map(|n: i32| (n as f32).log2() as i32))
}

const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// Turns an integer midi number (range 0-127 usually) into a note name, e.g. 69 -> A4
pub fn i32_note_formatter() -> Arc<dyn Fn(i32) -> String + Send + Sync> {
    Arc::new(move |x| {
        let note = x as usize;
        let note_name = NOTES[note % 12].to_string();
        let octave = (note / 12) as i32 - 1;
        format!("{note_name}{octave}")
    })
}
/// parses a note name into a midi number (range 0-127 usually), e.g. A#4 -> 70
pub fn from_i32_note_formatter() -> Arc<dyn Fn(&str) -> Option<i32> + Send + Sync> {
    Arc::new(|string| {
        // string is too short to be a note name
        if string.len() < 2 {
            return None;
        }
        let (note_name, octave) = if string.contains("#") {
            string.split_at(2)
        } else {
            string.split_at(1)
        };
        // using unwrap_or here, or else trying to parse "##" breaks it
        let note = NOTES.iter().position(|&r| r == note_name).unwrap_or(0) as i32;
        octave.parse().ok().map(|n: i32| (n + 1) * 12 + note)
    })
}
