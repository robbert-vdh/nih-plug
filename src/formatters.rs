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

/// Round an `f32` value to always have a specific number of decimal digits. Avoids returning
/// negative zero values to make sure string->value->string roundtrips work correctly. Otherwise
/// `-0.001` rounded to two digits would result in `-0.00`.
pub fn v2s_f32_rounded(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    let rounding_multiplier = 10u32.pow(digits as u32) as f32;
    Arc::new(move |value| {
        // See above
        if (value * rounding_multiplier).round() / rounding_multiplier == 0.0 {
            format!("{:.digits$}", 0.0)
        } else {
            format!("{value:.digits$}")
        }
    })
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
            format!("{value:.digits$}:1")
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
/// [`util::gain_to_db()`]. You should use either `" dB"` or `" dBFS"` for the parameter's unit.
/// `0.0` will be formatted as `-inf`. Avoids returning negative zero values to make sure
/// string->value->string roundtrips work correctly. Otherwise `-0.001` rounded to two digits
/// would result in `-0.00`.
pub fn v2s_f32_gain_to_db(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    let rounding_multiplier = 10u32.pow(digits as u32) as f32;
    Arc::new(move |value| {
        if value < util::MINUS_INFINITY_GAIN {
            String::from("-inf")
        } else {
            let value_db = util::gain_to_db(value);

            // See above
            if (value_db * rounding_multiplier).round() / rounding_multiplier == 0.0 {
                format!("{:.digits$}", 0.0)
            } else {
                format!("{value_db:.digits$}")
            }
        }
    })
}

/// Parse a decibel value to a linear voltage gain ratio. Handles the `dB` or `dBFS` units for you.
/// Used in conjunction with [`v2s_f32_gain_to_db()`]. `-inf dB` will be parsed to 0.0.
pub fn s2v_f32_gain_to_db() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim_end_matches(&[' ', 'd', 'D', 'b', 'B', 'f', 'F', 's', 'S']);
        // NOTE: The above line strips the `f`, so checked for `-inf` here will always return false
        if string.eq_ignore_ascii_case("-in") {
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

/// Parse a pan value in the format of [`v2s_f32_panning()`] to a linear value in the range `[-1,
/// 1]`.
pub fn s2v_f32_panning() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim();
        let cleaned_string = string
            .trim_end_matches(&[' ', 'l', 'L', 'c', 'C', 'r', 'R'])
            .parse()
            .ok();
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
            format!("{value:.digits$} Hz")
        } else {
            format!("{:.digits$} kHz", value / 1000.0, digits = digits.max(1))
        }
    })
}

/// [`v2s_f32_hz_then_khz()`], but also includes the note name. Can be used with
/// [`s2v_f32_hz_then_khz()`].
pub fn v2s_f32_hz_then_khz_with_note_name(
    digits: usize,
    include_cents: bool,
) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value| {
        // With 0.0 this would result in a subtraction below i32's minimum value, and it would look
        // ridiculous anyways so we'll just not even bother for tiny values
        if value.abs() < 1.0 {
            return format!("{value:.digits$} Hz");
        }

        // This is the inverse of the formula in `f32_midi_note_to_freq`
        let fractional_note = util::freq_to_midi_note(value);
        let note = fractional_note.round();
        let cents = ((fractional_note - note) * 100.0).round() as i32;

        let note_name = util::NOTES[(note as i32).rem_euclid(12) as usize];
        // NOTE: This is different compared from `(note as i32 / 12) - 1` because truncating always
        //       rounds towards zero
        let octave = (note / 12.0).floor() as i32 - 1;
        let note_str = if cents == 0 || !include_cents {
            format!("{note_name}{octave}")
        } else {
            format!("{note_name}{octave}, {cents:+} ct.")
        };

        if value < 1000.0 {
            format!("{value:.digits$} Hz, {note_str}")
        } else {
            format!(
                "{:.digits$} kHz, {}",
                value / 1000.0,
                note_str,
                digits = digits.max(1)
            )
        }
    })
}

/// Convert an input in the same format at that of [`v2s_f32_hz_then_khz()`] to a Hertz value. This
/// additionally also accepts note names in the same format as [`s2v_i32_note_formatter()`], and
/// optionally also with cents in the form of `D#5, -23 ct.`.
pub fn s2v_f32_hz_then_khz() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    // FIXME: This is a very crude way to reuse the note value formatter. There's no real runtime
    //        penalty for doing it this way, but it does look less pretty.
    let note_formatter = s2v_i32_note_formatter();

    Arc::new(move |string| {
        let string = string.trim();

        // The input can contain a frequency in Hz or kHz, a note name, a note name and cents, or
        // one of those two combined with a frequency. In the last case we'll ignore the frequency.
        // If the string cannot be parsed as a note name, we'll try parsing it as a frequency
        // instead. This is needed for the formatting roundtrip to work correctly. The input will
        // consists of 1 to three segments, so we'll try to unpack them like this so we can pattern
        // match on them
        let mut segments = string.split(',');
        let segments = (segments.next(), segments.next(), segments.next());

        if let (_, Some(midi_note_number_str), Some(cents_str))
        | (Some(midi_note_number_str), Some(cents_str), None) = segments
        {
            let cents_str = cents_str
                .trim_start_matches([' ', '+'])
                .trim_end_matches([' ', 'C', 'c', 'E', 'e', 'N', 'n', 'T', 't', 'S', 's', '.']);

            if let (Some(midi_note_number), Ok(cents)) = (
                note_formatter(midi_note_number_str),
                cents_str.parse::<i32>(),
            ) {
                let plain_note_freq = util::f32_midi_note_to_freq(midi_note_number as f32);
                let cents_multiplier = 2.0f32.powf(cents as f32 / 100.0 / 12.0);
                return Some(plain_note_freq * cents_multiplier);
            }
        }

        if let (_, Some(midi_note_number_str), _) | (Some(midi_note_number_str), None, None) =
            segments
        {
            if let Some(midi_note_number) = note_formatter(midi_note_number_str) {
                return Some(util::f32_midi_note_to_freq(midi_note_number as f32));
            }
        }

        // Otherwise we'll accept values in either Hz (with or without unit) or kHz
        let frequency_segment = segments.0?;
        let cleaned_string = frequency_segment
            .trim_end_matches([' ', 'k', 'K', 'h', 'H', 'z', 'Z'])
            .parse()
            .ok();
        match frequency_segment.get(frequency_segment.len().saturating_sub(3)..) {
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

/// Parse a note name to a MIDI number using the inverse mapping from [`v2s_i32_note_formatter()`].
pub fn s2v_i32_note_formatter() -> Arc<dyn Fn(&str) -> Option<i32> + Send + Sync> {
    Arc::new(|string| {
        let string = string.trim();
        if string.len() < 2 {
            return None;
        }

        // A valid trimmed string will either be be at least two characters (we already checked the
        // length) or at least three characters if the second character is a hash, and there may be
        // spaces in between the note name and the octave number
        let (note_name, octave) = string
            .split_once(|c: char| c.is_whitespace())
            .unwrap_or_else(|| {
                // Sharps need to be handled separately
                if string.len() > 2 && &string[1..2] == "#" {
                    (&string[..2], &string[2..])
                } else {
                    (&string[..1], &string[1..])
                }
            });

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

/// Parse a string in the same format as [`v2s_bool_bypass()`].
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The rounding function should never return strings containing negative zero values.
    #[test]
    fn v2s_f32_rounded_negative_zero() {
        let v2s = v2s_f32_rounded(2);

        assert_eq!("0.00", v2s(-0.001));

        // Sanity check
        assert_eq!("-0.01", v2s(-0.009));
        assert_eq!("0.01", v2s(0.009));
    }

    // More of these validators could use tests, but this one in particular is tricky and I noticed
    // an issue where it didn't roundtrip correctly
    #[test]
    fn f32_hz_then_khz_with_note_name_roundtrip() {
        let v2s = v2s_f32_hz_then_khz_with_note_name(1, true);
        let s2v = s2v_f32_hz_then_khz();

        for freq in [0.0, 5.0, 7.18, 8.18, 69.420, 18181.8, 133333.7] {
            let string = v2s(freq);
            // We can't compare `freq` and `roundtrip_freq` because the string is rounded on both
            // cents and frequency and is thus lossy
            let roundtrip_freq = s2v(&string).unwrap();
            let roundtrip_string = v2s(roundtrip_freq);
            assert_eq!(
                string, roundtrip_string,
                "Unexpected: {string} -> {roundtrip_freq} -> {roundtrip_string}"
            );
        }
    }
}
