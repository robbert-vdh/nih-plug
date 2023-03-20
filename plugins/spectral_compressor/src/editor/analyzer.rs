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

        // This only covers the style rules we're actually using
        let border_width = match cx.border_width().unwrap_or_default() {
            Units::Pixels(val) => val,
            Units::Percentage(val) => bounds.w.min(bounds.h) * (val / 100.0),
            _ => 0.0,
        };
        let border_color: vg::Color = cx.border_color().cloned().unwrap_or_default().into();

        // Used for the spectrum analyzer lines
        let line_width = cx.style.dpi_factor as f32 * 1.5;
        let text_color: vg::Color = cx.font_color().cloned().unwrap_or_default().into();
        let spectrum_paint = vg::Paint::color(text_color).with_line_width(line_width);
        // Used for the gain reduction bars. Lighter and semitransparent to make it stand out
        // against the spectrum analyzer
        let bar_paint_color = vg::Color::rgbaf(0.7, 0.9, 1.0, 0.7);
        let bar_paint = vg::Paint::color(bar_paint_color);

        // The analyzer data is pulled directly from the spectral `CompressorBank`
        let mut analyzer_data = self.analyzer_data.lock().unwrap();
        let analyzer_data = analyzer_data.read();
        let nyquist = self.sample_rate.load(Ordering::Relaxed) / 2.0;
        let bin_frequency = |bin_idx: f32| (bin_idx / analyzer_data.num_bins as f32) * nyquist;

        // TODO: Draw individual bars until the difference between the next two bars becomes less
        //       than one pixel. At that point draw it as a single mesh to get rid of aliasing.
        for (bin_idx, (magnetude, gain_difference_db)) in analyzer_data
            .envelope_followers
            .iter()
            .zip(analyzer_data.gain_difference_db.iter())
            .enumerate()
        {
            // We'll show the bins from 30 Hz (to your chest) to 22 kHz, scaled logarithmically
            const LN_40_HZ: f32 = 3.4011974; // 30.0f32.ln();
            const LN_22_KHZ: f32 = 9.998797; // 22000.0f32.ln();
            const LN_FREQ_RANGE: f32 = LN_22_KHZ - LN_40_HZ;

            {
                let ln_frequency = bin_frequency(bin_idx as f32).ln();
                let t = (ln_frequency - LN_40_HZ) / LN_FREQ_RANGE;
                if t <= 0.0 || t >= 1.0 {
                    continue;
                }

                // Scale this so that 1.0/0 dBFS magnetude is at 80% of the height, the bars begin
                // at -80 dBFS, and that the scaling is linear. This is the same scaling used in
                // Diopser's spectrum analyzer.
                nih_debug_assert!(*magnetude >= 0.0);
                let magnetude_db = nih_plug::util::gain_to_db(*magnetude);
                let height = ((magnetude_db + 80.0) / 100.0).clamp(0.0, 1.0);

                let mut path = vg::Path::new();
                path.move_to(
                    bounds.x + (bounds.w * t),
                    bounds.y + (bounds.h * (1.0 - height)),
                );
                path.line_to(bounds.x + (bounds.w * t), bounds.y + bounds.h);
                canvas.stroke_path(&mut path, &spectrum_paint);
            }

            // TODO: Visualize the target curve

            // TODO: Draw this as a single mesh instead, this doesn't work.
            // Avoid drawing tiny slivers for low gain reduction values
            if gain_difference_db.abs() > 0.2 {
                // The gain reduction bars are drawn width the width of the bin, centered on the
                // bin's center frequency
                let gr_start_ln_frequency = bin_frequency(bin_idx as f32 - 0.5).ln();
                let gr_end_ln_frequency = bin_frequency(bin_idx as f32 + 0.5).ln();

                let t_start = ((gr_start_ln_frequency - LN_40_HZ) / LN_FREQ_RANGE).max(0.0);
                let t_end = ((gr_end_ln_frequency - LN_40_HZ) / LN_FREQ_RANGE).min(1.0);

                // For the bar's height we'll draw 0 dB of gain reduction as a flat line (except we
                // don't actually draw 0 dBs of GR because it looks glitchy, but that's besides the
                // point). 40 dB of gain reduction causes the bar to be drawn from the center all
                // the way to the bottom of the spectrum analyzer. 40 dB of additional gain causes
                // the bar to be drawn from the center all the way to the top of the graph.
                // NOTE: Y-coordinates go from top to bottom, hence the minus
                // TODO: The y-position should be relative to the target curve
                let t_y = ((-gain_difference_db + 40.0) / 80.0).clamp(0.0, 1.0);

                let mut path = vg::Path::new();
                path.move_to(bounds.x + (bounds.w * t_start), bounds.y + (bounds.h * 0.5));
                path.line_to(bounds.x + (bounds.w * t_end), bounds.y + (bounds.h * 0.5));
                path.line_to(bounds.x + (bounds.w * t_end), bounds.y + (bounds.h * t_y));
                path.line_to(bounds.x + (bounds.w * t_start), bounds.y + (bounds.h * t_y));
                path.close();
                canvas.fill_path(&mut path, &bar_paint);
            }
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
            path.close();
        }

        let paint = vg::Paint::color(border_color).with_line_width(border_width);
        canvas.stroke_path(&mut path, &paint);
    }
}
