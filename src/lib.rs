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

// TODO: Once everything is more fleshed out, document the basic usage of this library

pub mod buffer;
pub mod context;
#[macro_use]
pub mod debug;
pub mod formatters;
pub mod param;
pub mod plugin;
pub mod util;
pub mod wrapper;

// Re-export our derive macros to make this a bit easier to use
pub use nih_plug_derive::Params;
