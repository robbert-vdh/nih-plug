//! A super simple peak meter widget.

use femtovg::{Paint, Path};
use nih_plug::prelude::util;
use std::cell::Cell;
use std::time::Duration;
use std::time::Instant;
use vizia::*;

/// The thickness of a tick inside of the peak meter's bar.
const TICK_WIDTH: f32 = 1.0;
/// The gap between individual ticks.
const TICK_GAP: f32 = 1.0;

/// The decibel value corresponding to the very left of the bar.
const MIN_TICK: f32 = -90.0;
/// The decibel value corresponding to the very right of the bar.
const MAX_TICK: f32 = 20.0;

/// A simple horizontal peak meter.
///
/// TODO: There are currently no styling options at all
/// TODO: Vertical peak meter, this is just a proof of concept to fit the gain GUI example.
pub struct PeakMeter;

/// The bar bit for the peak meter, manually drawn using vertical lines.
struct PeakMeterInner<L, P>
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
    ///
    /// See [`PeakMeterExt`] for additonal options.
    pub fn new<L>(
        cx: &mut Context,
        level_dbfs: L,
        hold_time: Option<Duration>,
    ) -> Handle<'_, PeakMeter>
    where
        L: Lens<Target = f32>,
    {
        Self.build2(cx, |cx| {
            // Now for something that may be illegal under some jurisdictions. If a hold time is
            // given, then we'll build a new lens that always gives the held peak level for the
            // current moment in time by mutating some values captured into the mapping closure.
            let peak_dbfs = match hold_time {
                Some(hold_time) => {
                    let held_peak_value_db = Cell::new(f32::MIN);
                    let last_held_peak_value: Cell<Option<Instant>> = Cell::new(None);
                    level_dbfs.clone().map(move |level| -> f32 {
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
                    })
                }
                None => level_dbfs
                    .clone()
                    .map(|_level| -> f32 { util::MINUS_INFINITY_DB }),
            };

            PeakMeterInner {
                level_dbfs,
                peak_dbfs,
            }
            .build(cx)
            .class("bar");

            // TODO: Ticks
        })
    }
}

impl View for PeakMeter {
    fn element(&self) -> Option<String> {
        Some(String::from("peak-meter"))
    }
}

impl<L, P> View for PeakMeterInner<L, P>
where
    L: Lens<Target = f32>,
    P: Lens<Target = f32>,
{
    fn draw(&self, cx: &mut Context, canvas: &mut Canvas) {
        let level_dbfs = *self.level_dbfs.get(cx);
        let peak_dbfs = *self.peak_dbfs.get(cx);

        // These basics are taken directly from the default implementation of this function
        let entity = cx.current;
        let bounds = cx.cache.get_bounds(entity);
        if bounds.w == 0.0 || bounds.h == 0.0 {
            return;
        }

        // TODO: It would be cool to allow the text color property to control the gradient here. For
        //       now we'll only support basic background colors and borders.
        let background_color = cx
            .style
            .background_color
            .get(entity)
            .cloned()
            .unwrap_or_default();
        let border_color = cx
            .style
            .border_color
            .get(entity)
            .cloned()
            .unwrap_or_default();
        let opacity = cx.cache.get_opacity(entity);
        let mut background_color: femtovg::Color = background_color.into();
        background_color.set_alphaf(background_color.a * opacity);
        let mut border_color: femtovg::Color = border_color.into();
        border_color.set_alphaf(border_color.a * opacity);

        let border_width = match cx
            .style
            .border_width
            .get(entity)
            .cloned()
            .unwrap_or_default()
        {
            Units::Pixels(val) => val,
            Units::Percentage(val) => bounds.w.min(bounds.h) * (val / 100.0),
            _ => 0.0,
        };

        let mut path = Path::new();
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
        let paint = Paint::color(background_color);
        canvas.fill_path(&mut path, paint);

        // And now for the fun stuff. We'll try to not overlap the border, but we'll draw that last
        // just in case.
        let bar_bounds = bounds.shrink(border_width / 2.0);
        let bar_ticks_start_x = bar_bounds.left().floor() as i32;
        let bar_ticks_end_x = bar_bounds.right().ceil() as i32;
        let bar_tick_coordinates =
            (bar_ticks_start_x..bar_ticks_end_x).step_by((TICK_WIDTH + TICK_GAP).round() as usize);
        for tick_x in bar_tick_coordinates {
            let tick_fraction =
                (tick_x - bar_ticks_start_x) as f32 / (bar_ticks_end_x - bar_ticks_start_x) as f32;
            let tick_db = (tick_fraction * (MAX_TICK - MIN_TICK)) + MIN_TICK;
            if tick_db > level_dbfs {
                break;
            }

            // femtovg draws paths centered on these coordinates, so in order to be pixel perfect we
            // need to account for that. Otherwise the ticks will be 2px wide instead of 1px.
            let mut path = Path::new();
            path.move_to(tick_x as f32 + 0.5, bar_bounds.top());
            path.line_to(tick_x as f32 + 0.5, bar_bounds.bottom());

            let grayscale_color = 0.3 + ((1.0 - tick_fraction) * 0.5);
            let mut paint = Paint::color(femtovg::Color::rgbaf(
                grayscale_color,
                grayscale_color,
                grayscale_color,
                opacity,
            ));
            paint.set_line_width(TICK_WIDTH);
            canvas.stroke_path(&mut path, paint);
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
            let mut path = Path::new();
            path.move_to(peak_x + 0.5, bar_bounds.top());
            path.line_to(peak_x + 0.5, bar_bounds.bottom());

            let mut paint = Paint::color(femtovg::Color::rgbaf(0.3, 0.3, 0.3, opacity));
            paint.set_line_width(TICK_WIDTH);
            canvas.stroke_path(&mut path, paint);
        }

        // Draw border last
        let mut paint = Paint::color(border_color);
        paint.set_line_width(border_width);
        canvas.stroke_path(&mut path, paint);
    }
}
