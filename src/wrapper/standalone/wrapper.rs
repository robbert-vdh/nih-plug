use parking_lot::RwLock;
use std::sync::Arc;

use super::context::WrapperProcessContext;
use crate::context::Transport;
use crate::plugin::{BufferConfig, BusConfig, Plugin};

/// Configuration for a standalone plugin that would normally be provided by the DAW.
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

    config: WrapperConfig,

    /// The bus and buffer configurations are static for the standalone target.
    bus_config: BusConfig,
    buffer_config: BufferConfig,
}

/// Errors that may arise while initializing the wrapped plugins.
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
        let wrapper = Arc::new(Wrapper {
            plugin: RwLock::new(P::default()),
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

    fn make_process_context(&self, transport: Transport) -> WrapperProcessContext<'_, P> {
        WrapperProcessContext {
            wrapper: self,
            transport,
        }
    }
}
