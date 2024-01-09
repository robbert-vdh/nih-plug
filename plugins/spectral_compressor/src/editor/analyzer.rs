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

use atomic_float::AtomicF32;
use nih_plug::nih_debug_assert;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::vizia::vg;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::analyzer::AnalyzerData;
use crate::curve::Curve;

// We'll show the bins from 30 Hz (to your chest) to 22 kHz, scaled logarithmically
#[allow(unused)]
const FREQ_RANGE_START_HZ: f32 = 30.0;
#[allow(unused)]
const FREQ_RANGE_END_HZ: f32 = 22_000.0;
const LN_FREQ_RANGE_START_HZ: f32 = 3.4011974; // 30.0f32.ln();
const LN_FREQ_RANGE_END_HZ: f32 = 9.998797; // 22_000.0f32.ln();
const LN_FREQ_RANGE: f32 = LN_FREQ_RANGE_END_HZ - LN_FREQ_RANGE_START_HZ;

/// The color used for drawing the overlay. Currently not configurable using the style sheet (that
/// would be possible by moving this to a dedicated view and overlaying that).
///
/// # Notes
///
/// This is drawn using some blending options that make it interact differently with darker
/// backgrounds.
const GR_BAR_OVERLAY_COLOR: vg::Color = vg::Color::rgbaf(0.85, 0.95, 1.0, 0.8);

/// The color used for drawing the downwards compression threshold curve. Looks somewhat similar to
/// `GR_BAR_OVERLAY_COLOR` when factoring in the blending.
const DOWNWARDS_THRESHOLD_CURVE_COLOR: vg::Color = vg::Color::rgbaf(0.45, 0.55, 0.6, 0.9);
/// The color used for drawing the upwards compression threshold curve. Slightly color to make to
/// make the output look less confusing.
const UPWARDS_THRESHOLD_CURVE_COLOR: vg::Color = vg::Color::rgbaf(0.55, 0.70, 0.65, 0.9);

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

        // The analyzer data is pulled directly from the spectral `CompressorBank`
        let mut analyzer_data = self.analyzer_data.lock().unwrap();
        let analyzer_data = analyzer_data.read();
        let nyquist = self.sample_rate.load(Ordering::Relaxed) / 2.0;

        draw_spectrum(cx, canvas, analyzer_data, nyquist);
        draw_threshold_curve(cx, canvas, analyzer_data);
        draw_gain_reduction(cx, canvas, analyzer_data, nyquist);
        // TODO: Display the frequency range below the graph

        // Draw the border last
        let border_width = cx.border_width();
        let border_color: vg::Color = cx.border_color().into();

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
        canvas.stroke_path(&path, &paint);
    }
}

/// Compute an unclamped value based on a decibel value -80 and is mapped to 0, +20 is mapped to 1,
/// and all other values are linearly interpolated from there
#[inline]
fn db_to_unclamped_t(db_value: f32) -> f32 {
    (db_value + 80.0) / 100.0
}

/// Draw the spectrum analyzer part of the analyzer. These are drawn as vertical bars until the
/// spacing between the bars becomes less the line width, at which point it's drawn as a solid mesh
/// instead.
fn draw_spectrum(
    cx: &mut DrawContext,
    canvas: &mut Canvas,
    analyzer_data: &AnalyzerData,
    nyquist_hz: f32,
) {
    let bounds = cx.bounds();

    let line_width = cx.scale_factor() * 1.5;
    let text_color: vg::Color = cx.font_color().into();
    // This is used to draw the individual bars
    let bars_paint = vg::Paint::color(text_color).with_line_width(line_width);
    // And this color is used to draw the mesh part of the spectrum. We'll create a gradient paint
    // that fades from this to `text_color` when we know the mesh's x-coordinates.
    let mut lighter_text_color = text_color;
    lighter_text_color.r = (lighter_text_color.r + 0.25) / 1.25;
    lighter_text_color.g = (lighter_text_color.g + 0.25) / 1.25;
    lighter_text_color.b = (lighter_text_color.b + 0.25) / 1.25;

    // The frequency belonging to a bin in Hz
    let bin_frequency = |bin_idx: f32| (bin_idx / analyzer_data.num_bins as f32) * nyquist_hz;
    // A `[0, 1]` value indicating at which relative x-coordinate a bin should be drawn at
    let bin_t =
        |bin_idx: f32| (bin_frequency(bin_idx).ln() - LN_FREQ_RANGE_START_HZ) / LN_FREQ_RANGE;
    // Converts a linear magnitude value in to a `[0, 1]` value where 0 is -80 dB or lower, and 1 is
    // +20 dB or higher.
    let magnitude_height = |magnitude: f32| {
        nih_debug_assert!(magnitude >= 0.0);
        let magnitude_db = nih_plug::util::gain_to_db(magnitude);
        db_to_unclamped_t(magnitude_db).clamp(0.0, 1.0)
    };

    // The first part of this drawing routing is simple. Individual bins are drawn as bars until the
    // distance between the bars approaches `mesh_start_delta_threshold`. After that the rest is
    // drawn as a solid mesh.
    let mesh_start_delta_threshold = line_width + 0.5;
    let mut mesh_bin_start_idx = analyzer_data.num_bins;
    let mut previous_physical_x_coord = bounds.x - 2.0;

    let mut bars_path = vg::Path::new();
    for (bin_idx, magnitude) in analyzer_data
        .envelope_followers
        .iter()
        .enumerate()
        .take(analyzer_data.num_bins)
    {
        let t = bin_t(bin_idx as f32);
        if t <= 0.0 || t >= 1.0 {
            continue;
        }

        let physical_x_coord = bounds.x + (bounds.w * t);
        if physical_x_coord - previous_physical_x_coord < mesh_start_delta_threshold {
            // NOTE: We'll draw this one bar earlier because we're not stroking the solid mesh part,
            //       and otherwise there would be a weird looking gap at the left side
            mesh_bin_start_idx = bin_idx.saturating_sub(1);
            previous_physical_x_coord = physical_x_coord;
            break;
        }

        // Scale this so that 1.0/0 dBFS magnitude is at 80% of the height, the bars begin
        // at -80 dBFS, and that the scaling is linear. This is the same scaling used in
        // Diopser's spectrum analyzer.
        let height = magnitude_height(*magnitude);

        bars_path.move_to(physical_x_coord, bounds.y + (bounds.h * (1.0 - height)));
        bars_path.line_to(physical_x_coord, bounds.y + bounds.h);

        previous_physical_x_coord = physical_x_coord;
    }
    canvas.stroke_path(&bars_path, &bars_paint);

    // The mesh path starts at the bottom left, follows the top envelope of the spectrum analyzer,
    // and ends in the bottom right
    let mut mesh_path = vg::Path::new();
    let mesh_start_x_coordiante = bounds.x + (bounds.w * bin_t(mesh_bin_start_idx as f32));
    let mesh_start_y_coordinate = bounds.y + bounds.h;

    mesh_path.move_to(mesh_start_x_coordiante, mesh_start_y_coordinate);
    for (bin_idx, magnitude) in analyzer_data
        .envelope_followers
        .iter()
        .enumerate()
        .take(analyzer_data.num_bins)
        .skip(mesh_bin_start_idx)
    {
        let t = bin_t(bin_idx as f32);
        if t <= 0.0 || t >= 1.0 {
            continue;
        }

        let physical_x_coord = bounds.x + (bounds.w * t);
        previous_physical_x_coord = physical_x_coord;
        let height = magnitude_height(*magnitude);
        if height > 0.0 {
            mesh_path.line_to(
                physical_x_coord,
                // This includes the line width, since this path is not stroked
                bounds.y + (bounds.h * (1.0 - height) - (line_width / 2.0)).max(0.0),
            );
        } else {
            mesh_path.line_to(physical_x_coord, mesh_start_y_coordinate);
        }
    }

    mesh_path.line_to(previous_physical_x_coord, mesh_start_y_coordinate);
    mesh_path.close();

    let mesh_paint = vg::Paint::linear_gradient_stops(
        mesh_start_x_coordiante,
        0.0,
        previous_physical_x_coord,
        0.0,
        [
            (0.0, lighter_text_color),
            (0.707, text_color),
            (1.0, text_color),
        ],
    )
    // NOTE:  This is very important, otherwise this looks all kinds of gnarly
    .with_anti_alias(false);
    canvas.fill_path(&mesh_path, &mesh_paint);
}

/// Overlays the threshold curve over the spectrum analyzer. If either the upwards or downwards
/// threshold offsets are non-zero then two curves are drawn.
fn draw_threshold_curve(cx: &mut DrawContext, canvas: &mut Canvas, analyzer_data: &AnalyzerData) {
    let bounds = cx.bounds();

    let line_width = cx.scale_factor() * 3.0;
    let downwards_paint =
        vg::Paint::color(DOWNWARDS_THRESHOLD_CURVE_COLOR).with_line_width(line_width);
    let upwards_paint = vg::Paint::color(UPWARDS_THRESHOLD_CURVE_COLOR).with_line_width(line_width);

    // This can be done slightly cleverer but for our purposes drawing line segments that are either
    // 1 pixel apart or that split the curve up into 100 segments (whichever results in the least
    // amount of line segments) should be sufficient
    let curve = Curve::new(&analyzer_data.curve_params);
    let num_points = 100.min(bounds.w.ceil() as usize);

    let mut draw_with_offset = |offset_db: f32, paint: vg::Paint| {
        let mut path = vg::Path::new();
        for i in 0..num_points {
            let x_t = i as f32 / (num_points - 1) as f32;
            let ln_freq = LN_FREQ_RANGE_START_HZ + (LN_FREQ_RANGE * x_t);

            // Evaluating the curve results in a value in dB, which must then be mapped to the same
            // scale used in `draw_spectrum()`
            let y_db = curve.evaluate_ln(ln_freq) + offset_db;
            let y_t = db_to_unclamped_t(y_db);

            let physical_x_pos = bounds.x + (bounds.w * x_t);
            // This value increases from bottom to top
            let physical_y_pos = bounds.y + (bounds.h * (1.0 - y_t));

            if i == 0 {
                path.move_to(physical_x_pos, physical_y_pos);
            } else {
                path.line_to(physical_x_pos, physical_y_pos);
            }
        }

        // This does a way better job at cutting off the tops and bottoms of the graph than we could do
        // by hand
        canvas.scissor(bounds.x, bounds.y, bounds.w, bounds.h);
        canvas.stroke_path(&path, &paint);
        canvas.reset_scissor();
    };

    let (upwards_offset_db, downwards_offset_db) = analyzer_data.curve_offsets_db;
    draw_with_offset(upwards_offset_db, upwards_paint);
    draw_with_offset(downwards_offset_db, downwards_paint);
}

/// Overlays the gain reduction display over the spectrum analyzer.
fn draw_gain_reduction(
    cx: &mut DrawContext,
    canvas: &mut Canvas,
    analyzer_data: &AnalyzerData,
    nyquist_hz: f32,
) {
    let bounds = cx.bounds();

    // As with the above, anti aliasing only causes issues
    let paint = vg::Paint::color(GR_BAR_OVERLAY_COLOR).with_anti_alias(false);

    let bin_frequency = |bin_idx: f32| (bin_idx / analyzer_data.num_bins as f32) * nyquist_hz;

    let mut path = vg::Path::new();
    for (bin_idx, gain_difference_db) in analyzer_data
        .gain_difference_db
        .iter()
        .enumerate()
        .take(analyzer_data.num_bins)
    {
        // Avoid drawing tiny slivers for low gain reduction values
        if gain_difference_db.abs() < 0.2 {
            continue;
        }

        // The gain reduction bars are drawn with the width of the bin, centered on the bin's center
        // frequency. The first and the last bin are extended to the edges of the graph because
        // otherwise it looks weird.
        let t_start = if bin_idx == 0 {
            0.0
        } else {
            let gr_start_ln_frequency = bin_frequency(bin_idx as f32 - 0.5).ln();
            (gr_start_ln_frequency - LN_FREQ_RANGE_START_HZ) / LN_FREQ_RANGE
        };
        let t_end = if bin_idx == analyzer_data.num_bins - 1 {
            1.0
        } else {
            let gr_end_ln_frequency = bin_frequency(bin_idx as f32 + 0.5).ln();
            (gr_end_ln_frequency - LN_FREQ_RANGE_START_HZ) / LN_FREQ_RANGE
        };
        if t_end < 0.0 || t_start > 1.0 {
            continue;
        }

        let (t_start, t_end) = (t_start.max(0.0), t_end.min(1.0));

        // For the bar's height we'll draw 0 dB of gain reduction as a flat line (except we
        // don't actually draw 0 dBs of GR because it looks glitchy, but that's besides the
        // point). 40 dB of gain reduction causes the bar to be drawn from the center all
        // the way to the bottom of the spectrum analyzer. 40 dB of additional gain causes
        // the bar to be drawn from the center all the way to the top of the graph.
        // NOTE: Y-coordinates go from top to bottom, hence the minus
        let t_y = ((-gain_difference_db + 40.0) / 80.0).clamp(0.0, 1.0);

        path.move_to(bounds.x + (bounds.w * t_start), bounds.y + (bounds.h * 0.5));
        path.line_to(bounds.x + (bounds.w * t_end), bounds.y + (bounds.h * 0.5));
        path.line_to(bounds.x + (bounds.w * t_end), bounds.y + (bounds.h * t_y));
        path.line_to(bounds.x + (bounds.w * t_start), bounds.y + (bounds.h * t_y));
        path.close();
    }

    canvas
        .global_composite_blend_func(vg::BlendFactor::DstAlpha, vg::BlendFactor::OneMinusDstColor);
    canvas.fill_path(&path, &paint);
    canvas.global_composite_blend_func(vg::BlendFactor::One, vg::BlendFactor::OneMinusSrcAlpha);
}
