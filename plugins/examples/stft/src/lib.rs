use nih_plug::prelude::*;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::f32;
use std::sync::Arc;

/// The size of the windows we'll process at a time.
const WINDOW_SIZE: usize = 64;
/// The length of the filter's impulse response.
const FILTER_WINDOW_SIZE: usize = 33;
/// The length of the FFT window we will use to perform FFT convolution. This includes padding to
/// prevent time domain aliasing as a result of cyclic convolution.
const FFT_WINDOW_SIZE: usize = WINDOW_SIZE + FILTER_WINDOW_SIZE - 1;

/// The gain compensation we need to apply for the STFT process.
const GAIN_COMPENSATION: f32 = 1.0 / FFT_WINDOW_SIZE as f32;

struct Stft {
    params: Arc<StftParams>,

    /// An adapter that performs most of the overlap-add algorithm for us.
    stft: util::StftHelper,

    /// The FFT of a simple low-pass FIR filter.
    lp_filter_spectrum: Vec<Complex32>,

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
        let r2c_plan = planner.plan_fft_forward(FFT_WINDOW_SIZE);
        let c2r_plan = planner.plan_fft_inverse(FFT_WINDOW_SIZE);
        let mut real_fft_buffer = r2c_plan.make_input_vec();
        let mut complex_fft_buffer = r2c_plan.make_output_vec();

        // Build a super simple low-pass filter from one of the built in window functions
        let mut filter_window = util::window::hann(FILTER_WINDOW_SIZE);
        // And make sure to normalize this so convolution sums to 1
        let filter_normalization_factor = filter_window.iter().sum::<f32>().recip();
        for sample in &mut filter_window {
            *sample *= filter_normalization_factor;
        }
        real_fft_buffer[0..FILTER_WINDOW_SIZE].copy_from_slice(&filter_window);

        // RustFFT doesn't actually need a scratch buffer here, so we'll pass an empty buffer
        // instead
        r2c_plan
            .process_with_scratch(&mut real_fft_buffer, &mut complex_fft_buffer, &mut [])
            .unwrap();

        Self {
            params: Arc::new(StftParams::default()),

            // We'll process the input in `WINDOW_SIZE` chunks, but our FFT window is slightly
            // larger to account for time domain aliasing so we'll need to add some padding ot each
            // block.
            stft: util::StftHelper::new(2, WINDOW_SIZE, FFT_WINDOW_SIZE - WINDOW_SIZE),

            lp_filter_spectrum: complex_fft_buffer.clone(),

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

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // We'll only do stereo for simplicity's sake
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        // The plugin's latency consists of the block size from the overlap-add procedure and half
        // of the filter kernel's size (since we're using a linear phase/symmetrical convolution
        // kernel)
        context.set_latency_samples(self.stft.latency_samples() + (FILTER_WINDOW_SIZE as u32 / 2));

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
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.stft
            .process_overlap_add(buffer, 1, |_channel_idx, real_fft_buffer| {
                // Forward FFT, `real_fft_buffer` already is already padded with zeroes, and the
                // padding from the last iteration will have already been added back to the start of
                // the buffer
                self.r2c_plan
                    .process_with_scratch(real_fft_buffer, &mut self.complex_fft_buffer, &mut [])
                    .unwrap();

                // As per the convolution theorem we can simply multiply these two buffers. We'll
                // also apply the gain compensation at this point.
                for (fft_bin, kernel_bin) in self
                    .complex_fft_buffer
                    .iter_mut()
                    .zip(&self.lp_filter_spectrum)
                {
                    *fft_bin *= *kernel_bin * GAIN_COMPENSATION;
                }

                // Inverse FFT back into the scratch buffer. This will be added to a ring buffer
                // which gets written back to the host at a one block delay.
                self.c2r_plan
                    .process_with_scratch(&mut self.complex_fft_buffer, real_fft_buffer, &mut [])
                    .unwrap();
            });

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Stft {
    const CLAP_ID: &'static str = "com.moist-plugins-gmbh.stft";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("An example plugin using the STFT helper");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Utility,
    ];
}

impl Vst3Plugin for Stft {
    const VST3_CLASS_ID: [u8; 16] = *b"StftMoistestPlug";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Tools,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(Stft);
nih_export_vst3!(Stft);
