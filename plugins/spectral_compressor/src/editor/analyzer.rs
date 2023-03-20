// Spectral Compressor: an FFT based compressor
// Copyright (C) 2021-2023 Robbert van der Helm
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

use atomic_float::AtomicF32;
use nih_plug::nih_debug_assert;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::vizia::vg;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::analyzer::AnalyzerData;

/// A very analyzer showing the envelope followers as a magnitude spectrum with an overlay for the
/// gain reduction.
pub struct Analyzer {
    analyzer_data: Arc<Mutex<triple_buffer::Output<AnalyzerData>>>,
    sample_rate: Arc<AtomicF32>,
}

impl Analyzer {
    /// Creates a new [`Analyzer`].
    pub fn new<LAnalyzerData, LRate>(
        cx: &mut Context,
        analyzer_data: LAnalyzerData,
        sample_rate: LRate,
    ) -> Handle<Self>
    where
        LAnalyzerData: Lens<Target = Arc<Mutex<triple_buffer::Output<AnalyzerData>>>>,
        LRate: Lens<Target = Arc<AtomicF32>>,
    {
        Self {
            analyzer_data: analyzer_data.get(cx),
            sample_rate: sample_rate.get(cx),
        }
        .build(
            cx,
            // This is an otherwise empty element only used for custom drawing
            |_cx| (),
        )
    }
}

impl View for Analyzer {
    fn element(&self) -> Option<&'static str> {
        Some("analyzer")
    }

    fn draw(&self, cx: &mut DrawContext, canvas: &mut Canvas) {
        let bounds = cx.bounds();
        if bounds.w == 0.0 || bounds.h == 0.0 {
            return;
        }

        // This only covers the style rules we're actually setting. Right now this doesn't support
        // backgrounds.
        let opacity = cx.opacity();
        let border_width = match cx.border_width().unwrap_or_default() {
            Units::Pixels(val) => val,
            Units::Percentage(val) => bounds.w.min(bounds.h) * (val / 100.0),
            _ => 0.0,
        };
        let mut border_color: vg::Color = cx.border_color().cloned().unwrap_or_default().into();
        border_color.set_alphaf(border_color.a * opacity);

        // The analyzer data is pulled directly from the spectral `CompressorBank`
        let mut analyzer_data = self.analyzer_data.lock().unwrap();
        let analyzer_data = analyzer_data.read();
        let nyquist = self.sample_rate.load(Ordering::Relaxed) / 2.0;

        let line_width = cx.style.dpi_factor as f32 * 1.5;
        let paint = vg::Paint::color(cx.font_color().cloned().unwrap_or_default().into())
            .with_line_width(line_width);
        for (bin_idx, (magnetude, gain_reduction_db)) in analyzer_data
            .envelope_followers
            .iter()
            .zip(analyzer_data.gain_reduction_db.iter())
            .enumerate()
        {
            // We'll show the bins from 30 Hz (to your chest) to 22 kHz, scaled logarithmically
            const LN_40_HZ: f32 = 3.4011974; // 30.0f32.ln();
            const LN_22_KHZ: f32 = 9.998797; // 22000.0f32.ln();
            const LN_FREQ_RANGE: f32 = LN_22_KHZ - LN_40_HZ;

            let frequency = (bin_idx as f32 / analyzer_data.num_bins as f32) * nyquist;
            let ln_frequency = frequency.ln();
            let t = (ln_frequency - LN_40_HZ) / LN_FREQ_RANGE;
            if t <= 0.0 || t >= 1.0 {
                continue;
            }

            // Scale this so that 1.0/0 dBFS magnetude is at 80% of the height, the bars begin at
            // -80 dBFS, and that the scaling is linear. This is the same scaling used in Diopser's
            // spectrum analyzer.
            nih_debug_assert!(*magnetude >= 0.0);
            let magnetude_db = nih_plug::util::gain_to_db(*magnetude);
            let height = ((magnetude_db + 80.0) / 100.0).clamp(0.0, 1.0);

            let mut path = vg::Path::new();
            path.move_to(
                bounds.x + (bounds.w * t),
                bounds.y + (bounds.h * (1.0 - height)),
            );
            path.line_to(bounds.x + (bounds.w * t), bounds.y + bounds.h);
            canvas.stroke_path(&mut path, &paint);

            // TODO: Visualize the gain reduction
            // TODO: Visualize the target curve
        }

        // TODO: Display the frequency range below the graph

        // Draw the border last
        let mut path = vg::Path::new();
        {
            let x = bounds.x + border_width / 2.0;
            let y = bounds.y + border_width / 2.0;
            let w = bounds.w - border_width;
            let h = bounds.h - border_width;
            path.move_to(x, y);
            path.line_to(x, y + h);
            path.line_to(x + w, y + h);
            path.line_to(x + w, y);
            path.line_to(x, y);
            path.close();
        }

        let paint = vg::Paint::color(border_color).with_line_width(border_width);
        canvas.stroke_path(&mut path, &paint);
    }
}
