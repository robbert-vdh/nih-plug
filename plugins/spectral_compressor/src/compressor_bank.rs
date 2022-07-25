// Spectral Compressor: an FFT based compressor
// Copyright (C) 2021-2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use nih_plug::prelude::*;
use realfft::num_complex::Complex32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::SpectralCompressorParams;

// These are the parameter ID prefixes used for the downwards and upwards cmpression parameters.
const DOWNWARDS_NAME_PREFIX: &str = "downwards_";
const UPWARDS_NAME_PREFIX: &str = "upwards_";

/// A bank of compressors so each FFT bin can be compressed individually. The vectors in this struct
/// will have a capacity of `MAX_WINDOW_SIZE / 2 + 1` and a size that matches the current complex
/// FFT buffer size. This is stored as a struct of arrays to make SIMD-ing easier in the future.
pub struct CompressorBank {
    /// If set, then the downwards thresholds should be updated on the next processing cycle. Can be
    /// set from a parameter value change listener, and is also set when calling `.reset_for_size`.
    pub should_update_downwards_thresholds: Arc<AtomicBool>,
    /// The same as `should_update_downwards_thresholds`, but for upwards thresholds.
    pub should_update_upwards_thresholds: Arc<AtomicBool>,
    /// If set, then the downwards ratios should be updated on the next processing cycle. Can be set
    /// from a parameter value change listener, and is also set when calling `.reset_for_size`.
    pub should_update_downwards_ratios: Arc<AtomicBool>,
    /// The same as `should_update_downwards_ratios`, but for upwards ratios.
    pub should_update_upwards_ratios: Arc<AtomicBool>,

    /// For each compressor bin, `log2(freq)` where `freq` is the frequency associated with that
    /// compressor. This is precomputed since all update functions need it.
    log2_freqs: Vec<f32>,

    /// Downwards compressor thresholds, in linear space.
    downwards_thresholds: Vec<f32>,
    /// The start (lower end) of the downwards's knee range, in linear space. This is calculated in
    /// decibel/log space and then converted to gain to keep everything in linear space.
    downwards_knee_starts: Vec<f32>,
    /// The end (upper end) of the downwards's knee range, in linear space.
    downwards_knee_ends: Vec<f32>,
    /// The reciprocals of the downwards compressor ratios. At 1.0 the cmopressor won't do anything.
    /// If [`CompressorBankParams::high_freq_ratio_rolloff`] is set to 1.0, then this will be the
    /// same for each compressor. We're doing the compression in linear space to avoid a logarithm,
    /// so the division by the ratio becomes an nth-root, or exponentation by the reciprocal of the
    /// ratio.
    downwards_ratio_recips: Vec<f32>,

    /// Upwards compressor thresholds, in linear space.
    upwards_thresholds: Vec<f32>,
    /// The start (lower end) of the upwards's knee range, in linear space.
    upwards_knee_starts: Vec<f32>,
    /// The end (upper end) of the upwards's knee range, in linear space.
    upwards_knee_ends: Vec<f32>,
    /// The same as `downwards_ratio_recipss`, but for the upwards compression.
    upwards_ratio_recips: Vec<f32>,

    /// The current envelope value for this bin, in linear space. Indexed by
    /// `[channel_idx][compressor_idx]`.
    envelopes: Vec<Vec<f32>>,
    /// When sidechaining is enabled, this contains the per-channel frqeuency spectrum magnitudes
    /// for the current block. The compressor thresholds and knee values are multiplied by these
    /// values to get the effective thresholds.
    sidechain_spectrum_magnitudes: Vec<Vec<f32>>,
    /// The window size this compressor bank was configured for. This is used to compute the
    /// coefficients for the envelope followers in the process function.
    window_size: usize,
    /// The sample rate this compressor bank was configured for. This is used to compute the
    /// coefficients for the envelope followers in the process function.
    sample_rate: f32,
}

#[derive(Params)]
pub struct ThresholdParams {
    /// The compressor threshold at the center frequency. When sidechaining is enabled, the input
    /// signal is gained by the inverse of this value. This replaces the input gain in the original
    /// Spectral Compressor. In the polynomial below, this is the intercept.
    #[id = "tresh_global"]
    pub threshold_db: FloatParam,
    /// The center frqeuency for the target curve when sidechaining is not enabled. The curve is a
    /// polynomial `threshold_db + curve_slope*x + curve_curve*(x^2)` that evaluates to a decibel
    /// value, where `x = log2(center_frequency) - log2(bin_frequency)`. In other words, this is
    /// evaluated in the log/log domain for decibels and octaves.
    #[id = "thresh_center_freq"]
    pub center_frequency: FloatParam,
    /// The slope for the curve, in the log/log domain. See the polynomial above.
    #[id = "thresh_curve_slope"]
    pub curve_slope: FloatParam,
    /// The, uh, 'curve' for the curve, in the logarithmic domain. This is the third coefficient in
    /// the quadratic polynomial and controls the parabolic behavior. Positive values turn the curve
    /// into a v-shaped curve, while negative values attenuate everything outside of the center
    /// frequency. See the polynomial above.
    #[id = "thresh_curve_curve"]
    pub curve_curve: FloatParam,

    /// Controls the type of threshold that should be used. Check [`ThresholdMode`] for more
    /// information.
    #[id = "thresh_mode"]
    pub mode: EnumParam<ThresholdMode>,
    /// A `[0, 1]` parameter that controls how much of the other channels should be mixed in when
    /// computing the channel gain value that is then multiplied with he thresholds and knee values
    /// to the the compression parameters when using the sidechain modes.
    #[id = "thresh_sc_link"]
    pub sc_channel_link: FloatParam,
}

/// The type of threshold to use.
#[derive(Enum, Debug, PartialEq, Eq)]
pub enum ThresholdMode {
    /// Configure the thresholds to offset pink noise. This means that the slope will receive an
    /// additional -3 dB/octave slope.
    #[id = "internal"]
    #[name = "Pink Noise"]
    Internal,
    /// Dynamically reconfigure the thresholds based on a sidechain input. The -3 dB/octave slope
    /// offset is not applied here so the curve stays true to the sidechain input at the default
    /// settings. This works by simply multiplying the sidechain gain levels with the precomputed
    /// threshold, knee start, and knee end values. The sidechain channel linking option determines
    /// how how much of the other channel values to mix in before multiplying the sidechain gain
    /// values with the thresholds.
    #[id = "sidechain"]
    #[name = "Sidechain"]
    Sidechain,
}

/// Contains the compressor parameters for both the upwards and downwards compressor banks.
#[derive(Params)]
pub struct CompressorBankParams {
    #[nested = "upwards"]
    pub upwards: Arc<CompressorParams>,
    #[nested = "downwards"]
    pub downwards: Arc<CompressorParams>,
}

/// This struct contains the parameters for either the upward or downward compressors. The `Params`
/// trait is implemented manually to avoid copy-pasting parameters for both types of compressor.
/// Both versions will have a parameter ID and a parameter name prefix to distinguish them.
pub struct CompressorParams {
    /// The prefix to use in the `.param_map()` function so the upwards and downwards compressors
    /// get unique parameter IDs.
    param_id_prefix: &'static str,

    /// The compression threshold relative to the target curve.
    pub threshold_offset_db: FloatParam,
    /// The compression ratio. At 1.0 the compressor is disengaged.
    pub ratio: FloatParam,
    /// A `[0, 1]` scaling factor that causes the compressors for the higher registers to have lower
    /// ratios than the compressors for the lower registers. The scaling is applied logarithmically
    /// rather than linearly over the compressors. If this is set to 1.0, then the ratios will be
    /// the same for every compressor.
    pub high_freq_ratio_rolloff: FloatParam,
    /// The compression knee width, in decibels.
    pub knee_width_db: FloatParam,
}

unsafe impl Params for CompressorParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        let prefix = self.param_id_prefix;
        vec![
            (
                format!("{prefix}threshold_offset"),
                self.threshold_offset_db.as_ptr(),
                // The parent `CompressorBankParams` struct will add the group here
                String::new(),
            ),
            (format!("{prefix}ratio"), self.ratio.as_ptr(), String::new()),
            (
                format!("{prefix}high_freq_rolloff"),
                self.high_freq_ratio_rolloff.as_ptr(),
                String::new(),
            ),
            (
                format!("{prefix}knee"),
                self.knee_width_db.as_ptr(),
                String::new(),
            ),
        ]
    }
}

impl ThresholdParams {
    /// Create a new [`ThresholdParams`] object. Changing any of the threshold parameters causes the
    /// passed compressor bank's thresholds to be updated.
    pub fn new(compressor_bank: &CompressorBank) -> Self {
        let should_update_downwards_thresholds =
            compressor_bank.should_update_downwards_thresholds.clone();
        let should_update_upwards_thresholds =
            compressor_bank.should_update_upwards_thresholds.clone();
        let set_update_both_thresholds = Arc::new(move |_| {
            should_update_downwards_thresholds.store(true, Ordering::SeqCst);
            should_update_upwards_thresholds.store(true, Ordering::SeqCst);
        });

        ThresholdParams {
            threshold_db: FloatParam::new(
                "Global Threshold",
                0.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 20.0,
                },
            )
            .with_callback(set_update_both_thresholds.clone())
            .with_unit(" dB")
            .with_step_size(0.1),
            center_frequency: FloatParam::new(
                "Threshold Center",
                420.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_callback(set_update_both_thresholds.clone())
            // This includes the unit
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            // These are polynomial coefficients that are evaluated in the log/log domain
            // (octaves/decibels). The global threshold is the intercept.
            curve_slope: FloatParam::new(
                "Threshold Slope",
                0.0,
                FloatRange::SymmetricalSkewed {
                    min: -36.0,
                    max: 36.0,
                    factor: FloatRange::skew_factor(-2.0),
                    center: 0.0,
                },
            )
            .with_callback(set_update_both_thresholds.clone())
            .with_unit(" dB/oct")
            .with_step_size(0.01),
            curve_curve: FloatParam::new(
                "Threshold Curve",
                0.0,
                FloatRange::SymmetricalSkewed {
                    min: -24.0,
                    max: 24.0,
                    factor: FloatRange::skew_factor(-2.0),
                    center: 0.0,
                },
            )
            .with_callback(set_update_both_thresholds.clone())
            .with_unit(" dB/oct²")
            .with_step_size(0.01),

            mode: EnumParam::new("Mode", ThresholdMode::Internal)
                // Not the most efficient way to do this, but it's a bit cleaner than the
                // alternative
                .with_callback(Arc::new(move |_| set_update_both_thresholds(0.0))),
            sc_channel_link: FloatParam::new(
                "SC Channel Link",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

impl CompressorBankParams {
    /// Create compressor bank parameter objects for both the downwards and upwards compressors of
    /// `compressor`. Changing the ratio and threshold parameters will cause the compressor to
    /// recompute its values on the next processing cycle.
    pub fn new(compressor: &CompressorBank) -> Self {
        CompressorBankParams {
            downwards: Arc::new(CompressorParams::new(
                DOWNWARDS_NAME_PREFIX,
                "Downwards",
                compressor.should_update_downwards_thresholds.clone(),
                compressor.should_update_downwards_ratios.clone(),
            )),
            upwards: Arc::new(CompressorParams::new(
                UPWARDS_NAME_PREFIX,
                "Upwards",
                compressor.should_update_upwards_thresholds.clone(),
                compressor.should_update_upwards_ratios.clone(),
            )),
        }
    }
}

impl CompressorParams {
    /// Create a new [`CompressorBankParams`] object with a prefix for all parameter names. Changing
    /// any of the threshold or ratio parameters causes the passed atomics to be updated. These
    /// should be taken from a [`CompressorBank`] so the parameters are linked to it.
    pub fn new(
        param_id_prefix: &'static str,
        name_prefix: &str,
        should_update_thresholds: Arc<AtomicBool>,
        should_update_ratios: Arc<AtomicBool>,
    ) -> Self {
        let set_update_thresholds =
            Arc::new(move |_| should_update_thresholds.store(true, Ordering::SeqCst));
        let set_update_ratios =
            Arc::new(move |_| should_update_ratios.store(true, Ordering::SeqCst));

        CompressorParams {
            param_id_prefix,

            // TODO: Set nicer default values for these things
            // As explained above, these offsets are relative to the target curve
            threshold_offset_db: FloatParam::new(
                format!("{name_prefix} Offset"),
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_callback(set_update_thresholds)
            .with_unit(" dB")
            .with_step_size(0.1),
            ratio: FloatParam::new(
                format!("{name_prefix} Ratio"),
                1.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 300.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_callback(set_update_ratios.clone())
            .with_step_size(0.01)
            .with_value_to_string(formatters::v2s_compression_ratio(2))
            .with_string_to_value(formatters::s2v_compression_ratio()),
            high_freq_ratio_rolloff: FloatParam::new(
                format!("{name_prefix} Hi-Freq Rolloff"),
                // TODO: Bit of a hacky way to set the default values differently for upwards and
                //       downwards compressors
                if param_id_prefix == UPWARDS_NAME_PREFIX {
                    0.75
                } else {
                    // These basically work in the opposite way
                    0.25
                },
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_callback(set_update_ratios)
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            knee_width_db: FloatParam::new(
                format!("{name_prefix} Knee"),
                6.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 36.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
        }
    }
}

impl CompressorBank {
    /// Set up the compressor for the given channel count and maximum FFT window size. The
    /// compressors won't be initialized yet.
    pub fn new(num_channels: usize, max_window_size: usize) -> Self {
        let complex_buffer_len = max_window_size / 2 + 1;

        CompressorBank {
            should_update_downwards_thresholds: Arc::new(AtomicBool::new(true)),
            should_update_upwards_thresholds: Arc::new(AtomicBool::new(true)),
            should_update_downwards_ratios: Arc::new(AtomicBool::new(true)),
            should_update_upwards_ratios: Arc::new(AtomicBool::new(true)),

            log2_freqs: Vec::with_capacity(complex_buffer_len),

            downwards_thresholds: Vec::with_capacity(complex_buffer_len),
            downwards_knee_starts: Vec::with_capacity(complex_buffer_len),
            downwards_knee_ends: Vec::with_capacity(complex_buffer_len),
            downwards_ratio_recips: Vec::with_capacity(complex_buffer_len),

            upwards_thresholds: Vec::with_capacity(complex_buffer_len),
            upwards_knee_starts: Vec::with_capacity(complex_buffer_len),
            upwards_knee_ends: Vec::with_capacity(complex_buffer_len),
            upwards_ratio_recips: Vec::with_capacity(complex_buffer_len),

            envelopes: vec![Vec::with_capacity(complex_buffer_len); num_channels],
            sidechain_spectrum_magnitudes: vec![
                Vec::with_capacity(complex_buffer_len);
                num_channels
            ],
            window_size: 0,
            sample_rate: 1.0,
        }
    }

    /// Change the capacities of the internal buffers to fit new parameters. Use the
    /// `.reset_for_size()` method to clear the buffers and set the current window size.
    pub fn update_capacity(&mut self, num_channels: usize, max_window_size: usize) {
        let complex_buffer_len = max_window_size / 2 + 1;

        self.log2_freqs
            .reserve_exact(complex_buffer_len.saturating_sub(self.log2_freqs.len()));

        self.downwards_thresholds
            .reserve_exact(complex_buffer_len.saturating_sub(self.downwards_thresholds.len()));
        self.downwards_ratio_recips
            .reserve_exact(complex_buffer_len.saturating_sub(self.downwards_ratio_recips.len()));
        self.downwards_knee_starts
            .reserve_exact(complex_buffer_len.saturating_sub(self.downwards_knee_starts.len()));
        self.downwards_knee_ends
            .reserve_exact(complex_buffer_len.saturating_sub(self.downwards_knee_ends.len()));

        self.upwards_thresholds
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_thresholds.len()));
        self.upwards_ratio_recips
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_ratio_recips.len()));
        self.upwards_knee_starts
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_knee_starts.len()));
        self.upwards_knee_ends
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_knee_ends.len()));

        self.envelopes.resize_with(num_channels, Vec::new);
        for envelopes in self.envelopes.iter_mut() {
            envelopes.reserve_exact(complex_buffer_len.saturating_sub(envelopes.len()));
        }

        self.sidechain_spectrum_magnitudes
            .resize_with(num_channels, Vec::new);
        for magnitudes in self.sidechain_spectrum_magnitudes.iter_mut() {
            magnitudes.reserve_exact(complex_buffer_len.saturating_sub(magnitudes.len()));
        }
    }

    /// Resize the number of compressors to match the current window size. Also precomputes the
    /// 2-log frequencies for each bin.
    ///
    /// If the window size is larger than the maximum window size, then this will allocate.
    pub fn resize(&mut self, buffer_config: &BufferConfig, window_size: usize) {
        let complex_buffer_len = window_size / 2 + 1;

        // These 2-log frequencies are needed when updating the compressor parameters, so we'll just
        // precompute them to avoid having to repeat the same expensive computations all the time
        self.log2_freqs.resize(complex_buffer_len, 0.0);
        // The first one should always stay at zero, `0.0f32.log2() == NaN`.
        for (i, log2_freq) in self.log2_freqs.iter_mut().enumerate().skip(1) {
            let freq = (i as f32 / window_size as f32) * buffer_config.sample_rate;
            *log2_freq = freq.log2();
        }

        self.downwards_thresholds.resize(complex_buffer_len, 1.0);
        self.downwards_ratio_recips.resize(complex_buffer_len, 1.0);
        self.downwards_knee_starts.resize(complex_buffer_len, 1.0);
        self.downwards_knee_ends.resize(complex_buffer_len, 1.0);

        self.upwards_thresholds.resize(complex_buffer_len, 1.0);
        self.upwards_ratio_recips.resize(complex_buffer_len, 1.0);
        self.upwards_knee_starts.resize(complex_buffer_len, 1.0);
        self.upwards_knee_ends.resize(complex_buffer_len, 1.0);

        for envelopes in self.envelopes.iter_mut() {
            envelopes.resize(complex_buffer_len, 0.0);
        }

        for magnitudes in self.sidechain_spectrum_magnitudes.iter_mut() {
            magnitudes.resize(complex_buffer_len, 0.0);
        }

        self.window_size = window_size;
        self.sample_rate = buffer_config.sample_rate;

        // The compressors need to be updated on the next processing cycle
        self.should_update_downwards_thresholds
            .store(true, Ordering::SeqCst);
        self.should_update_upwards_thresholds
            .store(true, Ordering::SeqCst);
        self.should_update_downwards_ratios
            .store(true, Ordering::SeqCst);
        self.should_update_upwards_ratios
            .store(true, Ordering::SeqCst);
    }

    /// Clear out the envelope followers.
    pub fn reset(&mut self) {
        for envelopes in self.envelopes.iter_mut() {
            envelopes.fill(0.0);
        }

        // Sidechain data doesn't need to be reset as it will be overwritten immediately before use
    }

    /// Apply the magnitude compression to a buffer of FFT bins. The compressors are first updated
    /// if needed. The overlap amount is needed to compute the effective sample rate. The
    /// `skip_bins_below` argument is used to avoid compressing DC bins, or the neighbouring bins
    /// the DC signal may have been convolved into because of the Hann window function.
    pub fn process(
        &mut self,
        buffer: &mut [Complex32],
        channel_idx: usize,
        params: &SpectralCompressorParams,
        overlap_times: usize,
        skip_bins_below: usize,
    ) {
        nih_debug_assert_eq!(buffer.len(), self.log2_freqs.len());

        self.update_if_needed(params);
        self.update_envelopes(buffer, channel_idx, params, overlap_times, skip_bins_below);
        self.compress(buffer, channel_idx, params, skip_bins_below);
    }

    /// Set the sidechain frequency spectrum magnitudes just before a [`process()`][Self::process()]
    /// call. These will be multiplied with the existing compressor thresholds and knee values to
    /// get the effective values for use with sidechaining.
    pub fn process_sidechain(&mut self, sc_buffer: &mut [Complex32], channel_idx: usize) {
        nih_debug_assert_eq!(sc_buffer.len(), self.log2_freqs.len());

        self.update_sidechain_spectra(sc_buffer, channel_idx);
    }

    /// Update the envelope followers based on the bin magnetudes.
    fn update_envelopes(
        &mut self,
        buffer: &mut [Complex32],
        channel_idx: usize,
        params: &SpectralCompressorParams,
        overlap_times: usize,
        skip_bins_below: usize,
    ) {
        // The coefficient the old envelope value is multiplied by when the current rectified sample
        // value is above the envelope's value. The 0 to 1 step response retains 36.8% of the old
        // value after the attack time has elapsed, and current value is 63.2% of the way towards 1.
        // The effective sample rate needs to compensate for the periodic nature of the STFT
        // operation. Since with a 2048 sample window and 4x overlap, you'd run this function once
        // for every 512 samples.
        let effective_sample_rate =
            self.sample_rate / (self.window_size as f32 / overlap_times as f32);
        let attack_old_t = if params.global.compressor_attack_ms.value == 0.0 {
            0.0
        } else {
            (-1.0 / (params.global.compressor_attack_ms.value / 1000.0 * effective_sample_rate))
                .exp()
        };
        let attack_new_t = 1.0 - attack_old_t;
        // The same as `attack_old_t`, but for the release phase of the envelope follower
        let release_old_t = if params.global.compressor_release_ms.value == 0.0 {
            0.0
        } else {
            (-1.0 / (params.global.compressor_release_ms.value / 1000.0 * effective_sample_rate))
                .exp()
        };
        let release_new_t = 1.0 - release_old_t;

        for (bin, envelope) in buffer
            .iter()
            .zip(self.envelopes[channel_idx].iter_mut())
            .skip(skip_bins_below)
        {
            let magnitude = bin.norm();
            if *envelope > magnitude {
                // Release stage
                *envelope = (release_old_t * *envelope) + (release_new_t * magnitude);
            } else {
                // Attack stage
                *envelope = (attack_old_t * *envelope) + (attack_new_t * magnitude);
            }
        }
    }

    /// Update the spectral data using the sidechain input
    fn update_sidechain_spectra(&mut self, sc_buffer: &mut [Complex32], channel_idx: usize) {
        nih_debug_assert!(channel_idx < self.sidechain_spectrum_magnitudes.len());

        for (bin, magnitude) in sc_buffer
            .iter()
            .zip(self.sidechain_spectrum_magnitudes[channel_idx].iter_mut())
        {
            *magnitude = bin.norm();
        }
    }

    /// Actually do the thing. [`Self::update_envelopes()`] must have been called before calling
    /// this.
    ///
    /// # Panics
    ///
    /// Panics if the buffer does not have the same length as the one that was passed to the last
    /// `resize()` call.
    fn compress(
        &self,
        buffer: &mut [Complex32],
        channel_idx: usize,
        params: &SpectralCompressorParams,
        skip_bins_below: usize,
    ) {
        // Well I'm not sure at all why this scaling works, but it does. With higher knee
        // bandwidths, the middle values needs to be pushed more towards the post-knee threshold
        // than with lower knee values. These scaling factors are used as exponents.
        let downwards_knee_scaling_factor =
            compute_knee_scaling_factor(params.compressors.downwards.knee_width_db.value);
        // Note the square root here, since the curve needs to go the other way for the upwards
        // version
        let upwards_knee_scaling_factor =
            compute_knee_scaling_factor(params.compressors.upwards.knee_width_db.value).sqrt();

        assert!(self.downwards_thresholds.len() == buffer.len());
        assert!(self.downwards_ratio_recips.len() == buffer.len());
        assert!(self.downwards_knee_starts.len() == buffer.len());
        assert!(self.downwards_knee_ends.len() == buffer.len());
        assert!(self.upwards_thresholds.len() == buffer.len());
        assert!(self.upwards_ratio_recips.len() == buffer.len());
        assert!(self.upwards_knee_starts.len() == buffer.len());
        assert!(self.upwards_knee_ends.len() == buffer.len());
        for (bin_idx, (bin, envelope)) in buffer
            .iter_mut()
            .zip(self.envelopes[channel_idx].iter())
            .enumerate()
            .skip(skip_bins_below)
        {
            // This works by computing a scaling factor, and then scaling the bin magnitudes by that.
            let mut scale = 1.0;

            // All compression happens in the linear domain to save a logarithm
            // SAFETY: These sizes were asserted above
            let downwards_threshold = unsafe { self.downwards_thresholds.get_unchecked(bin_idx) };
            let downwards_ratio_recip =
                unsafe { self.downwards_ratio_recips.get_unchecked(bin_idx) };
            let downwards_knee_start = unsafe { self.downwards_knee_starts.get_unchecked(bin_idx) };
            let downwards_knee_end = unsafe { self.downwards_knee_ends.get_unchecked(bin_idx) };
            if *downwards_ratio_recip != 1.0 {
                scale *= compress_downwards(
                    *envelope,
                    *downwards_threshold,
                    *downwards_ratio_recip,
                    *downwards_knee_start,
                    *downwards_knee_end,
                    downwards_knee_scaling_factor,
                );
            }

            // Upwards compression should not happen when the signal is _too_ quiet as we'd only be
            // amplifying noise
            let upwards_threshold = unsafe { self.upwards_thresholds.get_unchecked(bin_idx) };
            let upwards_ratio_recip = unsafe { self.upwards_ratio_recips.get_unchecked(bin_idx) };
            let upwards_knee_start = unsafe { self.upwards_knee_starts.get_unchecked(bin_idx) };
            let upwards_knee_end = unsafe { self.upwards_knee_ends.get_unchecked(bin_idx) };
            if *upwards_ratio_recip != 1.0 && *envelope > 1e-6 {
                scale *= compress_upwards(
                    *envelope,
                    *upwards_threshold,
                    *upwards_ratio_recip,
                    *upwards_knee_start,
                    *upwards_knee_end,
                    upwards_knee_scaling_factor,
                );
            }

            *bin *= scale;
        }
    }

    /// Update the compressors if needed. This is called just before processing, and the compressors
    /// are updated in accordance to the atomic flags set on this struct.
    fn update_if_needed(&mut self, params: &SpectralCompressorParams) {
        // The threshold curve is a polynomial in log-log (decibels-octaves) space. The reuslt from
        // evaluating this needs to be converted to linear gain for the compressors.
        let intercept = params.threshold.threshold_db.value;
        // The cheeky 3 additional dB/octave attenuation is to match pink noise with the default
        // settings. When using sidechaining we explicitly don't want this because the curve should
        // be a flat offset to the sidechain input at the default settings.
        let slope = match params.threshold.mode.value() {
            ThresholdMode::Internal => params.threshold.curve_slope.value - 3.0,
            ThresholdMode::Sidechain => params.threshold.curve_slope.value,
        };
        let curve = params.threshold.curve_curve.value;
        let log2_center_freq = params.threshold.center_frequency.value.log2();

        let downwards_high_freq_ratio_rolloff =
            params.compressors.downwards.high_freq_ratio_rolloff.value;
        let upwards_high_freq_ratio_rolloff =
            params.compressors.upwards.high_freq_ratio_rolloff.value;
        let log2_nyquist_freq = self
            .log2_freqs
            .last()
            .expect("The CompressorBank has not yet been resized");

        if self
            .should_update_downwards_thresholds
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let intercept = intercept + params.compressors.downwards.threshold_offset_db.value;
            for ((log2_freq, threshold), (knee_start, knee_end)) in self
                .log2_freqs
                .iter()
                .zip(self.downwards_thresholds.iter_mut())
                .zip(
                    self.downwards_knee_starts
                        .iter_mut()
                        .zip(self.downwards_knee_ends.iter_mut()),
                )
            {
                let offset = log2_freq - log2_center_freq;
                let threshold_db = intercept + (slope * offset) + (curve * offset * offset);
                let knee_start_db =
                    threshold_db - (params.compressors.downwards.knee_width_db.value / 2.0);
                let knee_end_db =
                    threshold_db + (params.compressors.downwards.knee_width_db.value / 2.0);

                // This threshold must never reach zero as it's used in divisions to get a gain ratio
                // above the threshold
                *threshold = util::db_to_gain(threshold_db).max(f32::EPSILON);
                *knee_start = util::db_to_gain(knee_start_db).max(f32::EPSILON);
                *knee_end = util::db_to_gain(knee_end_db).max(f32::EPSILON);
            }
        }

        if self
            .should_update_upwards_thresholds
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let intercept = intercept + params.compressors.upwards.threshold_offset_db.value;
            for ((log2_freq, threshold), (knee_start, knee_end)) in self
                .log2_freqs
                .iter()
                .zip(self.upwards_thresholds.iter_mut())
                .zip(
                    self.upwards_knee_starts
                        .iter_mut()
                        .zip(self.upwards_knee_ends.iter_mut()),
                )
            {
                let offset = log2_freq - log2_center_freq;
                let threshold_db = intercept + (slope * offset) + (curve * offset * offset);
                let knee_start_db =
                    threshold_db - (params.compressors.upwards.knee_width_db.value / 2.0);
                let knee_end_db =
                    threshold_db + (params.compressors.upwards.knee_width_db.value / 2.0);

                *threshold = util::db_to_gain(threshold_db).max(f32::EPSILON);
                *knee_start = util::db_to_gain(knee_start_db).max(f32::EPSILON);
                *knee_end = util::db_to_gain(knee_end_db).max(f32::EPSILON);
            }
        }

        if self
            .should_update_downwards_ratios
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // If the high-frequency rolloff is enabled then higher frequency bins will have their
            // ratios reduced to reduce harshness. This follows the octave scale.
            let target_ratio_recip = params.compressors.downwards.ratio.value.recip();
            if downwards_high_freq_ratio_rolloff == 0.0 {
                self.downwards_ratio_recips.fill(target_ratio_recip);
            } else {
                for (log2_freq, ratio) in self
                    .log2_freqs
                    .iter()
                    .zip(self.downwards_ratio_recips.iter_mut())
                {
                    let octave_fraction = log2_freq / log2_nyquist_freq;
                    let rolloff_t = octave_fraction * downwards_high_freq_ratio_rolloff;
                    // If the octave fraction times the rolloff amount is high, then this should get
                    // closer to `high_freq_ratio_rolloff` (which is in [0, 1]).
                    *ratio = (target_ratio_recip * (1.0 - rolloff_t)) + rolloff_t;
                }
            }
        }

        if self
            .should_update_upwards_ratios
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let target_ratio_recip = params.compressors.upwards.ratio.value.recip();
            if upwards_high_freq_ratio_rolloff == 0.0 {
                self.upwards_ratio_recips.fill(target_ratio_recip);
            } else {
                for (log2_freq, ratio) in self
                    .log2_freqs
                    .iter()
                    .zip(self.upwards_ratio_recips.iter_mut())
                {
                    let octave_fraction = log2_freq / log2_nyquist_freq;
                    let rolloff_t = octave_fraction * upwards_high_freq_ratio_rolloff;
                    *ratio = (target_ratio_recip * (1.0 - rolloff_t)) + rolloff_t;
                }
            }
        }
    }
}

/// Get the knee scaling factor for converting a linear `[0, 1]` knee range into the correct curve
/// for the soft knee. This is used to blend between compression at the knee start to compression at
/// the actual threshold. For upwards compression this needs an additional square root.
fn compute_knee_scaling_factor(downwards_knee_width_db: f32) -> f32 {
    ((downwards_knee_width_db * 2.0) + 2.0).log2() - 1.0
}

/// Get the compression scaling factor for downwards compression with the supplied parameters. The
/// input signal can be multiplied by this factor to get the compressed output signal. All
/// parameters are linear gain values.
fn compress_downwards(
    envelope: f32,
    threshold: f32,
    ratio_recip: f32,
    knee_start: f32,
    knee_end: f32,
    knee_scaling_factor: f32,
) -> f32 {
    // The soft-knee option will fade in the compression curve when reaching the knee
    // start until it mtaches the hard-knee curve at the knee-end
    if envelope >= knee_end {
        // Because we're working in the linear domain, we care about the ratio between
        // the threshold and the envelope's current value. And log-space division
        // becomes linear-space exponentiation by the reciprocal, or taking the nth
        // root.
        let threshold_ratio = envelope / threshold;
        threshold_ratio.powf(ratio_recip) / threshold_ratio
    } else if envelope >= knee_start {
        // When the knee width is set to 0 dB, `downwards_knee_start ==
        // downwards_knee_end` and this branch is never hit
        let linear_knee_width = knee_end - knee_start;
        let raw_knee_t = (envelope - knee_start) / linear_knee_width;
        nih_debug_assert!((0.0..=1.0).contains(&raw_knee_t));

        // TODO: Apart from a small discontinuety in the derivative/slope at the start
        //       of the knee this equation does exactly what you'd expect it to, but it
        //       feels a bit weird. Should probably look for a cleaner way to calculate
        //       this soft knee in linear-space at some point.
        let knee_t = (1.0 - raw_knee_t).powf(knee_scaling_factor);
        nih_debug_assert!((0.0..=1.0).contains(&knee_t));

        // We'll linearly interpolate between compression at the knee start and at the
        // actual threshold based on `knee_t`
        let knee_ratio = envelope / knee_start;
        let threshold_ratio = envelope / threshold;
        (knee_t * (knee_ratio.powf(ratio_recip) / knee_ratio))
            + ((1.0 - knee_t) * (threshold_ratio.powf(ratio_recip) / threshold_ratio))
    } else {
        1.0
    }
}

/// Get the compression scaling factor for upwards compression with the supplied parameters. The
/// input signal can be multiplied by this factor to get the compressed output signal. All
/// parameters are linear gain values.
fn compress_upwards(
    envelope: f32,
    threshold: f32,
    ratio_recip: f32,
    knee_start: f32,
    knee_end: f32,
    knee_scaling_factor: f32,
) -> f32 {
    // This goes the other way around compared to the downwards compression
    if envelope <= knee_start {
        // Notice how these ratios are reversed here
        let threshold_ratio = threshold / envelope;
        threshold_ratio / threshold_ratio.powf(ratio_recip)
    } else if envelope <= knee_end {
        // When the knee width is set to 0 dB, `upwards_knee_start == upwards_knee_end`
        // and this branch is never hit
        let linear_knee_width = knee_end - knee_start;
        let raw_knee_t = (envelope - knee_start) / linear_knee_width;
        nih_debug_assert!((0.0..=1.0).contains(&raw_knee_t));

        // TODO: Some note the downwards version
        let knee_t = (1.0 - raw_knee_t).powf(knee_scaling_factor);
        nih_debug_assert!((0.0..=1.0).contains(&knee_t));

        // The ratios are again inverted here compared to the downwards version
        let knee_ratio = knee_start / envelope;
        let threshold_ratio = threshold / envelope;
        (knee_t * (knee_ratio / knee_ratio.powf(ratio_recip)))
            + ((1.0 - knee_t) * (threshold_ratio / threshold_ratio.powf(ratio_recip)))
    } else {
        1.0
    }
}
