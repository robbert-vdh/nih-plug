use nih_plug::prelude::*;
use std::pin::Pin;

const WINDOW_SIZE: usize = 2048;

struct Stft {
    params: Pin<Box<StftParams>>,

    stft: util::StftHelper,
}

#[derive(Params)]
struct StftParams {}

impl Default for Stft {
    fn default() -> Self {
        Self {
            params: Box::pin(StftParams::default()),

            stft: util::StftHelper::new(2, WINDOW_SIZE),
        }
    }
}

impl Default for StftParams {
    fn default() -> Self {
        Self {}
    }
}

impl Plugin for Stft {
    const NAME: &'static str = "STFT Example";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const ACCEPTS_MIDI: bool = false;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // We'll only do stereo for simplicity's sake
        config.num_input_channels == config.num_output_channels && config.num_input_channels == 2
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        _buffer_config: &BufferConfig,
        context: &mut impl ProcessContext,
    ) -> bool {
        // Normally we'd also initialize the STFT helper for the correct channel count here, but we
        // only do stereo so that's not necessary
        self.stft.set_block_size(WINDOW_SIZE);
        context.set_latency_samples(self.stft.latency_samples());

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        self.stft.process(buffer, [], |block, _| {
            for channel_samples in block.iter_mut() {
                for sample in channel_samples {
                    // TODO: Use the FFTW bindings and do some STFT operation here instead of
                    //       reducing the gain at a 512 sample latency...
                    *sample *= 0.5;
                }
            }
        });

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Stft {
    const CLAP_ID: &'static str = "com.moist-plugins-gmbh.stft";
    const CLAP_DESCRIPTION: &'static str = "An example plugin using the STFT helper";
    const CLAP_FEATURES: &'static [&'static str] = &["audio_effect", "stereo", "tool"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Stft {
    const VST3_CLASS_ID: [u8; 16] = *b"StftMoistestPlug";
    const VST3_CATEGORIES: &'static str = "Fx|Tools";
}

nih_export_clap!(Stft);
nih_export_vst3!(Stft);
