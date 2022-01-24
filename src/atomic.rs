// nih-plugs: plugins, but rewritten in Rust
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

use atomic_float::AtomicF32;
use std::sync::atomic::AtomicI32;

// Type families galore!
pub trait AtomicType {
    /// An atomic version of this type with interior mutability.
    type AtomicType;

    fn new_atomic(self) -> Self::AtomicType;
}

impl AtomicType for f32 {
    type AtomicType = AtomicF32;

    fn new_atomic(self) -> AtomicF32 {
        AtomicF32::new(self)
    }
}

impl AtomicType for i32 {
    type AtomicType = AtomicI32;

    fn new_atomic(self) -> AtomicI32 {
        AtomicI32::new(self)
    }
}
