//! Convenience functions for formatting and parsing parameter values in common formats.

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
            .trim_end_matches(&[' ', 'd', 'B', 'F', 'S'])
            .parse()
            .ok()
            .map(util::db_to_gain)
    })
}

/// Turn an `f32` value `[-1, 1]` to a panning value `[100L, 100R]` Value of `0.0` becomes `"C"`
pub fn f32_panning() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| {
        if x == 0. {
            "C".to_string()
        } else if x > 0. {
            format!("{:.0}R", x * 100.)
        } else {
            format!("{:.0}L", x * -100.)
        }
    })
}
/// Parse a pan value to a linear value, range `[-1, 1]`. Used in
/// conjunction with [`f32_panning`].
pub fn from_f32_panning() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        if string.contains('L') {
            string
                .trim_end_matches(&[' ', 'L'])
                .parse()
                .ok()
                .map(|x: f32| x / -100.)
        } else if string.contains('R') {
            string
                .trim_end_matches(&[' ', 'R'])
                .parse()
                .ok()
                .map(|x: f32| x / 100.)
        } else if string == "C" {
            Some(0.)
        } else {
            string.trim_end_matches(&[' ']).parse().ok().map(|x: f32| x)
        }
    })
}

/// Round an `f32` value and divide it by 1000 when it gets over 1000
pub fn f32_hz_then_khz(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| {
        if x < 1000.0 {
            format!("{:.digits$}", x)
        } else {
            let digits = digits + 1;
            format!("{:.digits$}", x / 1000.0)
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
