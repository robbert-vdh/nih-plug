// Spectral Compressor: an FFT based compressor
// Copyright (C) 2021-2024 Robbert van der Helm
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

use crate::analyzer::AnalyzerData;
use crate::curve::{Curve, CurveParams};
use crate::SpectralCompressorParams;

// These are the parameter name prefixes used for the downwards and upwards compression parameters.
// The ID prefixes a re set in the `CompressorBankParams` struct.
const DOWNWARDS_NAME_PREFIX: &str = "Downwards";
const UPWARDS_NAME_PREFIX: &str = "Upwards";

/// The envelopes are initialized to the RMS value of a -24 dB sine wave to make sure extreme upwards
/// compression doesn't cause pops when switching between window sizes and when deactivating and
/// reactivating the plugin.
const ENVELOPE_INIT_VALUE: f32 = std::f32::consts::FRAC_1_SQRT_2 / 8.0;

/// The target frequency for the high frequency ratio rolloff. This is fixed to prevent Spectral
/// Compressor from getting brighter as the sample rate increases.
#[allow(unused)]
const HIGH_FREQ_RATIO_ROLLOFF_FREQUENCY: f32 = 22_050.0;
const HIGH_FREQ_RATIO_ROLLOFF_FREQUENCY_LN: f32 = 10.001068; // 22_050.0f32.ln()

/// The length of time over which the envelope followers fade back from being instant to using the
/// configured timingsafter the compressor bank has been reset.
const ENVELOPE_FOLLOWER_TIMING_FADE_MS: f32 = 150.0;

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
    /// If set, then the parameters for the downwards compression soft knee parabola should be
    /// updated on the next processing cycle. Can be set from a parameter value change listener, and
    /// is also set when calling `.reset_for_size`.
    pub should_update_downwards_knee_parabolas: Arc<AtomicBool>,
    /// The same as `should_update_downwards_knee_parabolas`, but for upwards compression.
    pub should_update_upwards_knee_parabolas: Arc<AtomicBool>,

    /// For each compressor bin, `ln(freq)` where `freq` is the frequency associated with that
    /// compressor. This is precomputed since all update functions need it.
    ln_freqs: Vec<f32>,

    /// Downwards compressor thresholds, in decibels.
    downwards_thresholds_db: Vec<f32>,
    /// The ratios for the the downwards compressors. At 1.0 the cmopressor won't do anything. If
    /// [`CompressorBankParams::high_freq_ratio_rolloff`] is set to 1.0, then this will be the same
    /// for each compressor.
    downwards_ratios: Vec<f32>,
    /// The knee is modelled as a parabola using the formula `x + a * (x + b)^2`. This is `a` in
    /// that equation. The formula is taken from the Digital Dynamic Range Compressor Design paper
    /// by Dimitrios Giannoulis et. al.
    downwards_knee_parabola_scale: Vec<f32>,
    /// `b` in the equation from `downwards_knee_parabola_scale`.
    downwards_knee_parabola_intercept: Vec<f32>,

    /// Upwards compressor thresholds, in decibels.
    upwards_thresholds_db: Vec<f32>,
    /// The same as `downwards_ratios`, but for the upwards compression.
    upwards_ratios: Vec<f32>,
    /// `downwards_knee_parabola_scale`, but for the upwards compressors.
    upwards_knee_parabola_scale: Vec<f32>,
    /// `downwards_knee_parabola_intercept`, but for the upwards compressors.
    upwards_knee_parabola_intercept: Vec<f32>,

    /// The current envelope value for this bin, in linear space. Indexed by
    /// `[channel_idx][compressor_idx]`.
    envelopes: Vec<Vec<f32>>,
    /// A scaling factor for the envelope follower timings. This is set to 0 and then slowly brought
    /// back up to 1 after after [`CompressorBank::reset()`] has been called to allow the envelope
    /// followers to settle back in.
    envelope_followers_timing_scale: f32,
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

    /// The input data for the spectrum analyzer. Stores both the spectrum analyzer values and the
    /// current gain reduction. Used to draw the spectrum analyzer and gain reduction display in the
    /// editor.
    analyzer_input_data: triple_buffer::Input<AnalyzerData>,
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
    /// value, where `x = ln(center_frequency) - ln(bin_frequency)`. In other words, this is
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
    #[name = "Sidechain Matching"]
    SidechainMatch,
    /// Compress the input signal based on the sidechain signal's activity. Can be used to
    /// spectrally duck the input, or to amplify parts of the input based on holes in the sidechain
    /// signal.
    #[id = "sidechain_compress"]
    #[name = "Sidechain Compression"]
    SidechainCompress,
}

/// Contains the compressor parameters for both the upwards and downwards compressor banks.
#[derive(Params)]
pub struct CompressorBankParams {
    #[nested(id_prefix = "upwards", group = "upwards")]
    pub upwards: Arc<CompressorParams>,
    #[nested(id_prefix = "downwards", group = "downwards")]
    pub downwards: Arc<CompressorParams>,
}

/// This struct contains the parameters for either the upward or downward compressors. The `Params`
/// trait is implemented manually to avoid copy-pasting parameters for both types of compressor.
/// Both versions will have a parameter ID and a parameter name prefix to distinguish them.
#[derive(Params)]
pub struct CompressorParams {
    /// The compression threshold relative to the target curve.
    #[id = "threshold_offset"]
    pub threshold_offset_db: FloatParam,
    /// The compression ratio. At 1.0 the compressor is disengaged.
    #[id = "ratio"]
    pub ratio: FloatParam,
    /// A `[0, 1]` scaling factor that causes the compressors for the higher registers to have lower
    /// ratios than the compressors for the lower registers. The scaling is applied logarithmically
    /// rather than linearly over the compressors. If this is set to 1.0, then the ratios will be
    /// the same for every compressor. A value of 0.5 means that at
    /// `HIGH_FREQ_RATIO_ROLLOFF_FREQUENCY` Hz, the compression ratio will be 0.5 times that as the
    /// one at 0 Hz.
    #[id = "high_freq_rolloff"]
    pub high_freq_ratio_rolloff: FloatParam,
    /// The compression knee width, in decibels.
    #[id = "knee"]
    pub knee_width_db: FloatParam,
}

impl ThresholdParams {
    /// Create a new [`ThresholdParams`] object. Changing any of the threshold parameters causes the
    /// passed compressor bank's thresholds and knee parabolas to be updated.
    pub fn new(compressor_bank: &CompressorBank) -> Self {
        let should_update_downwards_thresholds =
            compressor_bank.should_update_downwards_thresholds.clone();
        let should_update_upwards_thresholds =
            compressor_bank.should_update_upwards_thresholds.clone();
        let should_update_downwards_knee_parabolas = compressor_bank
            .should_update_downwards_knee_parabolas
            .clone();
        let should_update_upwards_knee_parabolas =
            compressor_bank.should_update_upwards_knee_parabolas.clone();
        let set_update_both_thresholds = Arc::new(move |_| {
            should_update_downwards_thresholds.store(true, Ordering::SeqCst);
            should_update_upwards_thresholds.store(true, Ordering::SeqCst);
            should_update_downwards_knee_parabolas.store(true, Ordering::SeqCst);
            should_update_upwards_knee_parabolas.store(true, Ordering::SeqCst);
        });

        ThresholdParams {
            threshold_db: FloatParam::new(
                "Global Threshold",
                -12.0,
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
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }

    /// Build [`CurveParams`] out of this set of parameters.
    pub fn curve_params(&self) -> CurveParams {
        CurveParams {
            intercept: self.threshold_db.value(),
            center_frequency: self.center_frequency.value(),
            // The cheeky 3 additional dB/octave attenuation is to match pink noise with the
            // default settings. When using sidechaining we explicitly don't want this because
            // the curve should be a flat offset to the sidechain input at the default settings.
            slope: match self.mode.value() {
                ThresholdMode::Internal => self.curve_slope.value() - 3.0,
                ThresholdMode::SidechainMatch | ThresholdMode::SidechainCompress => {
                    self.curve_slope.value()
                }
            },
            curve: self.curve_curve.value(),
        }
    }
}

impl CompressorBankParams {
    /// Create compressor bank parameter objects for both the downwards and upwards compressors of
    /// `compressor`. Changing the ratio, threshold, and knee parameters will cause the compressor
    /// to recompute its values on the next processing cycle.
    pub fn new(compressor: &CompressorBank) -> Self {
        CompressorBankParams {
            downwards: Arc::new(CompressorParams::new(
                DOWNWARDS_NAME_PREFIX,
                compressor.should_update_downwards_thresholds.clone(),
                compressor.should_update_downwards_ratios.clone(),
                compressor.should_update_downwards_knee_parabolas.clone(),
            )),
            upwards: Arc::new(CompressorParams::new(
                UPWARDS_NAME_PREFIX,
                compressor.should_update_upwards_thresholds.clone(),
                compressor.should_update_upwards_ratios.clone(),
                compressor.should_update_upwards_knee_parabolas.clone(),
            )),
        }
    }
}

impl CompressorParams {
    /// Create a new [`CompressorBankParams`] object with a prefix for all parameter names. Changing
    /// any of the threshold, ratio, or knee parameters causes the passed atomics to be updated.
    /// These should be taken from a [`CompressorBank`] so the parameters are linked to it.
    pub fn new(
        name_prefix: &str,
        should_update_thresholds: Arc<AtomicBool>,
        should_update_ratios: Arc<AtomicBool>,
        should_update_knee_parabolas: Arc<AtomicBool>,
    ) -> Self {
        let set_update_thresholds = Arc::new({
            let should_update_knee_parabolas = should_update_knee_parabolas.clone();
            move |_| {
                should_update_thresholds.store(true, Ordering::SeqCst);
                should_update_knee_parabolas.store(true, Ordering::SeqCst);
            }
        });
        let set_update_ratios = Arc::new({
            let should_update_knee_parabolas = should_update_knee_parabolas.clone();
            move |_| {
                should_update_ratios.store(true, Ordering::SeqCst);
                should_update_knee_parabolas.store(true, Ordering::SeqCst);
            }
        });
        let set_update_knee_parabolas = Arc::new(move |_| {
            should_update_knee_parabolas.store(true, Ordering::SeqCst);
        });

        CompressorParams {
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
                    max: 500.0,
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
                if name_prefix == UPWARDS_NAME_PREFIX {
                    0.75
                } else {
                    // When used subtly, no rolloff is usually better for downwards compression
                    0.0
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
            .with_callback(set_update_knee_parabolas)
            .with_unit(" dB")
            .with_step_size(0.1),
        }
    }
}

impl CompressorBank {
    /// Set up the compressor for the given channel count and maximum FFT window size. The
    /// compressors won't be initialized yet.
    pub fn new(
        analyzer_input_data: triple_buffer::Input<AnalyzerData>,
        num_channels: usize,
        max_window_size: usize,
    ) -> Self {
        let complex_buffer_len = max_window_size / 2 + 1;

        CompressorBank {
            should_update_downwards_thresholds: Arc::new(AtomicBool::new(true)),
            should_update_upwards_thresholds: Arc::new(AtomicBool::new(true)),
            should_update_downwards_ratios: Arc::new(AtomicBool::new(true)),
            should_update_upwards_ratios: Arc::new(AtomicBool::new(true)),
            should_update_downwards_knee_parabolas: Arc::new(AtomicBool::new(true)),
            should_update_upwards_knee_parabolas: Arc::new(AtomicBool::new(true)),

            ln_freqs: Vec::with_capacity(complex_buffer_len),

            downwards_thresholds_db: Vec::with_capacity(complex_buffer_len),
            downwards_ratios: Vec::with_capacity(complex_buffer_len),
            downwards_knee_parabola_scale: Vec::with_capacity(complex_buffer_len),
            downwards_knee_parabola_intercept: Vec::with_capacity(complex_buffer_len),

            upwards_thresholds_db: Vec::with_capacity(complex_buffer_len),
            upwards_ratios: Vec::with_capacity(complex_buffer_len),
            upwards_knee_parabola_scale: Vec::with_capacity(complex_buffer_len),
            upwards_knee_parabola_intercept: Vec::with_capacity(complex_buffer_len),

            envelopes: vec![Vec::with_capacity(complex_buffer_len); num_channels],
            envelope_followers_timing_scale: 0.0,
            sidechain_spectrum_magnitudes: vec![
                Vec::with_capacity(complex_buffer_len);
                num_channels
            ],
            window_size: 0,
            sample_rate: 1.0,

            analyzer_input_data,
        }
    }

    /// Change the capacities of the internal buffers to fit new parameters. Use the
    /// `.reset_for_size()` method to clear the buffers and set the current window size.
    pub fn update_capacity(&mut self, num_channels: usize, max_window_size: usize) {
        let complex_buffer_len = max_window_size / 2 + 1;

        self.ln_freqs
            .reserve_exact(complex_buffer_len.saturating_sub(self.ln_freqs.len()));

        self.downwards_thresholds_db
            .reserve_exact(complex_buffer_len.saturating_sub(self.downwards_thresholds_db.len()));
        self.downwards_ratios
            .reserve_exact(complex_buffer_len.saturating_sub(self.downwards_ratios.len()));
        self.downwards_knee_parabola_scale.reserve_exact(
            complex_buffer_len.saturating_sub(self.downwards_knee_parabola_scale.len()),
        );
        self.downwards_knee_parabola_intercept.reserve_exact(
            complex_buffer_len.saturating_sub(self.downwards_knee_parabola_intercept.len()),
        );

        self.upwards_thresholds_db
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_thresholds_db.len()));
        self.upwards_ratios
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_ratios.len()));
        self.upwards_knee_parabola_scale.reserve_exact(
            complex_buffer_len.saturating_sub(self.upwards_knee_parabola_scale.len()),
        );
        self.upwards_knee_parabola_intercept.reserve_exact(
            complex_buffer_len.saturating_sub(self.upwards_knee_parabola_intercept.len()),
        );

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
        self.ln_freqs.resize(complex_buffer_len, 0.0);
        // The first one should always stay at zero, `0.0f32.ln() == NaN`.
        for (i, ln_freq) in self.ln_freqs.iter_mut().enumerate().skip(1) {
            let freq = (i as f32 / window_size as f32) * buffer_config.sample_rate;
            *ln_freq = freq.ln();
        }

        self.downwards_thresholds_db.resize(complex_buffer_len, 1.0);
        self.downwards_ratios.resize(complex_buffer_len, 1.0);
        self.downwards_knee_parabola_scale
            .resize(complex_buffer_len, 1.0);
        self.downwards_knee_parabola_intercept
            .resize(complex_buffer_len, 1.0);

        self.upwards_thresholds_db.resize(complex_buffer_len, 1.0);
        self.upwards_ratios.resize(complex_buffer_len, 1.0);
        self.upwards_knee_parabola_scale
            .resize(complex_buffer_len, 1.0);
        self.upwards_knee_parabola_intercept
            .resize(complex_buffer_len, 1.0);

        for envelopes in self.envelopes.iter_mut() {
            envelopes.resize(complex_buffer_len, ENVELOPE_INIT_VALUE);
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
        self.should_update_downwards_knee_parabolas
            .store(true, Ordering::SeqCst);
        self.should_update_upwards_knee_parabolas
            .store(true, Ordering::SeqCst);
    }

    /// Clear out the envelope followers.
    pub fn reset(&mut self) {
        // This will make the timings instant for the first iteration after a reset and then slowly
        // fade the timings back to their intended values so the envelope followers can settle in.
        // Otherwise suspending and resetting the plugin, or changing the window size, may result in
        // some huge spikes.
        self.envelope_followers_timing_scale = 0.0;

        // Sidechain data doesn't need to be reset as it will be overwritten immediately before use
    }

    /// Apply the magnitude compression to a buffer of FFT bins. The compressors are first updated
    /// if needed. The overlap amount is needed to compute the effective sample rate. The
    /// `first_non_dc_bin` argument is used to avoid upwards compression on the DC bins, or the
    /// neighbouring bins the DC signal may have been convolved into because of the Hann window
    /// function.
    pub fn process(
        &mut self,
        buffer: &mut [Complex32],
        channel_idx: usize,
        params: &SpectralCompressorParams,
        overlap_times: usize,
        first_non_dc_bin: usize,
    ) {
        nih_debug_assert_eq!(buffer.len(), self.ln_freqs.len());

        // The gain difference/reduction amounts are accumulated in `self.analyzer_input_data`. When
        // processing the last channel, this data is divided by the channel count, the envelope
        // follower data is added, and the data is then sent to the editor so it can be displayed.
        // `analyzer_input_data` contains excess capacity so it can handle any supported window
        // size, so all operations on it are limited to the actual number of used bins.
        let num_bins = buffer.len();
        let num_channels = self.sidechain_spectrum_magnitudes.len();
        let should_update_analyzer_data = params.editor_state.is_open();
        if should_update_analyzer_data && channel_idx == 0 {
            // NOTE: This may briefly show a huge amount of accumulated data when the editor has
            //       just been opened. If this doesn't look too obvious or too jarring this is
            //       probably worth letting it be like this.
            let analyzer_input_data = self.analyzer_input_data.input_buffer();
            analyzer_input_data.gain_difference_db[..num_bins].fill(0.0);
        }

        self.update_if_needed(params);
        match params.threshold.mode.value() {
            ThresholdMode::Internal => {
                self.update_envelopes(buffer, channel_idx, params, overlap_times);
                self.compress(buffer, channel_idx, params, first_non_dc_bin)
            }
            ThresholdMode::SidechainMatch => {
                self.update_envelopes(buffer, channel_idx, params, overlap_times);
                self.compress_sidechain_match(buffer, channel_idx, params, first_non_dc_bin)
            }
            ThresholdMode::SidechainCompress => {
                // This mode uses regular compression, but the envelopes are computed from the
                // sidechain input magnitudes. These are already set in `process_sidechain`. This
                // separate envelope updating function is needed for the channel linking.
                self.update_envelopes_sidechain(channel_idx, params, overlap_times);
                self.compress(buffer, channel_idx, params, first_non_dc_bin)
            }
        };

        // When processing the last channel we can finalize the spectrum analyzer data and send it
        // to the editor for display
        if should_update_analyzer_data && channel_idx == num_channels - 1 {
            let analyzer_input_data = self.analyzer_input_data.input_buffer();

            // The editor needs to know about this too so it can draw the spectra correctly
            analyzer_input_data.curve_params = params.threshold.curve_params();
            analyzer_input_data.curve_offsets_db = (
                params.compressors.upwards.threshold_offset_db.value(),
                params.compressors.downwards.threshold_offset_db.value(),
            );
            analyzer_input_data.num_bins = num_bins;

            // The gain reduction data needs to be averaged, see above
            let channel_multiplier = (num_channels as f32).recip();
            for gain_difference_db in &mut analyzer_input_data.gain_difference_db[..num_bins] {
                *gain_difference_db *= channel_multiplier;
            }

            // The spectrum analyzer data has not yet been added
            assert!(self.envelopes.len() == num_channels);
            assert!(self.envelopes[0].len() >= num_bins);
            for (bin_idx, spectrum_data) in analyzer_input_data.envelope_followers[..num_bins]
                .iter_mut()
                .enumerate()
            {
                *spectrum_data = 0.0;
                for channel_idx in 0..num_channels {
                    // SAFETY: These bounds are already checked
                    *spectrum_data += unsafe {
                        self.envelopes
                            .get_unchecked(channel_idx)
                            .get_unchecked(bin_idx)
                    };
                }

                *spectrum_data *= channel_multiplier;
            }

            // After filling the object with data it can be sent to the editor. This happens
            // automatically when using the `.write()` interface, but since `AnalyzerData` contains
            // a lot of padding and we only use the first `num_bins` of the arrays that would be a
            // bit wasteful.
            self.analyzer_input_data.publish();
        }
    }

    /// Set the sidechain frequency spectrum magnitudes just before a [`process()`][Self::process()]
    /// call. These will be multiplied with the existing compressor thresholds and knee values to
    /// get the effective values for use with sidechaining.
    pub fn process_sidechain(&mut self, sc_buffer: &[Complex32], channel_idx: usize) {
        nih_debug_assert_eq!(sc_buffer.len(), self.ln_freqs.len());

        self.update_sidechain_spectra(sc_buffer, channel_idx);
    }

    /// Update the envelope followers based on the bin magnitudes.
    fn update_envelopes(
        &mut self,
        buffer: &[Complex32],
        channel_idx: usize,
        params: &SpectralCompressorParams,
        overlap_times: usize,
    ) {
        let effective_sample_rate =
            self.sample_rate / (self.window_size as f32 / overlap_times as f32);

        // The timings are scaled by `self.envelope_followers_timing_scale` to allow the envelope
        // followers to settle in quicker after a reset
        let attack_ms =
            params.global.compressor_attack_ms.value() * self.envelope_followers_timing_scale;
        let release_ms =
            params.global.compressor_release_ms.value() * self.envelope_followers_timing_scale;

        // This needs to gradually fade from 0.0 back to 1.0 after a reset
        if self.envelope_followers_timing_scale < 1.0 && channel_idx == self.envelopes.len() - 1 {
            let delta =
                ((ENVELOPE_FOLLOWER_TIMING_FADE_MS / 1000.0) * effective_sample_rate).recip();
            self.envelope_followers_timing_scale =
                (self.envelope_followers_timing_scale + delta).min(1.0);
        }

        // The coefficient the old envelope value is multiplied by when the current rectified sample
        // value is above the envelope's value. The 0 to 1 step response retains 36.8% of the old
        // value after the attack time has elapsed, and current value is 63.2% of the way towards 1.
        // The effective sample rate needs to compensate for the periodic nature of the STFT
        // operation. Since with a 2048 sample window and 4x overlap, you'd run this function once
        // for every 512 samples.
        let attack_old_t = if attack_ms == 0.0 {
            0.0
        } else {
            (-1.0 / (attack_ms / 1000.0 * effective_sample_rate)).exp()
        };
        let attack_new_t = 1.0 - attack_old_t;
        // The same as `attack_old_t`, but for the release phase of the envelope follower
        let release_old_t = if release_ms == 0.0 {
            0.0
        } else {
            (-1.0 / (release_ms / 1000.0 * effective_sample_rate)).exp()
        };
        let release_new_t = 1.0 - release_old_t;

        for (bin, envelope) in buffer.iter().zip(self.envelopes[channel_idx].iter_mut()) {
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

    /// The same as [`update_envelopes()`][Self::update_envelopes()], but based on the previously
    /// set sidechain bin magnitudes. This allows for channel linking.
    /// [`process_sidechain()`][Self::process_sidechain()] needs to be called for all channels
    /// before this function can be used to set the magnitude spectra.
    fn update_envelopes_sidechain(
        &mut self,
        channel_idx: usize,
        params: &SpectralCompressorParams,
        overlap_times: usize,
    ) {
        let effective_sample_rate =
            self.sample_rate / (self.window_size as f32 / overlap_times as f32);

        // The timings are scaled by `self.envelope_followers_timing_scale` to allow the envelope
        // followers to settle in quicker after a reset
        let attack_ms =
            params.global.compressor_attack_ms.value() * self.envelope_followers_timing_scale;
        let release_ms =
            params.global.compressor_release_ms.value() * self.envelope_followers_timing_scale;

        // This needs to gradually fade from 0.0 back to 1.0 after a reset
        if self.envelope_followers_timing_scale < 1.0 && channel_idx == self.envelopes.len() - 1 {
            let delta =
                ((ENVELOPE_FOLLOWER_TIMING_FADE_MS / 1000.0) * effective_sample_rate).recip();
            self.envelope_followers_timing_scale =
                (self.envelope_followers_timing_scale + delta).min(1.0);
        }

        // See `update_envelopes()`
        let attack_old_t = if attack_ms == 0.0 {
            0.0
        } else {
            (-1.0 / (attack_ms / 1000.0 * effective_sample_rate)).exp()
        };
        let attack_new_t = 1.0 - attack_old_t;
        let release_old_t = if release_ms == 0.0 {
            0.0
        } else {
            (-1.0 / (release_ms / 1000.0 * effective_sample_rate)).exp()
        };
        let release_new_t = 1.0 - release_old_t;

        // For the channel linking
        let num_channels = self.sidechain_spectrum_magnitudes.len() as f32;
        let other_channels_t = params.threshold.sc_channel_link.value() / num_channels;
        let this_channel_t = 1.0 - (other_channels_t * (num_channels - 1.0));

        for (bin_idx, envelope) in self.envelopes[channel_idx].iter_mut().enumerate() {
            // In this mode the envelopes are set based on the sidechain signal, taking channel
            // linking into account
            let sidechain_magnitude: f32 = self
                .sidechain_spectrum_magnitudes
                .iter()
                .enumerate()
                .map(|(sidechain_channel_idx, magnitudes)| {
                    let t = if sidechain_channel_idx == channel_idx {
                        this_channel_t
                    } else {
                        other_channels_t
                    };

                    unsafe { magnitudes.get_unchecked(bin_idx) * t }
                })
                .sum::<f32>();

            if *envelope > sidechain_magnitude {
                // Release stage
                *envelope = (release_old_t * *envelope) + (release_new_t * sidechain_magnitude);
            } else {
                // Attack stage
                *envelope = (attack_old_t * *envelope) + (attack_new_t * sidechain_magnitude);
            }
        }
    }

    /// Update the spectral data using the sidechain input
    fn update_sidechain_spectra(&mut self, sc_buffer: &[Complex32], channel_idx: usize) {
        nih_debug_assert!(channel_idx < self.sidechain_spectrum_magnitudes.len());

        for (bin, magnitude) in sc_buffer
            .iter()
            .zip(self.sidechain_spectrum_magnitudes[channel_idx].iter_mut())
        {
            *magnitude = bin.norm();
        }
    }

    /// Actually do the thing. [`Self::update_envelopes()`] or
    /// [`Self::update_envelopes_sidechain()`] must have been called before calling this.
    ///
    /// # Panics
    ///
    /// Panics if the buffer does not have the same length as the one that was passed to the last
    /// `resize()` call.
    fn compress(
        &mut self,
        buffer: &mut [Complex32],
        channel_idx: usize,
        params: &SpectralCompressorParams,
        first_non_dc_bin: usize,
    ) {
        // The gain reduction values are always added to the arrays stored in this object. This
        // makes it possible to visualize the gain reduction without a lot of conditionals.
        let analyzer_input_data = self.analyzer_input_data.input_buffer();

        let downwards_knee_width_db = params.compressors.downwards.knee_width_db.value();
        let upwards_knee_width_db = params.compressors.upwards.knee_width_db.value();

        assert!(analyzer_input_data.gain_difference_db.len() >= buffer.len());
        assert!(self.downwards_thresholds_db.len() == buffer.len());
        assert!(self.downwards_ratios.len() == buffer.len());
        assert!(self.downwards_knee_parabola_scale.len() == buffer.len());
        assert!(self.downwards_knee_parabola_intercept.len() == buffer.len());
        assert!(self.upwards_thresholds_db.len() == buffer.len());
        assert!(self.upwards_ratios.len() == buffer.len());
        assert!(self.upwards_knee_parabola_scale.len() == buffer.len());
        assert!(self.upwards_knee_parabola_intercept.len() == buffer.len());
        // NOTE: In the sidechain compression mode these envelopes are computed from the sidechain
        //       signal instead of the main input
        for (bin_idx, (bin, envelope)) in buffer
            .iter_mut()
            .zip(self.envelopes[channel_idx].iter())
            .enumerate()
        {
            // We'll apply the transfer curve to the envelope signal, and then scale the complex
            // `bin` by the gain difference
            let envelope_db = util::gain_to_db_fast_epsilon(*envelope);

            // SAFETY: These sizes were asserted above
            let downwards_threshold_db =
                unsafe { self.downwards_thresholds_db.get_unchecked(bin_idx) };
            let downwards_ratio = unsafe { self.downwards_ratios.get_unchecked(bin_idx) };
            let downwards_knee_parabola_scale =
                unsafe { self.downwards_knee_parabola_scale.get_unchecked(bin_idx) };
            let downwards_knee_parabola_intercept = unsafe {
                self.downwards_knee_parabola_intercept
                    .get_unchecked(bin_idx)
            };
            let downwards_compressed = compress_downwards(
                envelope_db,
                *downwards_threshold_db,
                *downwards_ratio,
                downwards_knee_width_db,
                *downwards_knee_parabola_scale,
                *downwards_knee_parabola_intercept,
            );

            // Upwards compression should not happen when the signal is _too_ quiet as we'd only be
            // amplifying noise. We also don't want to amplify DC noise and super low frequencies.
            let upwards_threshold_db = unsafe { self.upwards_thresholds_db.get_unchecked(bin_idx) };
            let upwards_ratio = unsafe { self.upwards_ratios.get_unchecked(bin_idx) };
            let upwards_knee_parabola_scale =
                unsafe { self.upwards_knee_parabola_scale.get_unchecked(bin_idx) };
            let upwards_knee_parabola_intercept =
                unsafe { self.upwards_knee_parabola_intercept.get_unchecked(bin_idx) };
            let upwards_compressed = if bin_idx >= first_non_dc_bin
                && *upwards_ratio != 1.0
                && envelope_db > util::MINUS_INFINITY_DB
            {
                compress_upwards(
                    envelope_db,
                    *upwards_threshold_db,
                    *upwards_ratio,
                    upwards_knee_width_db,
                    *upwards_knee_parabola_scale,
                    *upwards_knee_parabola_intercept,
                )
            } else {
                envelope_db
            };

            // If the comprssed output is -10 dBFS and the envelope follower was at -6 dBFS, then we
            // want to apply -4 dB of gain to the bin
            let gain_difference_db =
                downwards_compressed + upwards_compressed - (envelope_db * 2.0);
            unsafe {
                *analyzer_input_data
                    .gain_difference_db
                    .get_unchecked_mut(bin_idx) += gain_difference_db;
            }

            *bin *= util::db_to_gain_fast(gain_difference_db);
        }
    }

    /// The same as [`compress()`][Self::compress()], but multiplying the threshold and knee values
    /// with the sidechain gains.
    ///
    /// # Panics
    ///
    /// Panics if the buffer does not have the same length as the one that was passed to the last
    /// `resize()` call.
    fn compress_sidechain_match(
        &mut self,
        buffer: &mut [Complex32],
        channel_idx: usize,
        params: &SpectralCompressorParams,
        first_non_dc_bin: usize,
    ) {
        // See `compress()`
        let analyzer_input_data = self.analyzer_input_data.input_buffer();

        let downwards_knee_width_db = params.compressors.downwards.knee_width_db.value();
        let upwards_knee_width_db = params.compressors.upwards.knee_width_db.value();

        // For the channel linking
        let num_channels = self.sidechain_spectrum_magnitudes.len() as f32;
        let other_channels_t = params.threshold.sc_channel_link.value() / num_channels;
        let this_channel_t = 1.0 - (other_channels_t * (num_channels - 1.0));

        assert!(analyzer_input_data.gain_difference_db.len() >= buffer.len());
        assert!(self.sidechain_spectrum_magnitudes[channel_idx].len() == buffer.len());
        assert!(self.downwards_thresholds_db.len() == buffer.len());
        assert!(self.downwards_ratios.len() == buffer.len());
        assert!(self.upwards_thresholds_db.len() == buffer.len());
        assert!(self.upwards_ratios.len() == buffer.len());
        for (bin_idx, (bin, envelope)) in buffer
            .iter_mut()
            .zip(self.envelopes[channel_idx].iter())
            .enumerate()
        {
            let envelope_db = util::gain_to_db_fast_epsilon(*envelope);

            // The idea here is that we scale the compressor thresholds/knee values by the sidechain
            // signal, thus sort of creating a dynamic multiband compressor
            let sidechain_scale: f32 = self
                .sidechain_spectrum_magnitudes
                .iter()
                .enumerate()
                .map(|(sidechain_channel_idx, magnitudes)| {
                    let t = if sidechain_channel_idx == channel_idx {
                        this_channel_t
                    } else {
                        other_channels_t
                    };

                    unsafe { magnitudes.get_unchecked(bin_idx) * t }
                })
                .sum::<f32>()
                // The thresholds may never reach zero as they are used in divisions
                .max(f32::EPSILON);
            let sidechain_scale_db = util::gain_to_db_fast_epsilon(sidechain_scale);

            // Notice how the threshold and knee values are scaled here
            let downwards_threshold_db =
                unsafe { self.downwards_thresholds_db.get_unchecked(bin_idx) + sidechain_scale_db }
                    .max(util::MINUS_INFINITY_DB);
            let downwards_ratio = unsafe { self.downwards_ratios.get_unchecked(bin_idx) };
            // Because the thresholds are scaled based on the sidechain input, we also need to
            // recompute the knee coefficients
            let (downwards_knee_parabola_scale, downwards_knee_parabola_intercept) =
                downwards_soft_knee_coefficients(
                    downwards_threshold_db,
                    downwards_knee_width_db,
                    *downwards_ratio,
                );
            let downwards_compressed = compress_downwards(
                envelope_db,
                downwards_threshold_db,
                *downwards_ratio,
                downwards_knee_width_db,
                downwards_knee_parabola_scale,
                downwards_knee_parabola_intercept,
            );

            let upwards_threshold_db =
                unsafe { self.upwards_thresholds_db.get_unchecked(bin_idx) + sidechain_scale_db }
                    .max(util::MINUS_INFINITY_DB);
            let upwards_ratio = unsafe { self.upwards_ratios.get_unchecked(bin_idx) };
            let upwards_compressed = if bin_idx >= first_non_dc_bin
                && *upwards_ratio != 1.0
                && envelope_db > util::MINUS_INFINITY_DB
            {
                let (upwards_knee_parabola_scale, upwards_knee_parabola_intercept) =
                    upwards_soft_knee_coefficients(
                        upwards_threshold_db,
                        upwards_knee_width_db,
                        *upwards_ratio,
                    );
                compress_upwards(
                    envelope_db,
                    upwards_threshold_db,
                    *upwards_ratio,
                    upwards_knee_width_db,
                    upwards_knee_parabola_scale,
                    upwards_knee_parabola_intercept,
                )
            } else {
                envelope_db
            };

            // If the comprssed output is -10 dBFS and the envelope follower was at -6 dBFS, then we
            // want to apply -4 dB of gain to the bin
            let gain_difference_db =
                downwards_compressed + upwards_compressed - (envelope_db * 2.0);
            unsafe {
                *analyzer_input_data
                    .gain_difference_db
                    .get_unchecked_mut(bin_idx) += gain_difference_db;
            }

            *bin *= util::db_to_gain_fast(gain_difference_db);
        }
    }

    /// Update the compressors if needed. This is called just before processing, and the compressors
    /// are updated in accordance to the atomic flags set on this struct.
    fn update_if_needed(&mut self, params: &SpectralCompressorParams) {
        // The threshold curve is a polynomial in log-log (decibels-octaves) space
        let curve_params = params.threshold.curve_params();
        let curve = Curve::new(&curve_params);

        if self
            .should_update_downwards_thresholds
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let downwards_intercept = params.compressors.downwards.threshold_offset_db.value();
            for (ln_freq, threshold_db) in self
                .ln_freqs
                .iter()
                .zip(self.downwards_thresholds_db.iter_mut())
            {
                *threshold_db = curve.evaluate_ln(*ln_freq) + downwards_intercept;
            }
        }

        if self
            .should_update_upwards_thresholds
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let upwards_intercept = params.compressors.upwards.threshold_offset_db.value();
            for (ln_freq, threshold_db) in self
                .ln_freqs
                .iter()
                .zip(self.upwards_thresholds_db.iter_mut())
            {
                *threshold_db = curve.evaluate_ln(*ln_freq) + upwards_intercept;
            }
        }

        if self
            .should_update_downwards_ratios
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // If the high-frequency rolloff is enabled then higher frequency bins will have their
            // ratios reduced to reduce harshness. This follows the octave scale. It's easier to do
            // this cleanly using reciprocals.
            let target_ratio_recip = params.compressors.downwards.ratio.value().recip();
            let downwards_high_freq_ratio_rolloff =
                params.compressors.downwards.high_freq_ratio_rolloff.value();
            for (ln_freq, ratio) in self.ln_freqs.iter().zip(self.downwards_ratios.iter_mut()) {
                let octave_fraction = ln_freq / HIGH_FREQ_RATIO_ROLLOFF_FREQUENCY_LN;
                let rolloff_t = octave_fraction * downwards_high_freq_ratio_rolloff;

                // If the octave fraction times the rolloff amount is high, then this should get
                // closer to `high_freq_ratio_rolloff` (which is in [0, 1]).
                let ratio_recip = (target_ratio_recip * (1.0 - rolloff_t)) + rolloff_t;
                *ratio = ratio_recip.recip();
            }
        }

        if self
            .should_update_upwards_ratios
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let target_ratio_recip = params.compressors.upwards.ratio.value().recip();
            let upwards_high_freq_ratio_rolloff =
                params.compressors.upwards.high_freq_ratio_rolloff.value();
            for (ln_freq, ratio) in self.ln_freqs.iter().zip(self.upwards_ratios.iter_mut()) {
                let octave_fraction = ln_freq / HIGH_FREQ_RATIO_ROLLOFF_FREQUENCY_LN;
                let rolloff_t = octave_fraction * upwards_high_freq_ratio_rolloff;

                let ratio_recip = (target_ratio_recip * (1.0 - rolloff_t)) + rolloff_t;
                *ratio = ratio_recip.recip();
            }
        }

        if self
            .should_update_downwards_knee_parabolas
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let downwards_knee_width_db = params.compressors.downwards.knee_width_db.value();
            for ((ratio, threshold_db), (knee_parabola_scale, knee_parambola_intercept)) in self
                .downwards_ratios
                .iter()
                .zip(self.downwards_thresholds_db.iter())
                .zip(
                    self.downwards_knee_parabola_scale
                        .iter_mut()
                        .zip(self.downwards_knee_parabola_intercept.iter_mut()),
                )
            {
                // This is the formula from the Digital Dynamic Range Compressor Design paper by
                // Dimitrios Giannoulis et. al. These are `a` and `b` from the `x + a * (x + b)^2`
                // respectively used to compute the soft knee respectively.
                (*knee_parabola_scale, *knee_parambola_intercept) =
                    downwards_soft_knee_coefficients(
                        *threshold_db,
                        downwards_knee_width_db,
                        *ratio,
                    );
            }
        }

        if self
            .should_update_upwards_knee_parabolas
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let upwards_knee_width_db = params.compressors.upwards.knee_width_db.value();
            for ((ratio, threshold_db), (knee_parabola_scale, knee_parambola_intercept)) in self
                .upwards_ratios
                .iter()
                .zip(self.upwards_thresholds_db.iter())
                .zip(
                    self.upwards_knee_parabola_scale
                        .iter_mut()
                        .zip(self.upwards_knee_parabola_intercept.iter_mut()),
                )
            {
                // The upwards version is slightly different
                (*knee_parabola_scale, *knee_parambola_intercept) =
                    upwards_soft_knee_coefficients(*threshold_db, upwards_knee_width_db, *ratio);
            }
        }
    }
}

/// Apply downwards compression to the input with the supplied parameters. All values are in
/// decibels.
fn compress_downwards(
    input_db: f32,
    threshold_db: f32,
    ratio: f32,
    knee_width_db: f32,
    knee_parabola_scale: f32,
    knee_parabola_intercept: f32,
) -> f32 {
    // The soft-knee option will fade in the compression curve when reaching the knee start until it
    // matches the hard-knee curve at the knee-end
    let knee_start_db = threshold_db - (knee_width_db / 2.0);
    let knee_end_db = threshold_db + (knee_width_db / 2.0);
    if input_db <= knee_start_db {
        input_db
    } else if input_db <= knee_end_db {
        // See the `knee_parabola_intercept` field documentation for the full formula. The entire
        // osft knee part can be skipped if `knee_width_db == 0.0`.
        let parabola_x = input_db + knee_parabola_intercept;
        input_db + (knee_parabola_scale * parabola_x * parabola_x)
    } else {
        threshold_db + ((input_db - threshold_db) / ratio)
    }
}

/// Apply upwards compression to the input with the supplied parameters. All values are in
/// decibels.
fn compress_upwards(
    input_db: f32,
    threshold_db: f32,
    ratio: f32,
    knee_width_db: f32,
    knee_parabola_scale: f32,
    knee_parabola_intercept: f32,
) -> f32 {
    // We'll keep the terminology consistent, start is below the threshold, and end is above the
    // threshold
    let knee_start_db = threshold_db - (knee_width_db / 2.0);
    let knee_end_db = threshold_db + (knee_width_db / 2.0);

    // This goes the other way around compared to the downwards compression
    if input_db >= knee_end_db {
        input_db
    } else if input_db >= knee_start_db {
        let parabola_x = input_db + knee_parabola_intercept;
        input_db + (knee_parabola_scale * parabola_x * parabola_x)
    } else {
        threshold_db + ((input_db - threshold_db) / ratio)
    }
}

/// Compute the `(scale, intercept)`/`(a, b)` coefficients for the parabolic formula `x + a * (x +
/// b)^2`. The formula is taken from the Digital Dynamic Range Compressor Design paper by Dimitrios
/// Giannoulis et. al. This version applies to downwards compression. It can be precalculated for
/// the regular modes, since it's dependent on the threshold it has to be recomputed for every
/// sample with the sidechain matching mode.
fn downwards_soft_knee_coefficients(
    threshold_db: f32,
    knee_width_db: f32,
    ratio: f32,
) -> (f32, f32) {
    let scale = if knee_width_db != 0.0 {
        (2.0 * knee_width_db * ratio).recip() - (2.0 * knee_width_db).recip()
    } else {
        1.0
    };
    let intercept = -threshold_db + (knee_width_db / 2.0);

    (scale, intercept)
}

/// [`downwards_soft_knee_coefficients()`], but for upwards compression.
fn upwards_soft_knee_coefficients(threshold_db: f32, knee_width_db: f32, ratio: f32) -> (f32, f32) {
    // For the upwards version the scale becomes negated
    let scale = if knee_width_db != 0.0 {
        -((2.0 * knee_width_db * ratio).recip() - (2.0 * knee_width_db).recip())
    } else {
        1.0
    };
    // And the `+ (knee/2)` becomes `- (knee/2)` in the intercept
    let intercept = -threshold_db - (knee_width_db / 2.0);

    (scale, intercept)
}
