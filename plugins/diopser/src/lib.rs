#[macro_use]
extern crate nih_plug;

use nih_plug::Params;
use nih_plug::{
    Buffer, BufferConfig, BusConfig, Plugin, ProcessContext, ProcessStatus, Vst3Plugin,
};
use std::pin::Pin;

struct Diopser {
    params: Pin<Box<DiopserParams>>,
}

#[derive(Params)]
struct DiopserParams {}

impl Default for Diopser {
    fn default() -> Self {
        Self {
            params: Box::pin(DiopserParams::default()),
        }
    }
}

impl Default for DiopserParams {
    fn default() -> Self {
        Self {}
    }
}

impl Plugin for Diopser {
    const NAME: &'static str = "Diopser";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = "0.2.0";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This works with any symmetrical IO layout
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        _buffer_config: &BufferConfig,
        _context: &mut impl ProcessContext,
    ) -> bool {
        true
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Diopser {
    const VST3_CLASS_ID: [u8; 16] = *b"DiopserPlugRvdH.";
    const VST3_CATEGORIES: &'static str = "Fx|Filter";
}

nih_export_vst3!(Diopser);
