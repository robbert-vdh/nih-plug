//! A standalone plugin target that directly connects to the system's audio and MIDI ports instead
//! of relying on a plugin host. This is mostly useful for quickly testing GUI changes.

use clap::{FromArgMatches, IntoApp};

use self::wrapper::{Wrapper, WrapperConfig, WrapperError};
use super::util::setup_logger;
use crate::plugin::Plugin;

mod backend;
mod context;
mod wrapper;

/// Open an NIH-plug plugin as a standalone application. If the plugin has an editor, this will open
/// the editor and block until the editor is closed. Otherwise this will block until SIGINT is
/// received. This is mainly useful for quickly testing plugin GUIs. In order to use this, you will
/// first need to make your plugin's main struct `pub` and expose a `lib` artifact in addition to
/// your plugin's `cdylib`:
///
/// ```toml
/// # Cargo.toml
///
/// [lib]
/// # The `lib` artifact is needed for the standalone target
/// crate-type = ["cdylib", "lib"]
/// ```
///
/// You can then create a `src/main.rs` file that calls this function:
///
/// ```ignore
/// // src/main.rs
///
/// use plugin_name::prelude::*;
///
/// use plugin_name::PluginName;
///
/// fn main() {
///     nih_export_standalone::<PluginName>();
/// }
/// ```
///
/// By default this will connect to the 'default' audio and MIDI ports. Use the command line options
/// to change this. `--help` lists all available options.
///
/// If the wrapped plugin fails to initialize or throws an error during audio processing, then this
/// function will return `false`.
pub fn nih_export_standalone<P: Plugin>() -> bool {
    nih_export_standalone_with_args::<P, _>(std::env::args())
}

/// The same as [`nih_export_standalone()`], but with the arguments taken from an iterator instead
/// of using [`std::env::args()`].
pub fn nih_export_standalone_with_args<P: Plugin, Args: IntoIterator<Item = String>>(
    args: Args,
) -> bool {
    setup_logger();

    // Instead of parsing this directly, we need to take a bit of a roundabout approach to get the
    // plugin's name and vendor in here since they'd otherwise be taken from NIH-plug's own
    // `Cargo.toml` file.
    let config = WrapperConfig::from_arg_matches(
        &WrapperConfig::command()
            .name(P::NAME)
            .author(P::VENDOR)
            .get_matches_from(args),
    )
    .unwrap_or_else(|err| err.exit());

    nih_log!(
        "Audio and MIDI IO has not yet been implemented in the standalone targets. So if you're \
         not hearing anything, then that's correct!"
    );

    // TODO: We should try JACK first, then CPAL, and then fall back to the dummy backend. With a
    //       command line option to override this behavior.
    let backend = backend::Dummy::new(config.clone());
    let wrapper = match Wrapper::<P, _>::new(backend, config.clone()) {
        Ok(wrapper) => wrapper,
        Err(err) => {
            print_error(err, &config);
            return false;
        }
    };

    // TODO: Add a repl while the application is running to interact with parameters
    match wrapper.run() {
        Ok(()) => true,
        Err(err) => {
            print_error(err, &config);
            false
        }
    }
}

fn print_error(error: WrapperError, config: &WrapperConfig) {
    match error {
        WrapperError::IncompatibleConfig => {
            eprintln!("The plugin does not support the {} channel input and {} channel output configuration", config.input_channels, config.output_channels);
        }
        WrapperError::InitializationFailed => {
            eprintln!("The plugin failed to initialize");
        }
    }
}
