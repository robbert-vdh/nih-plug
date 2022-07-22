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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nih_plug::prelude::*;

/// Type alias for the compressor parameters. These two are split up so the parameter list/tree
/// looks a bit nicer.
pub type CompressorParams<'a> = (&'a ThresholdParams, &'a CompressorBankParams);

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
    /// Upwards compressor thresholds, in linear space.
    upwards_thresholds: Vec<f32>,
    /// Downwards compressor ratios. At 1.0 the cmopressor won't do anything. If
    /// [`CompressorBankParams::high_freq_ratio_rolloff`] is set to 1.0, then this will be the same
    /// for each compressor.
    downwards_ratios: Vec<f32>,
    /// Upwards compressor ratios. At 1.0 the cmopressor won't do anything. If
    /// [`CompressorBankParams::high_freq_ratio_rolloff`] is set to 1.0, then this will be the same
    /// for each compressor.
    upwards_ratios: Vec<f32>,

    /// The current envelope value for this bin, in linear space. Indexed by
    /// `[channel_idx][compressor_idx]`.
    envelopes: Vec<Vec<f32>>,
    // TODO: Parameters for the envelope followers so we can actuall ydo soemthing useful.
}

#[derive(Params)]
pub struct ThresholdParams {
    // TODO: Sidechaining
    /// The center frqeuency for the target curve when sidechaining is not enabled. The curve is a
    /// polynomial `threshold_db + curve_slope*x + curve_curve*(x^2)` that evaluates to a decibel
    /// value, where `x = log2(center_frequency) - log2(bin_frequency)`. In other words, this is
    /// evaluated in the log/log domain for decibels and octaves.
    #[id = "thresh_center_freq"]
    center_frequency: FloatParam,
    /// The compressor threshold at the center frequency. When sidechaining is enabled, the input
    /// signal is gained by the inverse of this value. This replaces the input gain in the original
    /// Spectral Compressor. In the polynomial above, this is the intercept.
    #[id = "input_db"]
    threshold_db: FloatParam,
    /// The slope for the curve, in the log/log domain. See the polynomial above.
    #[id = "thresh_curve_slope"]
    curve_slope: FloatParam,
    /// The, uh, 'curve' for the curve, in the logarithmic domain. This is the third coefficient in
    /// the quadratic polynomial and controls the parabolic behavior. Positive values turn the curve
    /// into a v-shaped curve, while negative values attenuate everything outside of the center
    /// frequency. See the polynomial above.
    #[id = "thresh_curve_curve"]
    curve_curve: FloatParam,
}

#[derive(Params)]
pub struct CompressorBankParams {
    // TODO: Target curve options
    /// The downwards compression threshold relative to the target curve.
    #[id = "thresh_down_off"]
    downwards_threshold_offset_db: FloatParam,
    /// The upwards compression threshold relative to the target curve.
    #[id = "thresh_up_off"]
    upwards_threshold_offset_db: FloatParam,

    /// A `[0, 1]` scaling factor that causes the compressors for the higher registers to have lower
    /// ratios than the compressors for the lower registers. The scaling is applied logarithmically
    /// rather than linearly over the compressors.
    ///
    /// TODO: Decide on whether or not this should only apply on upwards ratios, or if we may need
    ///       separate controls for both
    #[id = "ratio_hi_freq_rolloff"]
    high_freq_ratio_rolloff: FloatParam,
    /// The downwards compression ratio. At 1.0 the downwards compressor is disengaged.
    #[id = "ratio_down"]
    downwards_ratio: FloatParam,
    /// The upwards compression ratio. At 1.0 the upwards compressor is disengaged.
    #[id = "ratio_up"]
    upwards_ratio: FloatParam,

    /// The downwards compression knee width, in decibels.
    #[id = "knee_down_off"]
    downwards_knee_width_db: FloatParam,
    /// The upwards compression knee width, in decibels.
    #[id = "knee_up_off"]
    upwards_knee_width_db: FloatParam,

    /// The compressor's attack time in milliseconds. Controls both upwards and downwards
    /// compression.
    #[id = "attack"]
    compressor_attack_ms: FloatParam,
    /// The compressor's release time in milliseconds. Controls both upwards and downwards
    /// compression.
    #[id = "release"]
    compressor_release_ms: FloatParam,
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
            center_frequency: FloatParam::new(
                "Threshold Center",
                500.0,
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
            // (octaves/decibels). The threshold is the intercept.
            threshold_db: FloatParam::new(
                "Global Threshold",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_callback(set_update_both_thresholds.clone())
            .with_unit(" dB")
            .with_step_size(0.1),
            curve_slope: FloatParam::new(
                "Threshold Slope",
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_callback(set_update_both_thresholds.clone())
            .with_unit(" dB/oct")
            .with_step_size(0.1),
            curve_curve: FloatParam::new(
                "Threshold Curve",
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_callback(set_update_both_thresholds)
            .with_unit(" dB/oct²")
            .with_step_size(0.1),
        }
    }
}

impl CompressorBankParams {
    /// Create a new [`CompressorBankParams`] object. Changing any of the threshold or ratio
    /// parameters causes the passed compressor bank's parameters to be updated.
    pub fn new(compressor_bank: &CompressorBank) -> Self {
        let should_update_downwards_thresholds =
            compressor_bank.should_update_downwards_thresholds.clone();
        let set_update_downwards_thresholds =
            Arc::new(move |_| should_update_downwards_thresholds.store(true, Ordering::SeqCst));
        let should_update_upwards_thresholds =
            compressor_bank.should_update_upwards_thresholds.clone();
        let set_update_upwards_thresholds =
            Arc::new(move |_| should_update_upwards_thresholds.store(true, Ordering::SeqCst));
        let should_update_downwards_ratios = compressor_bank.should_update_downwards_ratios.clone();
        let set_update_downwards_ratios =
            Arc::new(move |_| should_update_downwards_ratios.store(true, Ordering::SeqCst));
        let should_update_upwards_ratios = compressor_bank.should_update_upwards_ratios.clone();
        let set_update_upwards_ratios =
            Arc::new(move |_| should_update_upwards_ratios.store(true, Ordering::SeqCst));

        let should_update_downwards_ratios = compressor_bank.should_update_downwards_ratios.clone();
        let should_update_upwards_ratios = compressor_bank.should_update_upwards_ratios.clone();
        let set_update_both_ratios = Arc::new(move |_| {
            should_update_downwards_ratios.store(true, Ordering::SeqCst);
            should_update_upwards_ratios.store(true, Ordering::SeqCst);
        });

        CompressorBankParams {
            // TODO: Set nicer default values for these things
            // As explained above, these offsets are relative to the target curve
            downwards_threshold_offset_db: FloatParam::new(
                "Downwards Offset",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_callback(set_update_downwards_thresholds)
            .with_unit(" dB")
            .with_step_size(0.1),
            upwards_threshold_offset_db: FloatParam::new(
                "Upwards Offset",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_callback(set_update_upwards_thresholds)
            .with_unit(" dB")
            .with_step_size(0.1),

            high_freq_ratio_rolloff: FloatParam::new(
                "High-freq Ratio Rolloff",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_callback(set_update_both_ratios)
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            downwards_ratio: FloatParam::new(
                "Downwards Ratio",
                1.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 300.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_callback(set_update_downwards_ratios)
            .with_step_size(0.1)
            .with_value_to_string(formatters::v2s_compression_ratio(1))
            .with_string_to_value(formatters::s2v_compression_ratio()),
            upwards_ratio: FloatParam::new(
                "Upwards Ratio",
                1.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 300.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_callback(set_update_upwards_ratios)
            .with_step_size(0.1)
            .with_value_to_string(formatters::v2s_compression_ratio(1))
            .with_string_to_value(formatters::s2v_compression_ratio()),

            downwards_knee_width_db: FloatParam::new(
                "Downwards Knee",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 36.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
            upwards_knee_width_db: FloatParam::new(
                "Upwards Knee",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 36.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),

            compressor_attack_ms: FloatParam::new(
                "Attack",
                150.0,
                FloatRange::Skewed {
                    // TODO: Make sure to handle 0 attack and release times in the compressor
                    min: 0.0,
                    max: 10_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.1),
            compressor_release_ms: FloatParam::new(
                "Release",
                300.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 10_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
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
            upwards_thresholds: Vec::with_capacity(complex_buffer_len),
            downwards_ratios: Vec::with_capacity(complex_buffer_len),
            upwards_ratios: Vec::with_capacity(complex_buffer_len),

            envelopes: vec![Vec::with_capacity(complex_buffer_len); num_channels],
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
        self.upwards_thresholds
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_thresholds.len()));
        self.downwards_ratios
            .reserve_exact(complex_buffer_len.saturating_sub(self.downwards_ratios.len()));
        self.upwards_ratios
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_ratios.len()));

        self.envelopes.resize_with(num_channels, Vec::new);
        for envelopes in self.envelopes.iter_mut() {
            envelopes.reserve_exact(complex_buffer_len.saturating_sub(envelopes.len()));
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
        for (i, log2_freq) in self.log2_freqs.iter_mut().enumerate() {
            let freq = (i as f32 / window_size as f32) * buffer_config.sample_rate;
            *log2_freq = freq.log2();
        }

        self.downwards_thresholds.resize(complex_buffer_len, 1.0);
        self.upwards_thresholds.resize(complex_buffer_len, 1.0);
        self.downwards_ratios.resize(complex_buffer_len, 1.0);
        self.upwards_ratios.resize(complex_buffer_len, 1.0);

        for envelopes in self.envelopes.iter_mut() {
            envelopes.resize(complex_buffer_len, 0.0);
        }

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
    }

    /// Update the compressors if needed. This is called just before processing, and the compressors
    /// are updated in accordance to the atomic flags set on this struct.
    fn update_if_needed(&mut self, (threshold, compressor): CompressorParams) {
        // The threshold curve is a polynomial in log-log (decibels-octaves) space. The reuslt from
        // evaluating this needs to be converted to linear gain for the compressors.
        let intercept = threshold.threshold_db.value;
        // The cheeky 3 additional dB/octave attenuation is to match pink noise with the default
        // settings
        let slope = threshold.curve_slope.value - 3.0;
        let curve = threshold.curve_curve.value;
        let log2_center_freq = threshold.center_frequency.value.log2();

        let high_freq_ratio_rolloff = compressor.high_freq_ratio_rolloff.value;
        let log2_nyquist_freq = self
            .log2_freqs
            .last()
            .expect("The CompressorBank has not yet been resized");

        if self
            .should_update_downwards_thresholds
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let intercept = intercept + compressor.downwards_threshold_offset_db.value;
            for (log2_freq, threshold) in self
                .log2_freqs
                .iter()
                .zip(self.downwards_thresholds.iter_mut())
            {
                let offset = log2_center_freq - log2_freq;
                let threshold_db = intercept + (slope * offset) + (curve * offset * offset);
                *threshold = util::db_to_gain(threshold_db)
            }
        }

        if self
            .should_update_upwards_thresholds
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let intercept = intercept + compressor.upwards_threshold_offset_db.value;
            for (log2_freq, threshold) in self
                .log2_freqs
                .iter()
                .zip(self.upwards_thresholds.iter_mut())
            {
                let offset = log2_center_freq - log2_freq;
                let threshold_db = intercept + (slope * offset) + (curve * offset * offset);
                *threshold = util::db_to_gain(threshold_db)
            }
        }

        if self
            .should_update_downwards_ratios
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // If the high-frequency rolloff is enabled then higher frequency bins will have their
            // ratios reduced to reduce harshness. This follows the octave scale.
            let target_ratio = compressor.downwards_ratio.value;
            if high_freq_ratio_rolloff == 1.0 {
                self.downwards_ratios.fill(target_ratio);
            } else {
                for (log2_freq, ratio) in
                    self.log2_freqs.iter().zip(self.downwards_ratios.iter_mut())
                {
                    // This is scaled by octaves since we're calculating this in log space
                    let octave_fraction = log2_freq / log2_nyquist_freq;
                    *ratio = target_ratio * (1.0 - (octave_fraction * high_freq_ratio_rolloff));
                }
            }
        }

        if self
            .should_update_upwards_ratios
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let target_ratio = compressor.upwards_ratio.value;
            if high_freq_ratio_rolloff == 1.0 {
                self.upwards_ratios.fill(target_ratio);
            } else {
                for (log2_freq, ratio) in self.log2_freqs.iter().zip(self.upwards_ratios.iter_mut())
                {
                    let octave_fraction = log2_freq / log2_nyquist_freq;
                    *ratio = target_ratio * (1.0 - (octave_fraction * high_freq_ratio_rolloff));
                }
            }
        }
    }
}
