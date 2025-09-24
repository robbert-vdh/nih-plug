//! Documentation is still a work in progress. The best way to learn right now is to browse through
//! the examples and to browse through these docs. There is no full guide yet, but here are some
//! pointers to get started:
//!
//! - All useful functionality is exported through the [`prelude`] module. Add
//!   `use nih_plug::prelude::*;` to the top of your `lib.rs` file to get started.
//! - Make sure to check out the macros from the [`debug`] module. These should be used instead of,
//!   `println!()`/`eprint!()`, `dbg!()` and similar macros, and they are re-exported from the
//!   prelude. NIH-plug sets up a flexible logger for you that all of these functions will output
//!   to. By default, the output is logged to STDERR unless you're running Windows and a Windows
//!   debugger is attached, in which case the output is logged to the debug console instead. The
//!   `NIH_LOG` environment variable controls whether output is logged to STDERR, the Windows debug
//!   console, or to a file. Check the [`nih_log!()`] macro for more information.
//! - The aforementioned debug module also contains non-fatal debug-assertions macros that are only
//!   evaluated during debug builds. The framework uses these all over the place to check for
//!   invariants, so it's important to test your plugins using debug builds while developing.
//! - Check out the features list in NIH-plug's `Cargo.toml` file for optional features you can
//!   enable. This includes things like SIMD support for the buffer adapters and panicking on
//!   allocations during DSP code in debug mode.
//!
//! - An NIH-plug plugin consists of an implementation of the [`Plugin`][prelude::Plugin] trait and
//!   a call to [`nih_export_vst3!()`] and/or [`nih_export_clap!()`] in your `lib.rs` file to expose
//!   the plugin functionality. Some of these traits will require you to implement an additional
//!   trait containing API-specific information for the plugin.
//!
//!   Check the `Plugin` trait's documentation for more information on NIH-plug's general structure
//!   and approach with respect to declarativity.
//! - NIH-plug comes with a bundler that creates plugin bundles for you based on the exported plugin
//!   formats and the operating system and architecture you're compiling for. Check out the
//!   readme for
//!   [`nih_plug_xtask`](https://github.com/robbert-vdh/nih-plug/tree/master/nih_plug_xtask) for
//!   instructions on how to use this within your own project.
//! - It's also possible to export a standalone application from a plugin using the
//!   [`nih_export_standalone()`] function. Check that function's documentation to learn how to do
//!   this. This requires enabling the `standalone` crate feature.
//! - Everything is described in more detail on the [`Plugin`][prelude::Plugin] trait and everything
//!   linked from there, but a plugin's general lifecycle involves the following function calls.
//!
//!   1. When the host loads the plugin, your plugin object will be instantiated using its
//!      [`Default`] implementation. The plugin should refrain from performing expensive
//!      calculations or IO at this point.
//!   2. The host will select an audio IO layout from
//!      [`Plugin::AUDIO_IO_LAYOUTS`][prelude::Plugin::AUDIO_IO_LAYOUTS]. The first layout is always
//!      used as the default one, and should reflect the plugin's most commonly used configuration.
//!      Usually this is a stereo layout.
//!   3. After that, [`Plugin::initialize()`][prelude::Plugin::initialize()] will be called with the
//!      the selected IO configuration and the audio buffer settings. Here you should allocate any
//!      data structures you need or precompute data that depends on the sample rate or maximum
//!      buffer size. This is the only place where you can safely allocate memory.
//!   4. The [`Plugin::reset()`][prelude::Plugin::reset()] function is always called immediately
//!      after `initialize()`. This is where you should clear out coefficients, envelopes, phases,
//!      and other runtime data. The reason for this split is that this function may be called at
//!      any time by the host from the audio thread, and it thus needs to be realtime-safe.
//!
//!      Whenever a preset is loaded, both of these functions will be called again.
//!   5. After that the [`Plugin::process()`][prelude::Plugin::process()] function will be called
//!      repeatedly until the plugin is deactivated. Here the plugin receives a
//!      [`Buffer`][prelude::Buffer] object that contains the input audio (if the plugin has inputs)
//!      which the plugin should overwrite with output audio. Check the documentation on the
//!      `Buffer` object for all of the ways you can use this API. You can access note events,
//!      transport data, and more through the [`ProcessContext`][prelude::ProcessContext] that's
//!      also passed to the process function.
//!   6. [`Plugin::deactivate()`][prelude::Plugin::deactivate()] is called from the when the plugin
//!      gets deactivated. You probably don't need to do anything here, but you could deallocate or
//!      clean up resources here.
//!
//!  - Plugin parameters are managed automatically by creating a struct deriving the
//!    [`Params`][prelude::Params] trait and returning a handle to it from the
//!    [`Plugin::params()`][prelude::Plugin::params()] function. Any
//!    [`FloatParam`][prelude::FloatParam], [`IntParam`][prelude::IntParam],
//!    [`BoolParam`][prelude::BoolParam] or [`EnumParam`][prelude::EnumParam] fields on that struct
//!    will automatically be registered as a parameter if they have an `#[id = "foobar"]` attribute.
//!    The string `"foobar"` here uniquely identifies the parameter, making it possible to reorder
//!    and rename parameters as long as this string stays constant. You can also store persistent
//!    non-parameter data and other parameter objects in a `Params` struct. Check out the trait's
//!    documentation for details on all supported features, and also be sure to take a look at the
//!    [example plugins](https://github.com/robbert-vdh/nih-plug/tree/master/plugins).
//!  - After calling `.with_smoother()` during an integer or floating point parameter's creation,
//!    you can use `param.smoothed` to access smoothed values for that parameter. Be sure to check
//!    out the [`Smoother`][prelude::Smoother] API for more details.
//!
//! There's a whole lot more to discuss, but once you understand the above you should be able to
//! figure out the rest by reading through the examples and the API documentation. Good luck!

#![cfg_attr(feature = "docs", feature(doc_auto_cfg))]
#![cfg_attr(feature = "simd", feature(portable_simd))]
// Around Rust 1.64 Clippy started throwing this for all instances of `dyn Fn(...) -> ... + Send +
// Sync`. Creating type aliases for all of these callbacks probably won't make things easier to read.
#![allow(clippy::type_complexity)]

// These macros are also in the crate root and in the prelude, but having the module itself be pub
// as well makes it easy to import _just_ the macros without using `#[macro_use] extern crate nih_plug;`
#[macro_use]
pub mod debug;

/// A re-export of the `log` crate for use in the debug macros. This should not be used directly.
pub use log;

/// Everything you'll need to use NIH-plug. Import this with `use nih_plug::prelude::*;`.
pub mod prelude;

// These modules are also re-exported in the prelude
pub mod formatters;
pub mod util;

pub mod audio_setup;
pub mod buffer;
pub mod context;
pub mod editor;
mod event_loop;
pub mod midi;
pub mod params;
pub mod plugin;
pub mod wrapper;

// This is also re-exported from the prelude but since the other export entry points are macros and
// macros are always accessible from the crate's root, it seems like a good idea to keep the
// symmetry and also export this function in the same places
#[cfg(feature = "standalone")]
pub use wrapper::standalone::nih_export_standalone;
