// nih-plug: plugins, but rewritten in Rust
// Copyright (C) 2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

pub const MINUS_INFINITY_DB: f32 = -100.0;

/// Convert decibels to gain, treating anything below -100 dB as minus infinity.
pub fn db_to_gain(dbs: f32) -> f32 {
    if dbs > MINUS_INFINITY_DB {
        10.0f32.powf(dbs * 0.05)
    } else {
        0.0
    }
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
}
