use nih_plug::prelude::*;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::f32;
use std::sync::Arc;

const WINDOW_SIZE: usize = 2048;
const OVERLAP_TIMES: usize = 4;

struct Stft {
    params: Arc<StftParams>,

    /// An adapter that performs most of the overlap-add algorithm for us.
    stft: util::StftHelper,
    /// A Hann window function, passed to the overlap-add helper.
    window_function: Vec<f32>,

    /// The FFT of a simple low-pass FIR filter.
    lp_filter_kernel: Vec<Complex32>,

    /// The algorithm for the FFT operation.
    r2c_plan: Arc<dyn RealToComplex<f32>>,
    /// The algorithm for the IFFT operation.
    c2r_plan: Arc<dyn ComplexToReal<f32>>,
    /// The output of our real->complex FFT.
    complex_fft_buffer: Vec<Complex32>,
}

#[derive(Params)]
struct StftParams {}

impl Default for Stft {
    fn default() -> Self {
        let mut planner = RealFftPlanner::new();
        let r2c_plan = planner.plan_fft_forward(WINDOW_SIZE);
        let c2r_plan = planner.plan_fft_inverse(WINDOW_SIZE);
        let mut real_fft_buffer = r2c_plan.make_input_vec();
        let mut complex_fft_buffer = r2c_plan.make_output_vec();

        // Build a super simple low-pass filter from one of the built in window function
        const FILTER_WINDOW_SIZE: usize = 33;
        let filter_window = util::window::hann(FILTER_WINDOW_SIZE);
        real_fft_buffer[0..FILTER_WINDOW_SIZE].copy_from_slice(&filter_window);

        // And make sure to normalize this so convolution sums to 1
        let filter_normalization_factor = real_fft_buffer.iter().sum::<f32>().recip();
        for sample in &mut real_fft_buffer {
            *sample *= filter_normalization_factor;
        }

        // RustFFT doesn't actually need a scratch buffer here, so we'll pass an empty buffer
        // instead
        r2c_plan
            .process_with_scratch(&mut real_fft_buffer, &mut complex_fft_buffer, &mut [])
            .unwrap();

        Self {
            params: Arc::new(StftParams::default()),

            stft: util::StftHelper::new(2, WINDOW_SIZE),
            window_function: util::window::hann(WINDOW_SIZE),

            lp_filter_kernel: complex_fft_buffer.clone(),

            r2c_plan,
            c2r_plan,
            complex_fft_buffer,
        }
    }
}

#[allow(clippy::derivable_impls)]
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

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
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
        context.set_latency_samples(self.stft.latency_samples());

        true
    }

    fn reset(&mut self) {
        // Normally we'd also initialize the STFT helper for the correct channel count here, but we
        // only do stereo so that's not necessary. Setting the block size also zeroes out the
        // buffers.
        self.stft.set_block_size(WINDOW_SIZE);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // Compensate for the window function, the overlap, and the extra gain introduced by the
        // IDFT operation
        const GAIN_COMPENSATION: f32 = f32::consts::E / OVERLAP_TIMES as f32 / WINDOW_SIZE as f32;

        self.stft.process_overlap_add(
            buffer,
            &self.window_function,
            OVERLAP_TIMES,
            |_channel_idx, real_fft_buffer| {
                // Forward FFT, the helper has already applied window function
                self.r2c_plan
                    .process_with_scratch(real_fft_buffer, &mut self.complex_fft_buffer, &mut [])
                    .unwrap();

                // As per the convolution theorem we can simply multiply these two buffers. We'll
                // also apply the gain compensation at this point.
                for (fft_bin, kernel_bin) in self
                    .complex_fft_buffer
                    .iter_mut()
                    .zip(&self.lp_filter_kernel)
                {
                    *fft_bin *= *kernel_bin * GAIN_COMPENSATION;
                }

                // Inverse FFT back into the scratch buffer. This will be added to a ring buffer
                // which gets written back to the host at a one block delay.
                self.c2r_plan
                    .process_with_scratch(&mut self.complex_fft_buffer, real_fft_buffer, &mut [])
                    .unwrap();
            },
        );

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
