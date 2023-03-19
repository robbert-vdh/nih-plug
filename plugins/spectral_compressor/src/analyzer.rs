/// The data stored used for the spectrum analyzer. This also contains the gain reduction and the
/// threshold curve (which is dynamic in the sidechain matching mode).
///
/// All of these values are raw gain/amplitude or dB values obtained directly from the DSP code. If
/// this needs to be skewed for visualization then that should be done in the editor.
///
/// This pulls the data directly from the spectral compression part of Spectral Compressor, so the
/// window size and overlap amounts are equal to the ones used by SC's main algorithm. If the
/// current window size is 2048, then only the first `2048 / 2 + 1` elements in the arrays are used.
pub struct AnalyzerData {
    /// The amplitudes of all frequency bins in a windowed FFT of Spectral Compressor's output. Also
    /// includes the DC offset bin which we don't draw, just to make this a bit less confusing.
    pub spectrum: [f32; crate::MAX_WINDOW_SIZE / 2 + 1],
    /// The gain reduction applied to each band, in decibels. Positive values mean that a band
    /// becomes louder, and negative values mean a band got attenuated. Does not (and should not)
    /// factor in the output gain.
    pub gain_reduction_db: [f32; crate::MAX_WINDOW_SIZE / 2 + 1],
    // TODO: Include the threshold curve. Decide on whether to only visualizer the 'global'
    //       threshold curve or to also show the individual upwards/downwards thresholds. Or omit
    //       this and implement it in a nicer way for the premium Spectral Compressor.
}
