//! General conversion functions and utilities.

mod stft;
pub mod window;

pub use stft::StftHelper;

pub const MINUS_INFINITY_DB: f32 = -100.0;
pub const MINUS_INFINITY_GAIN: f32 = 1e-5; // 10f32.powf(MINUS_INFINITY_DB / 20)
pub const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Temporarily allow allocations within `func` if NIH-plug was configured with the
/// `assert_process_allocs` feature.
#[cfg(all(debug_assertions, feature = "assert_process_allocs"))]
pub fn permit_alloc<T, F: FnOnce() -> T>(func: F) -> T {
    assert_no_alloc::permit_alloc(func)
}

/// Temporarily allow allocations within `func` if NIH-plug was configured with the
/// `assert_process_allocs` feature.
#[cfg(not(all(debug_assertions, feature = "assert_process_allocs")))]
pub fn permit_alloc<T, F: FnOnce() -> T>(func: F) -> T {
    func()
}

/// Convert decibels to a voltage gain ratio, treating anything below -100 dB as minus infinity.
pub fn db_to_gain(dbs: f32) -> f32 {
    if dbs > MINUS_INFINITY_DB {
        10.0f32.powf(dbs * 0.05)
    } else {
        0.0
    }
}

/// Convert a voltage gain ratio to decibels. Gain ratios that aren't positive will be treated as
/// [`MINUS_INFINITY_DB`].
pub fn gain_to_db(gain: f32) -> f32 {
    if gain > MINUS_INFINITY_GAIN {
        gain.log10() * 20.0
    } else {
        MINUS_INFINITY_DB
    }
}

/// Convert a MIDI note ID to a frequency at A4 = 440 Hz equal temperament and middle C = note 60 =
/// C4.
pub fn midi_note_to_freq(note: u8) -> f32 {
    2.0f32.powf((note as f32 - 69.0) / 12.0) * 440.0
}

/// Return the rate of exponential decay necessary over `num_ticks` to
/// decrease from `start` to `target`.
pub fn exponential_decay_rate(num_ticks: f32, start: f32, target: f32) -> f32 {
    // Exponential growth function is
    // y = a(1 + r)^t
    // Transform this to get t...
    // r = (y/a)^(1/t) - 1
    (target / start).powf(1.0 / num_ticks) - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_to_gain_positive() {
        assert_eq!(db_to_gain(3.0), 1.4125376);
    }

    #[test]
    fn test_db_to_gain_negative() {
        assert_eq!(db_to_gain(-3.0), 1.4125376f32.recip());
    }

    #[test]
    fn test_db_to_gain_minus_infinity() {
        assert_eq!(db_to_gain(-100.0), 0.0);
    }

    #[test]
    fn test_gain_to_db_positive() {
        assert_eq!(gain_to_db(4.0), 12.041201);
    }

    #[test]
    fn test_gain_to_db_negative() {
        assert_eq!(gain_to_db(0.25), -12.041201);
    }

    #[test]
    fn test_gain_to_db_minus_infinity_zero() {
        assert_eq!(gain_to_db(0.0), MINUS_INFINITY_DB);
    }

    #[test]
    fn test_gain_to_db_minus_infinity_negative() {
        assert_eq!(gain_to_db(-2.0), MINUS_INFINITY_DB);
    }

    #[test]
    fn test_exponential_decay_rate_single_step_halflife() {
        assert_eq!(exponential_decay_rate(1.0, 1.0, 0.5), -0.5);
    }
}
