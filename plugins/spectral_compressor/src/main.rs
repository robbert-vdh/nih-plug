use nih_plug::prelude::*;

use spectral_compressor::SpectralCompressor;

fn main() {
    nih_export_standalone::<SpectralCompressor>();
}
