//! Convenience functions for formatting and parsing parameter values in various common formats.
//!
//! Functions prefixed with `v2s_` are meant to be used with the `.value_to_string()` parameter
//! functions, while the `s2v_` functions are meant to be used wit the `.string_to_value()`.
//! functions. Most of these formatters come as a pair. Check each formatter's documentation for any
//! additional usage information.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::util;

// TODO: The v2s and s2v naming convention isn't ideal, but at least it's unambiguous. Is there a
//       better way to name these functions? Should we just split this up into two modules?

/// Round an `f32` value to always have a specific number of decimal digits.
pub fn v2s_f32_rounded(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| format!("{:.digits$}", value))
}

/// Format a `[0, 1]` number as a percentage. Does not include the percent sign, you should specify
/// this as the parameter's unit.
pub fn v2s_f32_percentage(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| format!("{:.digits$}", value * 100.0))
}

/// Parse a `[0, 100]` percentage to a `[0, 1]` number. Handles the percentage unit for you. Used in
/// conjunction with [`v2s_f32_percentage()`].
pub fn s2v_f32_percentage() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        string
            .trim_end_matches(&[' ', '%'])
            .parse()
            .ok()
            .map(|x: f32| x / 100.0)
    })
}

/// Format a positive number as a compression ratio. A value of 4 will be formatted as `4.0:1` while
/// 0.25 is formatted as `1:4.0`.
pub fn v2s_compression_ratio(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| {
        if value >= 1.0 {
            format!("{:.digits$}:1", value)
        } else {
            format!("1:{:.digits$}", value.recip())
        }
    })
}

/// Parse a `x:y` compression ratio back to a floating point number. Used in conjunction with
/// [`v2s_compression_ratio()`]. Plain numbers are parsed directly for UX's sake.
pub fn s2v_compression_ratio() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim();
        string
            .trim()
            .split_once(':')
            .and_then(|(numerator, denominator)| {
                let numerator: f32 = numerator.trim().parse().ok()?;
                let denominator: f32 = denominator.trim().parse().ok()?;

                Some(numerator / denominator)
            })
            // Just parse the value directly if it doesn't contain a colon
            .or_else(|| string.parse().ok())
    })
}

/// Turn an `f32` value from voltage gain to decibels using the semantics described in
/// [`util::gain_to_db()]. You should use either `" dB"` or `" dBFS"` for the parameter's unit.
/// `0.0` will be formatted as `-inf`.
pub fn v2s_f32_gain_to_db(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| {
        if value < util::MINUS_INFINITY_GAIN {
            String::from("-inf")
        } else {
            // Never print -0.0 since that just looks weird and confusing
            let value_db = util::gain_to_db(value);
            let value_db = if value_db.abs() < 1e-6 { 0.0 } else { value_db };

            format!("{:.digits$}", value_db)
        }
    })
}

/// Parse a decibel value to a linear voltage gain ratio. Handles the `dB` or `dBFS` units for you.
/// Used in conjunction with [`v2s_f32_gain_to_db()`]. `-inf dB` will be parsed to 0.0.
pub fn s2v_f32_gain_to_db() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim_end_matches(&[' ', 'd', 'D', 'b', 'B', 'f', 'F', 's', 'S']);
        if string.eq_ignore_ascii_case("-inf") {
            Some(0.0)
        } else {
            string.parse().ok().map(util::db_to_gain)
        }
    })
}

/// Turn an `f32` `[-1, 1]` value to a panning value where negative values are represented by
/// `[100L, 1L]`, 0 gets turned into `C`, and positive values become `[1R, 100R]` values.
pub fn v2s_f32_panning() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| match value.partial_cmp(&0.0) {
        Some(Ordering::Less) => format!("{:.0}L", value * -100.0),
        Some(Ordering::Equal) => String::from("C"),
        Some(Ordering::Greater) => format!("{:.0}R", value * 100.0),
        None => String::from("NaN"),
    })
}

/// Parse a pan value in the format of [`v2s_f32_panning()] to a linear value in the range `[-1,
/// 1]`.
pub fn s2v_f32_panning() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim();
        let cleaned_string = string.trim_end_matches(&[' ', 'l', 'L']).parse().ok();
        match string.chars().last()?.to_uppercase().next()? {
            'L' => cleaned_string.map(|x: f32| x / -100.0),
            'C' => Some(0.0),
            'R' => cleaned_string.map(|x: f32| x / 100.0),
            _ => None,
        }
    })
}

/// Format a `f32` Hertz value as a rounded `Hz` below 1000 Hz, and as a rounded `kHz` value above
/// 1000 Hz. This already includes the unit.
pub fn v2s_f32_hz_then_khz(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| {
        if value < 1000.0 {
            format!("{:.digits$} Hz", value)
        } else {
            format!("{:.digits$} kHz", value / 1000.0, digits = digits.max(1))
        }
    })
}

/// Convert an input in the same format at that of [`v2s_f32_hz_then_khz()] to a Hertz value. This
/// additionally also accepts note names in the same format as [`s2v_i32_note_formatter()`], and
/// optionally also with cents in the form of `D#5, -23 ct.`.
pub fn s2v_f32_hz_then_khz() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    // FIXME: This is a very crude way to reuse the note value formatter. There's no real runtime
    //        penalty for doing it this way, but it does look less pretty.
    let note_formatter = s2v_i32_note_formatter();

    Arc::new(move |string| {
        let string = string.trim();

        // If the user inputs a note representation, then we'll use that
        if let Some((midi_note_number_str, cents_str)) = string.split_once(',') {
            // If it contains a comma we'll also try parsing cents
            let cents_str = cents_str
                .trim_start_matches([' ', '+', 'c', 'e', 'n', 't', 's', '.'])
                .trim_end();

            if let (Some(midi_note_number), Ok(cents)) = (
                note_formatter(midi_note_number_str),
                cents_str.parse::<i32>(),
            ) {
                let plain_note_freq = util::f32_midi_note_to_freq(midi_note_number as f32);
                let cents_multiplier = 2.0f32.powf(cents as f32 / 100.0);
                return Some(plain_note_freq * cents_multiplier);
            }
        } else if let Some(midi_note_number) = note_formatter(string) {
            return Some(util::f32_midi_note_to_freq(midi_note_number as f32));
        }

        // Otherwise we'll accept values in either Hz (with or without unit) or kHz
        let cleaned_string = string
            .trim_end_matches([' ', 'k', 'K', 'h', 'H', 'z', 'Z'])
            .parse()
            .ok();
        match string.get(string.len().saturating_sub(3)..) {
            Some(unit) if unit.eq_ignore_ascii_case("khz") => cleaned_string.map(|x| x * 1000.0),
            // Even if there's no unit at all, just assume the input is in Hertz
            _ => cleaned_string,
        }
    })
}

/// Format an order/power of two. Useful in conjunction with [`s2v_i32_power_of_two()`] to limit
/// integer parameter ranges to be only powers of two.
pub fn v2s_i32_power_of_two() -> Arc<dyn Fn(i32) -> String + Send + Sync> {
    Arc::new(|value| format!("{}", 1 << value))
}

/// Parse a parameter input string to a power of two. Useful in conjunction with
/// [`v2s_i32_power_of_two()`] to limit integer parameter ranges to be only powers of two.
pub fn s2v_i32_power_of_two() -> Arc<dyn Fn(&str) -> Option<i32> + Send + Sync> {
    Arc::new(|string| string.parse().ok().map(|n: i32| (n as f32).log2() as i32))
}

/// Turns an integer MIDI note number (usually in the range [0, 127]) into a note name, where 60 is
/// C4 and 69 is A4 (nice).
pub fn v2s_i32_note_formatter() -> Arc<dyn Fn(i32) -> String + Send + Sync> {
    Arc::new(move |value| {
        let note_name = util::NOTES[value.rem_euclid(12) as usize];
        let octave = (value / 12) - 1;
        format!("{note_name}{octave}")
    })
}

/// Parse a note name to a MIDI number using the inverse mapping from [`v2s_i32_note_formatter()].
pub fn s2v_i32_note_formatter() -> Arc<dyn Fn(&str) -> Option<i32> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim();
        if string.len() < 2 {
            return None;
        }

        // A valid trimmed string will either be be two characters (we already checked the length),
        // or two characters separated by spaces
        let (note_name, octave) = string
            .split_once(|c: char| c.is_whitespace())
            .unwrap_or_else(|| (&string[..1], &string[1..]));

        let note_id = util::NOTES
            .iter()
            .position(|&candidate| note_name.eq_ignore_ascii_case(candidate))?
            as i32;
        let octave: i32 = octave.trim().parse().ok()?;

        // 0 = C-1, 12 = C0, 24 = C1
        Some(note_id + (12 * (octave + 1)))
    })
}

/// Display 'Bypassed' or 'Not Bypassed' depending on whether the parameter is true or false.
/// 'Enabled' would have also been a possibility here, but that could be a bit confusing.
pub fn v2s_bool_bypass() -> Arc<dyn Fn(bool) -> String + Send + Sync> {
    Arc::new(move |value| {
        if value {
            String::from("Bypassed")
        } else {
            String::from("Not Bypassed")
        }
    })
}

/// Parse a string in the same format as [`v2s_bool_bypass()].
pub fn s2v_bool_bypass() -> Arc<dyn Fn(&str) -> Option<bool> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim();
        if string.eq_ignore_ascii_case("bypassed") {
            Some(true)
        } else if string.eq_ignore_ascii_case("not bypassed") {
            Some(false)
        } else {
            None
        }
    })
}
