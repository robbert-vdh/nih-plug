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

#[macro_use]
pub mod debug;

pub mod formatters;
pub mod util;

// Re-export our derive macros to make this a bit easier to use
pub use nih_plug_derive::Params;

// And also re-export anything you'd need to build a plugin
pub use buffer::Buffer;
pub use context::ProcessContext;
pub use param::internals::Params;
pub use param::range::Range;
pub use param::smoothing::{Smoother, SmoothingStyle};
pub use param::{BoolParam, FloatParam, IntParam, Param};
pub use plugin::{
    BufferConfig, BusConfig, Editor, NoEditor, NoteEvent, Plugin, ProcessStatus, Vst3Plugin,
};

// The rest is either internal or already re-exported
mod buffer;
mod context;
pub mod param;
pub mod plugin;
pub mod wrapper;
