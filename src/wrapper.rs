//! Wrappers for different plugin types. Each wrapper has an entry point macro that you can pass the
//! name of a type that implements `Plugin` to. The macro will handle the rest.

pub(crate) mod state;
pub(crate) mod util;
pub mod vst3;
