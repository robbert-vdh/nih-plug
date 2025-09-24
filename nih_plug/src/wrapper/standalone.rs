//! A standalone plugin target that directly connects to the system's audio and MIDI ports instead
//! of relying on a plugin host. This is mostly useful for quickly testing GUI changes.

use clap::{CommandFactory, FromArgMatches};

use self::backend::Backend;
use self::config::WrapperConfig;
use self::wrapper::{Wrapper, WrapperError};
use super::util::setup_logger;
use crate::prelude::Plugin;

mod backend;
mod config;
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
/// use nih_plug::prelude::*;
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
    // TODO: If the backend fails to initialize then the standalones will exit normally instead of
    //       with an error code. This should probably be changed.
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

    match config.backend {
        config::BackendType::Auto => {
            let result = backend::Jack::new::<P>(config.clone()).map(|backend| {
                nih_log!("Using the JACK backend");
                run_wrapper::<P, _>(backend, config.clone())
            });

            #[cfg(target_os = "linux")]
            let result = result.or_else(|_| {
                match backend::CpalMidir::new::<P>(config.clone(), cpal::HostId::Alsa) {
                    Ok(backend) => {
                        nih_log!("Using the ALSA backend");
                        Ok(run_wrapper::<P, _>(backend, config.clone()))
                    }
                    Err(err) => {
                        nih_error!(
                            "Could not initialize either the JACK or the ALSA backends, falling \
                             back to the dummy audio backend: {err:#}"
                        );
                        Err(())
                    }
                }
            });
            #[cfg(target_os = "macos")]
            let result = result.or_else(|_| {
                match backend::CpalMidir::new::<P>(config.clone(), cpal::HostId::CoreAudio) {
                    Ok(backend) => {
                        nih_log!("Using the CoreAudio backend");
                        Ok(run_wrapper::<P, _>(backend, config.clone()))
                    }
                    Err(err) => {
                        nih_error!(
                            "Could not initialize either the JACK or the CoreAudio backends, \
                             falling back to the dummy audio backend: {err:#}"
                        );
                        Err(())
                    }
                }
            });
            #[cfg(target_os = "windows")]
            let result = result.or_else(|_| {
                match backend::CpalMidir::new::<P>(config.clone(), cpal::HostId::Wasapi) {
                    Ok(backend) => {
                        nih_log!("Using the WASAPI backend");
                        Ok(run_wrapper::<P, _>(backend, config.clone()))
                    }
                    Err(err) => {
                        nih_error!(
                            "Could not initialize either the JACK or the WASAPI backends, falling \
                             back to the dummy audio backend: {err:#}"
                        );
                        Err(())
                    }
                }
            });

            result.unwrap_or_else(|_| {
                nih_error!("Falling back to the dummy audio backend, audio and MIDI will not work");
                run_wrapper::<P, _>(backend::Dummy::new::<P>(config.clone()), config)
            })
        }
        config::BackendType::Jack => match backend::Jack::new::<P>(config.clone()) {
            Ok(backend) => run_wrapper::<P, _>(backend, config),
            Err(err) => {
                nih_error!("Could not initialize the JACK backend: {:#}", err);
                false
            }
        },
        #[cfg(target_os = "linux")]
        config::BackendType::Alsa => {
            match backend::CpalMidir::new::<P>(config.clone(), cpal::HostId::Alsa) {
                Ok(backend) => run_wrapper::<P, _>(backend, config),
                Err(err) => {
                    nih_error!("Could not initialize the ALSA backend: {:#}", err);
                    false
                }
            }
        }
        #[cfg(target_os = "macos")]
        config::BackendType::CoreAudio => {
            match backend::CpalMidir::new::<P>(config.clone(), cpal::HostId::CoreAudio) {
                Ok(backend) => run_wrapper::<P, _>(backend, config),
                Err(err) => {
                    nih_error!("Could not initialize the CoreAudio backend: {:#}", err);
                    false
                }
            }
        }
        #[cfg(target_os = "windows")]
        config::BackendType::Wasapi => {
            match backend::CpalMidir::new::<P>(config.clone(), cpal::HostId::Wasapi) {
                Ok(backend) => run_wrapper::<P, _>(backend, config),
                Err(err) => {
                    nih_error!("Could not initialize the WASAPI backend: {:#}", err);
                    false
                }
            }
        }
        config::BackendType::Dummy => {
            run_wrapper::<P, _>(backend::Dummy::new::<P>(config.clone()), config)
        }
    }
}

fn run_wrapper<P: Plugin, B: Backend<P>>(backend: B, config: WrapperConfig) -> bool {
    let wrapper = match Wrapper::<P, _>::new(backend, config) {
        Ok(wrapper) => wrapper,
        Err(err) => {
            print_error(err);
            return false;
        }
    };

    // TODO: Add a repl while the application is running to interact with parameters
    match wrapper.run() {
        Ok(()) => true,
        Err(err) => {
            print_error(err);
            false
        }
    }
}

fn print_error(error: WrapperError) {
    match error {
        WrapperError::InitializationFailed => {
            nih_error!("The plugin failed to initialize");
        }
    }
}
