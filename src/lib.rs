// TODO: Once everything is more fleshed out, document the basic usage of this library and
//       restructure these re-exports into a more useful prelude

#[macro_use]
pub mod debug;

pub mod formatters;
pub mod util;

// Re-export our derive macros to make this a bit easier to use
pub use nih_plug_derive::Params;

// And also re-export anything you'd need to build a plugin
pub use buffer::Buffer;
pub use context::{GuiContext, ParamSetter, ProcessContext};
pub use param::internals::Params;
pub use param::range::Range;
pub use param::smoothing::{Smoother, SmoothingStyle};
pub use param::{BoolParam, FloatParam, IntParam, Param};
// TODO: Consider re-exporting these from another module so you can import them all at once
pub use param::{Display, EnumIter, EnumParam};
pub use plugin::{
    BufferConfig, BusConfig, Editor, NoteEvent, ParentWindowHandle, Plugin, ProcessStatus,
    Vst3Plugin,
};

// The rest is either internal or already re-exported
mod buffer;
mod context;
pub mod param;
pub mod plugin;
pub mod wrapper;
