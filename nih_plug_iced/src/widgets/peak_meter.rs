//! A super simple peak meter widget.

use crossbeam::atomic::AtomicCell;
use std::marker::PhantomData;
use std::time::Duration;
use std::time::Instant;

use crate::core::text::{self, Paragraph, Renderer as TextRenderer};
use crate::core::widget::tree::{self, Tree};
use crate::core::{
    alignment, layout, mouse, padding, renderer, Background, Border, Color, Element, Font, Layout,
    Length, Pixels, Point, Rectangle, Size, Widget,
};

/// The thickness of this widget's borders.
const BORDER_WIDTH: f32 = 1.0;
/// The thickness of a tick inside of the peak meter's bar.
const TICK_WIDTH: f32 = 1.0;

/// A simple horizontal peak meter.
///
/// TODO: There are currently no styling options at all
/// TODO: Vertical peak meter, this is just a proof of concept to fit the gain GUI example.
pub struct PeakMeter<Message> {
    /// The current measured value in decibel.
    current_value_db: f32,

    /// The time the old peak value should remain visible.
    hold_time: Option<Duration>,

    height: Length,
    width: Length,
    text_size: Option<Pixels>,
    font: Option<Font>,

    /// We don't emit any messages, but iced requires us to define some message type anyways.
    _phantom: PhantomData<Message>,
}

/// State for a [`PeakMeter`].
#[derive(Debug, Default)]
struct State {
    /// The last peak value in decibel.
    held_peak_value_db: AtomicCell<f32>,
    /// When the last peak value was hit.
    last_held_peak_value: AtomicCell<Option<Instant>>,
}

impl<Message> PeakMeter<Message> {
    /// Creates a new [`PeakMeter`] using the current measurement in decibel. This measurement can
    /// already have some form of smoothing applied to it. This peak slider widget can draw the last
    /// hold value for you.
    pub fn new(value_db: f32) -> Self {
        Self {
            current_value_db: value_db,

            hold_time: None,

            width: Length::Fixed(180.0),
            height: Length::Fixed(30.0),
            text_size: None,
            font: None,

            _phantom: PhantomData,
        }
    }

    /// Keep showing the peak value for a certain amount of time.
    pub fn hold_time(mut self, time: Duration) -> Self {
        self.hold_time = Some(time);
        self
    }

    /// Sets the width of the [`PeakMeter`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`PeakMeter`].
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets the text size of the [`PeakMeter`]'s ticks bar.
    pub fn text_size(mut self, size: impl Into<Pixels>) -> Self {
        self.text_size = Some(size.into());
        self
    }

    /// Sets the font of the [`PeakMeter`]'s ticks bar.
    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for PeakMeter<Message>
where
    Message: Clone,
    Renderer: TextRenderer,
    Renderer::Font: From<crate::Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        (self.width, self.height).into()
    }

    fn layout(
        &self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();

        let bounds = layout.bounds();
        let bar_bounds = bounds.shrink(padding::bottom(bounds.height / 2.0));
        let ticks_bounds = bounds.shrink(padding::top(bounds.height / 2.0));

        // We'll draw a simple horizontal for [-90, 20] dB where we'll treat -80 as -infinity, with
        // a label containing the tick markers below it. If `.hold_time()` was called then we'll
        // also display the last held value
        const MIN_TICK: f32 = -90.0;
        const MAX_TICK: f32 = 20.0;
        let text_ticks = [-80i32, -60, -40, -20, 0];
        // Draw a tick with one pixel in between, otherwise the bilinear interpolation makes
        // everything a smeary mess
        let bar_ticks_start = (bar_bounds.x + BORDER_WIDTH).round() as i32;
        let bar_ticks_end = (bar_bounds.x + bar_bounds.width - (BORDER_WIDTH * 2.0)).ceil() as i32;
        let bar_tick_coordinates =
            (bar_ticks_start..bar_ticks_end).step_by((TICK_WIDTH + 1.0).round() as usize);
        let db_to_x_coord = |db: f32| {
            let tick_fraction = (db - MIN_TICK) / (MAX_TICK - MIN_TICK);
            bar_ticks_start as f32
                + ((bar_ticks_end - bar_ticks_start) as f32 * tick_fraction).round()
        };

        for tick_x in bar_tick_coordinates {
            let tick_fraction =
                (tick_x - bar_ticks_start) as f32 / (bar_ticks_end - bar_ticks_start) as f32;
            let tick_db = (tick_fraction * (MAX_TICK - MIN_TICK)) + MIN_TICK;
            if tick_db > self.current_value_db {
                break;
            }

            let tick_bounds = Rectangle {
                x: tick_x as f32,
                y: bar_bounds.y + BORDER_WIDTH,
                width: TICK_WIDTH,
                height: bar_bounds.height - (BORDER_WIDTH * 2.0),
            };

            let grayscale_color = 0.3 + ((1.0 - tick_fraction) * 0.5);
            let tick_color = Color::from_rgb(grayscale_color, grayscale_color, grayscale_color);
            renderer.fill_quad(
                renderer::Quad {
                    bounds: tick_bounds,
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                },
                Background::Color(tick_color),
            );
        }

        // Draw the hold peak value if the hold time option has been set
        if let Some(hold_time) = self.hold_time {
            let now = Instant::now();
            let mut held_peak_value_db = state.held_peak_value_db.load();
            let last_peak_value = state.last_held_peak_value.load();
            if self.current_value_db >= held_peak_value_db
                || last_peak_value.is_none()
                || now > last_peak_value.unwrap() + hold_time
            {
                state.held_peak_value_db.store(self.current_value_db);
                state.last_held_peak_value.store(Some(now));
                held_peak_value_db = self.current_value_db;
            }

            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: db_to_x_coord(held_peak_value_db),
                        y: bar_bounds.y + BORDER_WIDTH,
                        width: TICK_WIDTH,
                        height: bar_bounds.height - (BORDER_WIDTH * 2.0),
                    },
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                },
                Background::Color(Color::from_rgb(0.3, 0.3, 0.3)),
            );
        }

        // Draw the bar after the ticks since the first and last tick may overlap with the borders
        renderer.fill_quad(
            renderer::Quad {
                bounds: bar_bounds,
                border: Border {
                    color: Color::BLACK,
                    width: BORDER_WIDTH,
                    radius: 0.0.into(),
                },
                ..Default::default()
            },
            Background::Color(Color::TRANSPARENT),
        );

        let text_size = self
            .text_size
            .unwrap_or_else(|| Pixels((renderer.default_size().0 * 0.7).round()));
        let font = self
            .font
            .map(Renderer::Font::from)
            .unwrap_or_else(|| renderer.default_font());

        // Beneath the bar we want to draw the names of the ticks
        for tick_db in text_ticks {
            let x_coordinate = db_to_x_coord(tick_db as f32);

            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: x_coordinate,
                        y: ticks_bounds.y,
                        width: TICK_WIDTH,
                        height: ticks_bounds.height * 0.3,
                    },
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                },
                Background::Color(Color::from_rgb(0.3, 0.3, 0.3)),
            );

            let tick_text = if tick_db == text_ticks[0] {
                String::from("-inf")
            } else {
                tick_db.to_string()
            };
            renderer.fill_text(
                text::Text {
                    content: tick_text,
                    font,
                    size: text_size,
                    bounds: ticks_bounds.size(),
                    horizontal_alignment: alignment::Horizontal::Center,
                    vertical_alignment: alignment::Vertical::Top,
                    line_height: Default::default(),
                    shaping: Default::default(),
                    wrapping: text::Wrapping::None,
                },
                Point {
                    x: x_coordinate,
                    y: ticks_bounds.y + (ticks_bounds.height * 0.35),
                },
                style.text_color,
                *viewport,
            );
        }

        // Every proper graph needs a unit label
        let zero_db_x_coordinate = db_to_x_coord(0.0);

        let zero_db_text_width = Renderer::Paragraph::with_text(text::Text {
            content: "0",
            font,
            size: text_size,
            bounds: ticks_bounds.size(),
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Top,
            line_height: Default::default(),
            shaping: Default::default(),
            wrapping: text::Wrapping::None,
        })
        .min_width();

        renderer.fill_text(
            text::Text {
                // The spacing looks a bit off if we start with a space here so we'll add a little
                // offset to the x-coordinate instead
                content: "dBFS".into(),
                font,
                size: text_size,
                bounds: ticks_bounds.size(),
                horizontal_alignment: alignment::Horizontal::Left,
                vertical_alignment: alignment::Vertical::Top,
                line_height: Default::default(),
                shaping: Default::default(),
                wrapping: text::Wrapping::None,
            },
            Point {
                x: zero_db_x_coordinate + (zero_db_text_width / 2.0) + (text_size.0 * 0.2),
                y: ticks_bounds.y + (ticks_bounds.height * 0.35),
            },
            style.text_color,
            *viewport,
        );
    }
}

impl<'a, Theme, Message, Renderer> From<PeakMeter<Message>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Renderer: TextRenderer + 'a,
    Renderer::Font: From<crate::Font>,
{
    fn from(widget: PeakMeter<Message>) -> Self {
        Element::new(widget)
    }
}
