// Soft Vacuum: Airwindows Hard Vacuum port with oversampling
// Copyright (C) 2023 Robbert van der Helm
// Copyright (c) 2018 Chris Johnson
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

/// Single-channel port of the Hard Vacuum algorithm from
/// <https://github.com/airwindows/airwindows/blob/283343b9e90c28fdb583f27e198f882f268b051b/plugins/LinuxVST/src/HardVacuum/HardVacuumProc.cpp>.
#[derive(Debug, Default)]
pub struct HardVacuum {
    last_sample: f32,
}

impl HardVacuum {
    /// Reset the processor's state. In this case this only resets the discrete derivative
    /// calculation. Doesn't make a huge difference but it's still useful to make the effect
    /// deterministic.
    pub fn reset(&mut self) {
        self.last_sample = 0.0;
    }
}
