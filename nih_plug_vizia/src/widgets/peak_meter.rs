//! A super simple peak meter widget.

use nih_plug::prelude::util;
use std::cell::Cell;
use std::time::Duration;
use std::time::Instant;
use vizia::prelude::*;
use vizia::vg;

/// The thickness of a tick inside of the peak meter's bar.
const TICK_WIDTH: f32 = 1.0;
/// The gap between individual ticks.
const TICK_GAP: f32 = 1.0;

/// The decibel value corresponding to the very left of the bar.
const MIN_TICK: f32 = -90.0;
/// The decibel value corresponding to the very right of the bar.
const MAX_TICK: f32 = 20.0;
/// The ticks that will be shown beneath the peak meter's bar. The first value is shown as
/// -infinity, and at the last position we'll draw the `dBFS` string.
const TEXT_TICKS: [i32; 6] = [-80, -60, -40, -20, 0, 12];

/// A simple horizontal peak meter.
///
/// TODO: There are currently no styling options at all
/// TODO: Vertical peak meter, this is just a proof of concept to fit the gain GUI example.
pub struct PeakMeter;

/// The bar bit for the peak meter, manually drawn using vertical lines.
struct PeakMeterBar<L, P>
where
    L: Lens<Target = f32>,
    P: Lens<Target = f32>,
{
    level_dbfs: L,
    peak_dbfs: P,
}

impl PeakMeter {
    /// Creates a new [`PeakMeter`] for the given value in decibel, optionally holding the peak
    /// value for a certain amount of time.
    pub fn new<L>(cx: &mut Context, level_dbfs: L, hold_time: Option<Duration>) -> Handle<Self>
    where
        L: Lens<Target = f32>,
    {
        Self.build(cx, |cx| {
            // Now for something that may be illegal under some jurisdictions. If a hold time is
            // given, then we'll build a new lens that always gives the held peak level for the
            // current moment in time by mutating some values captured into the mapping closure.
            let held_peak_value_db = Cell::new(f32::MIN);
            let last_held_peak_value: Cell<Option<Instant>> = Cell::new(None);
            let peak_dbfs = level_dbfs.clone().map(move |level| -> f32 {
                match hold_time {
                    Some(hold_time) => {
                        let mut peak_level = held_peak_value_db.get();
                        let peak_time = last_held_peak_value.get();

                        let now = Instant::now();
                        if *level >= peak_level
                            || peak_time.is_none()
                            || now > peak_time.unwrap() + hold_time
                        {
                            peak_level = *level;
                            held_peak_value_db.set(peak_level);
                            last_held_peak_value.set(Some(now));
                        }

                        peak_level
                    }
                    None => util::MINUS_INFINITY_DB,
                }
            });

            PeakMeterBar {
                level_dbfs,
                peak_dbfs,
            }
            .build(cx, |_| {})
            .class("bar");

            ZStack::new(cx, |cx| {
                const WIDTH_PCT: f32 = 50.0;
                for tick_db in TEXT_TICKS {
                    let tick_fraction = (tick_db as f32 - MIN_TICK) / (MAX_TICK - MIN_TICK);
                    let tick_pct = tick_fraction * 100.0;
                    // We'll shift negative numbers slightly to the left so they look more centered
                    let needs_minus_offset = tick_db < 0;

                    ZStack::new(cx, |cx| {
                        let first_tick = tick_db == TEXT_TICKS[0];
                        let last_tick = tick_db == TEXT_TICKS[TEXT_TICKS.len() - 1];

                        if !last_tick {
                            // FIXME: This is not aligned to the pixel grid and some ticks will look
                            //        blurry, is there a way to fix this?
                            Element::new(cx).class("ticks__tick");
                        }

                        let font_size = {
                            let current = cx.current();
                            let draw_cx = DrawContext::new(cx);
                            draw_cx.font_size(current) * draw_cx.style.dpi_factor as f32
                        };
                        let label = if first_tick {
                            Label::new(cx, "-inf")
                                .class("ticks__label")
                                .class("ticks__label--inf")
                        } else if last_tick {
                            // This is only inclued in the array to make positioning this easier
                            Label::new(cx, "dBFS")
                                .class("ticks__label")
                                .class("ticks__label--dbfs")
                        } else {
                            Label::new(cx, &tick_db.to_string()).class("ticks__label")
                        }
                        .overflow(Overflow::Visible);

                        if needs_minus_offset {
                            label.child_right(Pixels(font_size * 0.15));
                        }
                    })
                    .height(Stretch(1.0))
                    .left(Percentage(tick_pct - (WIDTH_PCT / 2.0)))
                    .width(Percentage(WIDTH_PCT))
                    .child_left(Stretch(1.0))
                    .child_right(Stretch(1.0))
                    .overflow(Overflow::Visible);
                }
            })
            .class("ticks")
            .overflow(Overflow::Visible);
        })
        .overflow(Overflow::Visible)
    }
}

impl View for PeakMeter {
    fn element(&self) -> Option<&'static str> {
        Some("peak-meter")
    }
}

impl<L, P> View for PeakMeterBar<L, P>
where
    L: Lens<Target = f32>,
    P: Lens<Target = f32>,
{
    fn draw(&self, cx: &mut DrawContext, canvas: &mut Canvas) {
        let level_dbfs = self.level_dbfs.get(cx);
        let peak_dbfs = self.peak_dbfs.get(cx);

        // These basics are taken directly from the default implementation of this function
        let bounds = cx.bounds();
        if bounds.w == 0.0 || bounds.h == 0.0 {
            return;
        }

        // TODO: It would be cool to allow the text color property to control the gradient here. For
        //       now we'll only support basic background colors and borders.
        let background_color = cx.background_color().cloned().unwrap_or_default();
        let border_color = cx.border_color().cloned().unwrap_or_default();
        let opacity = cx.opacity();
        let mut background_color: vg::Color = background_color.into();
        background_color.set_alphaf(background_color.a * opacity);
        let mut border_color: vg::Color = border_color.into();
        border_color.set_alphaf(border_color.a * opacity);

        let border_width = match cx.border_width().unwrap_or_default() {
            Units::Pixels(val) => val,
            Units::Percentage(val) => bounds.w.min(bounds.h) * (val / 100.0),
            _ => 0.0,
        };

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

        // Fill with background color
        let paint = vg::Paint::color(background_color);
        canvas.fill_path(&mut path, &paint);

        // And now for the fun stuff. We'll try to not overlap the border, but we'll draw that last
        // just in case.
        let bar_bounds = bounds.shrink(border_width / 2.0);
        let bar_ticks_start_x = bar_bounds.left().floor() as i32;
        let bar_ticks_end_x = bar_bounds.right().ceil() as i32;

        // NOTE: We'll scale this with the nearest integer DPI ratio. That way it will still look
        //       good at 2x scaling, and it won't look blurry at 1.x times scaling.
        let dpi_scale = cx.logical_to_physical(1.0).floor().max(1.0);
        let bar_tick_coordinates = (bar_ticks_start_x..bar_ticks_end_x)
            .step_by(((TICK_WIDTH + TICK_GAP) * dpi_scale).round() as usize);
        for tick_x in bar_tick_coordinates {
            let tick_fraction =
                (tick_x - bar_ticks_start_x) as f32 / (bar_ticks_end_x - bar_ticks_start_x) as f32;
            let tick_db = (tick_fraction * (MAX_TICK - MIN_TICK)) + MIN_TICK;
            if tick_db > level_dbfs {
                break;
            }

            // femtovg draws paths centered on these coordinates, so in order to be pixel perfect we
            // need to account for that. Otherwise the ticks will be 2px wide instead of 1px.
            let mut path = vg::Path::new();
            path.move_to(tick_x as f32 + (dpi_scale / 2.0), bar_bounds.top());
            path.line_to(tick_x as f32 + (dpi_scale / 2.0), bar_bounds.bottom());

            let grayscale_color = 0.3 + ((1.0 - tick_fraction) * 0.5);
            let mut paint = vg::Paint::color(vg::Color::rgbaf(
                grayscale_color,
                grayscale_color,
                grayscale_color,
                opacity,
            ));
            paint.set_line_width(TICK_WIDTH * dpi_scale);
            canvas.stroke_path(&mut path, &paint);
        }

        // Draw the hold peak value if the hold time option has been set
        let db_to_x_coord = |db: f32| {
            let tick_fraction = (db - MIN_TICK) / (MAX_TICK - MIN_TICK);
            bar_ticks_start_x as f32
                + ((bar_ticks_end_x - bar_ticks_start_x) as f32 * tick_fraction).round()
        };
        if (MIN_TICK..MAX_TICK).contains(&peak_dbfs) {
            // femtovg draws paths centered on these coordinates, so in order to be pixel perfect we
            // need to account for that. Otherwise the ticks will be 2px wide instead of 1px.
            let peak_x = db_to_x_coord(peak_dbfs);
            let mut path = vg::Path::new();
            path.move_to(peak_x + (dpi_scale / 2.0), bar_bounds.top());
            path.line_to(peak_x + (dpi_scale / 2.0), bar_bounds.bottom());

            let mut paint = vg::Paint::color(vg::Color::rgbaf(0.3, 0.3, 0.3, opacity));
            paint.set_line_width(TICK_WIDTH * dpi_scale);
            canvas.stroke_path(&mut path, &paint);
        }

        // Draw border last
        let mut paint = vg::Paint::color(border_color);
        paint.set_line_width(border_width);
        canvas.stroke_path(&mut path, &paint);
    }
}
