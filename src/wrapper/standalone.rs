//! A standalone plugin target that directly connects to the system's audio and MIDI ports instead
//! of relying on a plugin host. This is mostly useful for quickly testing GUI changes.

use self::wrapper::Wrapper;
use crate::plugin::Plugin;

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
/// # TODOs
///
/// The aforementioned command line options have not yet been implemented. Currently there's also no
/// way to change these options at runtime, for instance through the plugin's GUI. And lastly
/// there's no way to interact with parameters outside of what's exposed through the plugin's GUI.
/// We should implement a REPL at some point for interacting with the plugin.
pub fn nih_export_standalone<P: Plugin>() {
    nih_export_standalone_with_args::<P, _>(std::env::args())
}

pub fn nih_export_standalone_with_args<P: Plugin, Args: IntoIterator<Item = String>>(args: Args) {
    // TODO: Do something with the arguments

    Wrapper::<P>::new();

    // TODO: Open the editor if available, do IO things
    // TODO: If the plugin has an editor, block until the editor is closed. Otherwise block
    //       indefinitely or until SIGINT (how do signal handlers work in Rust?)
}
