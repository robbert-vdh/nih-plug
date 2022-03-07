use fftw::array::AlignedVec;
use fftw::plan::{C2RPlan, C2RPlan32, R2CPlan, R2CPlan32};
use fftw::types::{c32, Flag};
use nih_plug::prelude::*;
use std::f32;
use std::pin::Pin;

const WINDOW_SIZE: usize = 2048;
const OVERLAP_TIMES: usize = 4;

struct Stft {
    params: Pin<Box<StftParams>>,

    /// An adapter that performs most of the overlap-add algorithm for us.
    stft: util::StftHelper,
    /// A Hann window window, passed to the overlap-add helper.
    window_function: Vec<f32>,

    /// The FFT of a simple low pass FIR filter.
    lp_filter_kernel: Vec<c32>,

    /// The algorithms for the FFT and IFFT operations.
    plan: Plan,
    /// Scratch buffers for computing our FFT. The [`StftHelper`] already contains a buffer for the
    /// real values.
    complex_fft_scratch_buffer: AlignedVec<c32>,
}

/// FFTW uses raw pointers which aren't Send+Sync, so we'll wrap this in a separate struct.
struct Plan {
    r2c_plan: R2CPlan32,
    c2r_plan: C2RPlan32,
}

unsafe impl Send for Plan {}
unsafe impl Sync for Plan {}

#[derive(Params)]
struct StftParams {}

impl Default for Stft {
    fn default() -> Self {
        let mut r2c_plan: R2CPlan32 = R2CPlan32::aligned(&[WINDOW_SIZE], Flag::MEASURE).unwrap();
        let c2r_plan: C2RPlan32 = C2RPlan32::aligned(&[WINDOW_SIZE], Flag::MEASURE).unwrap();
        let mut real_fft_scratch_buffer: AlignedVec<f32> = AlignedVec::new(WINDOW_SIZE);
        let mut complex_fft_scratch_buffer: AlignedVec<c32> = AlignedVec::new(WINDOW_SIZE / 2 + 1);

        // Build a super simple low pass filter from one of the built in window function
        const FILTER_WINDOW_SIZE: usize = 33;
        let filter_window = util::window::hann(FILTER_WINDOW_SIZE);
        real_fft_scratch_buffer[0..FILTER_WINDOW_SIZE].copy_from_slice(&filter_window);

        // And make sure to normalize this so convolution sums to 1
        let filter_normalization_factor = real_fft_scratch_buffer.iter().sum::<f32>().recip();
        for sample in real_fft_scratch_buffer.as_slice_mut() {
            *sample *= filter_normalization_factor;
        }

        r2c_plan
            .r2c(
                &mut real_fft_scratch_buffer,
                &mut complex_fft_scratch_buffer,
            )
            .unwrap();

        Self {
            params: Box::pin(StftParams::default()),

            stft: util::StftHelper::new(2, WINDOW_SIZE),
            window_function: util::window::hann(WINDOW_SIZE),

            lp_filter_kernel: complex_fft_scratch_buffer
                .iter()
                .take(WINDOW_SIZE)
                .copied()
                .collect(),

            plan: Plan { r2c_plan, c2r_plan },
            complex_fft_scratch_buffer,
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
        // Compensate for the window function, the overlap, and the extra gain introduced by the
        // IDFT operation
        const GAIN_COMPENSATION: f32 = f32::consts::E / OVERLAP_TIMES as f32 / WINDOW_SIZE as f32;

        self.stft.process_overlap_add(
            buffer,
            &self.window_function,
            OVERLAP_TIMES,
            |_channel_idx, real_fft_scratch_buffer| {
                // Forward FFT, the helper has already applied window function
                self.plan
                    .r2c_plan
                    .r2c(
                        real_fft_scratch_buffer,
                        &mut self.complex_fft_scratch_buffer,
                    )
                    .unwrap();

                // As per the convolution theorem we can simply multiply these two buffers. We'll
                // also apply the gain compensation at this point.
                for (fft_bin, kernel_bin) in self
                    .complex_fft_scratch_buffer
                    .as_slice_mut()
                    .iter_mut()
                    .zip(&self.lp_filter_kernel)
                {
                    *fft_bin *= *kernel_bin * GAIN_COMPENSATION;
                }

                // Inverse FFT back into the scratch buffer. This will be added to a ring buffer
                // which gets written back to the host at a one block delay.
                self.plan
                    .c2r_plan
                    .c2r(
                        &mut self.complex_fft_scratch_buffer,
                        real_fft_scratch_buffer,
                    )
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
