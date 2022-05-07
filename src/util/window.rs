//! Windowing functions, useful in conjuction with [`StftHelper`][super::StftHelper].

use std::f32;

/// A Hann window function.
///
/// <https://en.wikipedia.org/wiki/Hann_function>
pub fn hann(size: usize) -> Vec<f32> {
    let mut window = vec![0.0; size];
    hann_in_place(&mut window);

    window
}

/// The same as [`hann()`], but filling an existing slice instead.
pub fn hann_in_place(window: &mut [f32]) {
    let size = window.len();

    // We want to scale `[0, size - 1]` to `[0, pi]`.
    // XXX: The `sin^2()` version results in weird rounding errors that cause spectral leakeage
    let scale = (size as f32 - 1.0).recip() * f32::consts::TAU;
    for (i, sample) in window.iter_mut().enumerate() {
        let cos = (i as f32 * scale).cos();
        *sample = 0.5 - (0.5 * cos)
    }
}

/// Multiply a buffer with a window function.
#[inline]
pub fn multiply_with_window(buffer: &mut [f32], window_function: &[f32]) {
    // TODO: ALso use SIMD here if available
    for (sample, window_sample) in buffer.iter_mut().zip(window_function) {
        *sample *= window_sample;
    }
}
