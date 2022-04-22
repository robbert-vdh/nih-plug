use parking_lot::RwLock;
use std::sync::Arc;

use super::context::WrapperProcessContext;
use crate::context::Transport;
use crate::plugin::{BufferConfig, BusConfig, Editor, Plugin};

/// Configuration for a standalone plugin that would normally be provided by the DAW.
#[derive(Debug, Clone)]
pub struct WrapperConfig {
    /// The number of input channels.
    pub input_channels: u32,
    /// The number of output channels.
    pub output_channels: u32,
    /// The audio backend's sample rate.
    pub sample_rate: f32,
    /// The audio backend's period size.
    pub period_size: u32,

    /// The current tempo.
    pub tempo: f32,
    /// The time signature's numerator.
    pub timesig_num: u32,
    /// The time signature's denominator.
    pub timesig_denom: u32,
}

pub struct Wrapper<P: Plugin> {
    /// The wrapped plugin instance.
    plugin: RwLock<P>,
    /// The plugin's editor, if it has one. This object does not do anything on its own, but we need
    /// to instantiate this in advance so we don't need to lock the entire [`Plugin`] object when
    /// creating an editor.
    editor: Option<Box<dyn Editor>>,

    config: WrapperConfig,

    /// The bus and buffer configurations are static for the standalone target.
    bus_config: BusConfig,
    buffer_config: BufferConfig,
}

/// Errors that may arise while initializing the wrapped plugins.
#[derive(Debug, Clone, Copy)]
pub enum WrapperError {
    /// The plugin does not accept the IO configuration from the config.
    IncompatibleConfig,
    /// The plugin returned `false` during initialization.
    InitializationFailed,
}

impl<P: Plugin> Wrapper<P> {
    /// Instantiate a new instance of the standalone wrapper. Returns an error if the plugin does
    /// not accept the IO configuration from the wrapper config.
    pub fn new(config: WrapperConfig) -> Result<Arc<Self>, WrapperError> {
        let plugin = P::default();
        let editor = plugin.editor();

        let wrapper = Arc::new(Wrapper {
            plugin: RwLock::new(plugin),
            editor,

            bus_config: BusConfig {
                num_input_channels: config.input_channels,
                num_output_channels: config.output_channels,
            },
            buffer_config: BufferConfig {
                sample_rate: config.sample_rate,
                max_buffer_size: config.period_size,
            },
            config,
        });

        // Right now the IO configuration is fixed in the standalone target, so if the plugin cannot
        // work with this then we cannot initialize the plugin at all.
        {
            let mut plugin = wrapper.plugin.write();
            if !plugin.accepts_bus_config(&wrapper.bus_config) {
                return Err(WrapperError::IncompatibleConfig);
            }

            if !plugin.initialize(
                &wrapper.bus_config,
                &wrapper.buffer_config,
                &mut wrapper.make_process_context(Transport::new(wrapper.config.sample_rate)),
            ) {
                return Err(WrapperError::InitializationFailed);
            }
        }

        Ok(wrapper)
    }

    /// Open the editor, start processing audio, and block this thread until the editor is closed.
    /// If the plugin does not have an editor, then this will block until SIGINT is received.
    ///
    /// Will return an error if the plugin threw an error during audio processing or if the editor
    /// could not be opened.
    pub fn run(self: Arc<Self>) -> Result<(), WrapperError> {
        // TODO: Open the editor and block until it is closed
        // TODO: Do IO things
        // TODO: Block until SIGINT is received if the plugin does not have an editor

        Ok(())
    }

    fn make_process_context(&self, transport: Transport) -> WrapperProcessContext<'_, P> {
        WrapperProcessContext {
            wrapper: self,
            transport,
        }
    }
}
