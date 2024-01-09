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

use crate::curve::CurveParams;

/// The data stored used for the spectrum analyzer. This also contains the gain reduction and the
/// threshold curve (which is dynamic in the sidechain matching mode).
///
/// All of these values are raw gain/amplitude or dB values obtained directly from the DSP code. If
/// this needs to be skewed for visualization then that should be done in the editor.
///
/// This pulls the data directly from the spectral compression part of Spectral Compressor, so the
/// window size and overlap amounts are equal to the ones used by SC's main algorithm. If the
/// current window size is 2048, then only the first `2048 / 2 + 1` elements in the arrays are used.
#[derive(Debug, Clone)]
pub struct AnalyzerData {
    /// The parameters used for the global threshold curve. This is used to draw the same curve used
    /// by the compressors on the analyzer.
    pub curve_params: CurveParams,
    /// The upwards and downwards threshold offsets for the curve. These are used to draw the curve
    /// twice with some distance between them if either is non-zero.
    pub curve_offsets_db: (f32, f32),

    /// The number of used bins. This is part of the `AnalyzerData` since recomputing it in the
    /// editor could result in a race condition.
    pub num_bins: usize,
    /// The amplitudes of all frequency bins in a windowed FFT of Spectral Compressor's output. Also
    /// includes the DC offset bin which we don't draw, just to make this a bit less confusing.
    ///
    /// This data is taken directly from the envelope followers, so it has the same rise and fall
    /// time as what is used by the compressors.
    pub envelope_followers: [f32; crate::MAX_WINDOW_SIZE / 2 + 1],
    /// The gain different applied to each band, in decibels. Alternatively, the negative gain
    /// reduction. Positive values mean that a band becomes louder, and negative values mean a band
    /// got attenuated. Does not (and should not) factor in the output gain.
    pub gain_difference_db: [f32; crate::MAX_WINDOW_SIZE / 2 + 1],
    // TODO: Include the threshold curve. Decide on whether to only visualizer the 'global'
    //       threshold curve or to also show the individual upwards/downwards thresholds. Or omit
    //       this and implement it in a nicer way for the premium Spectral Compressor.
}

impl Default for AnalyzerData {
    fn default() -> Self {
        Self {
            curve_params: CurveParams::default(),
            curve_offsets_db: (0.0, 0.0),
            num_bins: 0,
            envelope_followers: [0.0; crate::MAX_WINDOW_SIZE / 2 + 1],
            gain_difference_db: [0.0; crate::MAX_WINDOW_SIZE / 2 + 1],
        }
    }
}
