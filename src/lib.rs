//! Documentation is currently a work in progress. Import everything from the [`prelude`] module and
//! check out the example plugins to get started.

#![cfg_attr(feature = "docs", feature(doc_auto_cfg))]
#![cfg_attr(feature = "simd", feature(portable_simd))]

#[macro_use]
mod debug;

/// A re-export of the `log` crate for use in the debug macros. This should not be used directly.
pub use log;

/// Everything you'll need to use NIH-plug. Import this with `use nih_plug::prelude::*;`.
pub mod prelude;

// These modules have also been re-exported in the prelude.
pub mod formatters;
pub mod util;

pub mod buffer;
pub mod context;
mod event_loop;
pub mod midi;
pub mod param;
pub mod plugin;
pub mod wrapper;
