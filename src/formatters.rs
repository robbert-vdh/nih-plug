//! Convenience functions for formatting and parsing parameter values in common formats.

use std::sync::Arc;

/// Round an `f32` value to always have a specific number of decimal digits.
pub fn f32_rounded(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| format!("{:.digits$}", x))
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

/// Turn an `f32` value from linear to dBFS (reference value 1)
pub fn f32_lin_to_db(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| format!("{:.digits$}", x.log10() * 20.0))
}

/// Turn an `f32` value from dBFS (reference value 1) to linear
pub fn f32_db_to_lin(digits: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |x| format!("{:.digits$}", 10f32.powf(x / 20.0)))
}
/// Parse a `dBFS` value to a linear value. Handles the dB unit for you. Used in
/// conjunction with [`f32_lin_to_db`].
pub fn from_f32_lin_to_db() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|string| {
        string
            .trim_end_matches(&[' ', 'd', 'B'])
            .parse()
            .ok()
            .map(|x: f32| 10f32.powf(x / 20.0))
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
