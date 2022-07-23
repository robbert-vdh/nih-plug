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
    /// The reciprocals of the downwards compressor ratios. At 1.0 the cmopressor won't do anything.
    /// If [`CompressorBankParams::high_freq_ratio_rolloff`] is set to 1.0, then this will be the
    /// same for each compressor. We're doing the compression in linear space to avoid a logarithm,
    /// so the division by the ratio becomes an nth-root, or exponentation by the reciprocal of the
    /// ratio.
    downwards_ratio_recips: Vec<f32>,
    /// Upwards compressor thresholds, in linear space.
    upwards_thresholds: Vec<f32>,
    /// The same as `downwards_ratio_recipss`, but for the upwards compression.
    upwards_ratio_recips: Vec<f32>,

    /// The current envelope value for this bin, in linear space. Indexed by
    /// `[channel_idx][compressor_idx]`.
    envelopes: Vec<Vec<f32>>,
    /// The window size this compressor bank was configured for. This is used to compute the
    /// coefficients for the envelope followers in the process function.
    window_size: usize,
    /// The sample rate this compressor bank was configured for. This is used to compute the
    /// coefficients for the envelope followers in the process function.
    sample_rate: f32,
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
    /// The slope for the curve, in the log/log domain. See the polynomial above.
    #[id = "thresh_curve_slope"]
    curve_slope: FloatParam,
    /// The, uh, 'curve' for the curve, in the logarithmic domain. This is the third coefficient in
    /// the quadratic polynomial and controls the parabolic behavior. Positive values turn the curve
    /// into a v-shaped curve, while negative values attenuate everything outside of the center
    /// frequency. See the polynomial above.
    #[id = "thresh_curve_curve"]
    curve_curve: FloatParam,
    /// The compressor threshold at the center frequency. When sidechaining is enabled, the input
    /// signal is gained by the inverse of this value. This replaces the input gain in the original
    /// Spectral Compressor. In the polynomial above, this is the intercept.
    #[id = "input_db"]
    threshold_db: FloatParam,
}

/// Contains the compressor parameters for both the upwards and downwards compressor banks.
#[derive(Params)]
pub struct CompressorBankParams {
    #[nested = "downwards"]
    pub downwards: CompressorParams,
    #[nested = "upwards"]
    pub upwards: CompressorParams,
}

/// This struct contains the parameters for either the upward or downward compressors. The `Params`
/// trait is implemented manually to avoid copy-pasting parameters for both types of compressor.
/// Both versions will have a parameter ID and a parameter name prefix to distinguish them.
pub struct CompressorParams {
    /// The prefix to use in the `.param_map()` function so the upwards and downwards compressors
    /// get unique parameter IDs.
    param_id_prefix: &'static str,

    /// The compression threshold relative to the target curve.
    threshold_offset_db: FloatParam,
    /// The compression ratio. At 1.0 the compressor is disengaged.
    ratio: FloatParam,
    /// The compression knee width, in decibels.
    knee_width_db: FloatParam,

    /// A `[0, 1]` scaling factor that causes the compressors for the higher registers to have lower
    /// ratios than the compressors for the lower registers. The scaling is applied logarithmically
    /// rather than linearly over the compressors. If this is set to 1.0, then the ratios will be
    /// the same for every compressor.
    high_freq_ratio_rolloff: FloatParam,
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
                format!("{prefix}knee"),
                self.knee_width_db.as_ptr(),
                String::new(),
            ),
            (
                format!("{prefix}high_freq_rolloff"),
                self.high_freq_ratio_rolloff.as_ptr(),
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
            // (octaves/decibels). The global threshold is the intercept.
            curve_slope: FloatParam::new(
                "Threshold Slope",
                0.0,
                FloatRange::Linear {
                    min: -36.0,
                    max: 36.0,
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
            .with_callback(set_update_both_thresholds.clone())
            .with_unit(" dB/oct²")
            .with_step_size(0.1),
            threshold_db: FloatParam::new(
                "Global Threshold",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_callback(set_update_both_thresholds)
            .with_unit(" dB")
            .with_step_size(0.1),
        }
    }
}

impl CompressorBankParams {
    /// Create compressor bank parameter objects for both the downwards and upwards compressors of
    /// `compressor`. Changing the ratio and threshold parameters will cause the compressor to
    /// recompute its values on the next processing cycle.
    pub fn new(compressor: &CompressorBank) -> Self {
        CompressorBankParams {
            downwards: CompressorParams::new(
                "downwards_",
                "Downwards",
                compressor.should_update_downwards_thresholds.clone(),
                compressor.should_update_downwards_ratios.clone(),
            ),
            upwards: CompressorParams::new(
                "upwards_",
                "Upwards",
                compressor.should_update_upwards_thresholds.clone(),
                compressor.should_update_upwards_ratios.clone(),
            ),
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
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_callback(set_update_ratios)
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            knee_width_db: FloatParam::new(
                format!("{name_prefix} Knee"),
                0.0,
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
            downwards_ratio_recips: Vec::with_capacity(complex_buffer_len),
            upwards_thresholds: Vec::with_capacity(complex_buffer_len),
            upwards_ratio_recips: Vec::with_capacity(complex_buffer_len),

            envelopes: vec![Vec::with_capacity(complex_buffer_len); num_channels],
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
        self.upwards_thresholds
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_thresholds.len()));
        self.upwards_ratio_recips
            .reserve_exact(complex_buffer_len.saturating_sub(self.upwards_ratio_recips.len()));

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
        self.downwards_ratio_recips.resize(complex_buffer_len, 1.0);
        self.upwards_thresholds.resize(complex_buffer_len, 1.0);
        self.upwards_ratio_recips.resize(complex_buffer_len, 1.0);

        for envelopes in self.envelopes.iter_mut() {
            envelopes.resize(complex_buffer_len, 0.0);
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
        assert_eq!(buffer.len(), self.log2_freqs.len());

        self.update_if_needed(params);
        self.update_envelopes(buffer, channel_idx, params, overlap_times, skip_bins_below);
        self.compress(buffer, channel_idx, params, skip_bins_below);
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

    /// Actually do the thing. [`Self::update_envelopes()`] must have been called before calling
    /// this.
    fn compress(
        &self,
        buffer: &mut [Complex32],
        channel_idx: usize,
        params: &SpectralCompressorParams,
        skip_bins_below: usize,
    ) {
        // Well I'm not sure at all why this scaling works, but it does. With higher knee
        // bandwidths, the middle values needs to be pushed more towards the post-knee threshold
        // than with lower knee values.
        let downwards_knee_scaling_factor =
            ((params.compressors.downwards.knee_width_db.value * 2.0) + 2.0).log2() - 1.0;
        let upwards_knee_scaling_factor =
            ((params.compressors.upwards.knee_width_db.value * 2.0) + 2.0).log2() - 1.0;

        // Is this what they mean by zip and and ship it?
        let downwards_values = self
            .downwards_thresholds
            .iter()
            .zip(self.downwards_ratio_recips.iter());
        let upwards_values = self
            .upwards_thresholds
            .iter()
            .zip(self.upwards_ratio_recips.iter());
        for (
            ((bin, envelope), (downwards_threshold, downwards_ratio_recip)),
            (upwards_threshold, upwards_ratio_recip),
        ) in buffer
            .iter_mut()
            .zip(self.envelopes[channel_idx].iter())
            .zip(downwards_values)
            .zip(upwards_values)
            .skip(skip_bins_below)
        {
            // This works by computing a scaling factor, and then scaling the bin magnitudes by that.
            let mut scale = 1.0;

            // All compression happens in the linear domain to save a logarithm
            if *downwards_ratio_recip != 1.0 {
                // TODO: We need the knee starts and ends on this struct
                // TODO: As mentioned above, soft knee, replace the threshold
                if envelope > downwards_threshold {
                    // Because we're working in the linear domain, we care about the ratio between
                    // the threshold and the envelope's current value. And log-space division
                    // becomes linear-space exponentiation by the reciprocal, or taking the nth
                    // root.
                    let threshold_ratio = *envelope / *downwards_threshold;
                    scale /= threshold_ratio / threshold_ratio.powf(*downwards_ratio_recip);
                }
            }

            // TODO: More stuff
            // TODO: Upwards compression

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
        // settings
        let slope = params.threshold.curve_slope.value - 3.0;
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
            for (log2_freq, threshold) in self
                .log2_freqs
                .iter()
                .zip(self.downwards_thresholds.iter_mut())
            {
                let offset = log2_freq - log2_center_freq;
                let threshold_db = intercept + (slope * offset) + (curve * offset * offset);
                // This threshold may never reach zero as it's used in divisions to get a gain ratio
                // above the threshold
                *threshold = util::db_to_gain(threshold_db).max(f32::EPSILON);
            }
        }

        if self
            .should_update_upwards_thresholds
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let intercept = intercept + params.compressors.upwards.threshold_offset_db.value;
            for (log2_freq, threshold) in self
                .log2_freqs
                .iter()
                .zip(self.upwards_thresholds.iter_mut())
            {
                let offset = log2_freq - log2_center_freq;
                let threshold_db = intercept + (slope * offset) + (curve * offset * offset);
                *threshold = util::db_to_gain(threshold_db).max(f32::EPSILON);
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
