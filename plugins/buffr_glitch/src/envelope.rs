// Buffr Glitch: a MIDI-controlled buffer repeater
// Copyright (C) 2022-2023 Robbert van der Helm
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

/// The most barebones envelope generator you can imagine using a bog standard first order IIR
/// filter. We don't need anything fancy right now.
#[derive(Debug, Default)]
pub struct AREnvelope {
    /// The internal filter state.
    state: f32,

    /// For each sample, the output becomes `(state * t) + (target * (1.0 - t))`. This is `t` during
    /// the attack portion of the envelope generator.
    attack_retain_t: f32,
    /// `attack_retain_t`, but for the release portion.
    release_retain_t: f32,

    /// The value the envelope follower should try to achieve when not in the release stage.
    target_value: f32,
    /// Whether the envelope follower is currently in its release stage.
    releasing: bool,
}

impl AREnvelope {
    pub fn set_attack_time(&mut self, sample_rate: f32, time_ms: f32) {
        self.attack_retain_t = (-1.0 / (time_ms / 1000.0 * sample_rate)).exp();
    }

    pub fn set_release_time(&mut self, sample_rate: f32, time_ms: f32) {
        self.release_retain_t = (-1.0 / (time_ms / 1000.0 * sample_rate)).exp();
    }

    /// Completely reset the envelope follower.
    pub fn reset(&mut self) {
        self.state = 0.0;
        self.releasing = false;
    }

    /// Only reset the release state, but don't reset the internal filter state.
    pub fn soft_reset(&mut self) {
        self.releasing = false;
    }

    /// Set the maximum value the envelope follower should achieve.
    pub fn set_target(&mut self, target: f32) {
        self.target_value = target;
    }

    /// Get the next value from the envelope follower.
    pub fn next(&mut self) -> f32 {
        let (target, t) = if self.releasing {
            (0.0, self.release_retain_t)
        } else {
            (self.target_value, self.attack_retain_t)
        };

        let new = (self.state * t) + (target * (1.0 - t));
        self.state = new;

        new
    }

    /// Start the release segment of the envelope generator.
    pub fn start_release(&mut self) {
        self.releasing = true;
    }

    /// Whether the envelope generator is still in its release stage and the value hasn't dropped
    /// down to 0.0 yet.
    pub fn is_releasing(&self) -> bool {
        self.releasing && self.state >= 0.001
    }
}
