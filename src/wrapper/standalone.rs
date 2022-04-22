//! A standalone plugin target that directly connects to the system's audio and MIDI ports instead
//! of relying on a plugin host. This is mostly useful for quickly testing GUI changes.

use self::wrapper::Wrapper;
use crate::plugin::Plugin;

mod wrapper;

/// Open an NIH-plug plugin as a standalone application. If the plugin has an editor, this will open
/// the editor and block until the editor is closed. Otherwise this will block until SIGINT is
/// received. This is mainly useful for quickly testing plugin GUIs. You should call this function
/// from a `main()` function.
///
/// By default this will connect to the 'default' audio and MIDI ports. Use the command line options
/// to change this. `--help` lists all available options.
///
/// # TODO
///
/// The aforementioned command line options have not yet been implemented.
//
// TODO: Actually implement command line flags for changing the IO configuration
// TODO: Add a way to set the IO configuration at runtime, for instance through the plugin's GUI
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
