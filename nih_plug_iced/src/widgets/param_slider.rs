//! A slider that integrates with NIH-plug's [`Param`] types.

use crate::{
    alignment, backend, event, layout, mouse, renderer, text, touch, Clipboard, Color, Element,
    Event, Layout, Length, Point, Rectangle, Shell, Size, Widget,
};
use nih_plug::prelude::{GuiContext, Param, ParamSetter};

use super::util;
use super::ParamMessage;

/// A slider that integrates with NIH-plug's [`Param`] types.
///
/// TODO: There are currently no styling options at all
/// TODO: Handle Shift+drag for granular drag
/// TODO: Handle Ctrl+click for reset
/// TODO: Handle Double click for reset
/// TODO: Handle Alt+click for text entry
pub struct ParamSlider<'a, P: Param, Renderer: text::Renderer> {
    state: &'a mut State,

    param: &'a P,
    /// We'll visualize the parameter's current value by drawing the difference between the current
    /// normalized value and the default normalized value.
    setter: ParamSetter<'a>,

    height: Length,
    width: Length,
    text_size: Option<u16>,
    font: Renderer::Font,
}

/// State for a [`ParamSlider`].
#[derive(Debug, Default)]
pub struct State {
    drag_active: bool,
}

impl<'a, P: Param, Renderer: text::Renderer> ParamSlider<'a, P, Renderer> {
    /// Creates a new [`ParamSlider`] for the given parameter.
    pub fn new(state: &'a mut State, param: &'a P, context: &'a dyn GuiContext) -> Self {
        let setter = ParamSetter::new(context);

        Self {
            state,

            param,
            setter,

            width: Length::Units(180),
            height: Length::Units(30),
            text_size: None,
            font: Renderer::Font::default(),
        }
    }

    /// Sets the width of the [`ParamSlider`].
    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    /// Sets the height of the [`ParamSlider`].
    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    /// Sets the text size of the [`ParamSlider`].
    pub fn text_size(mut self, size: u16) -> Self {
        self.text_size = Some(size);
        self
    }

    /// Sets the font of the [`ParamSlider`].
    pub fn font(mut self, font: Renderer::Font) -> Self {
        self.font = font;
        self
    }
}

impl<'a, P: Param, Renderer: text::Renderer> Widget<ParamMessage, Renderer>
    for ParamSlider<'a, P, Renderer>
{
    fn width(&self) -> Length {
        self.width
    }

    fn height(&self) -> Length {
        self.height
    }

    fn layout(&self, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let limits = limits.width(self.width).height(self.height);
        let size = limits.resolve(Size::ZERO);

        layout::Node::new(size)
    }

    fn on_event(
        &mut self,
        event: Event,
        layout: Layout<'_>,
        cursor_position: Point,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, ParamMessage>,
    ) -> event::Status {
        let bounds = layout.bounds();

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if bounds.contains(cursor_position) {
                    shell.publish(ParamMessage::BeginSetParameter(self.param.as_ptr()));
                    self.state.drag_active = true;

                    // Immediately trigger a parameter update if the value would be different
                    self.set_normalized_value(
                        shell,
                        util::remap_rect_x_coordinate(&bounds, cursor_position.x),
                    );

                    return event::Status::Captured;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. } | touch::Event::FingerLost { .. }) => {
                if bounds.contains(cursor_position) {
                    shell.publish(ParamMessage::EndSetParameter(self.param.as_ptr()));
                    self.state.drag_active = false;

                    return event::Status::Captured;
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. }) => {
                if self.state.drag_active && bounds.contains(cursor_position) {
                    self.set_normalized_value(
                        shell,
                        util::remap_rect_x_coordinate(&bounds, cursor_position.x),
                    );

                    return event::Status::Captured;
                }
            }
            _ => {}
        }

        event::Status::Ignored
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor_position: Point,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let bounds = layout.bounds();
        let is_mouse_over = bounds.contains(cursor_position);

        if is_mouse_over {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor_position: Point,
        _viewport: &Rectangle,
    ) {
        const BORDER_WIDTH: f32 = 1.0;

        let bounds = layout.bounds();
        let is_mouse_over = bounds.contains(cursor_position);

        // The bar itself
        let background_color = if is_mouse_over {
            Color::new(0.5, 0.5, 0.5, 0.1)
        } else {
            Color::TRANSPARENT
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border_color: Color::BLACK,
                border_width: BORDER_WIDTH,
                border_radius: 0.0,
            },
            background_color,
        );

        // We'll visualize the difference between the current value and the default value
        let current_value = self.param.normalized_value();
        let fill_start_x = util::remap_rect_x_t(
            &bounds,
            self.setter.default_normalized_param_value(self.param),
        );
        let fill_end_x = util::remap_rect_x_t(&bounds, current_value);

        let fill_color = Color::from_rgb8(196, 196, 196);
        let fill_rect = Rectangle {
            x: fill_start_x.min(fill_end_x),
            y: bounds.y + BORDER_WIDTH,
            width: (fill_end_x - fill_start_x).abs(),
            height: bounds.height - BORDER_WIDTH * 2.0,
        };
        renderer.fill_quad(
            renderer::Quad {
                bounds: fill_rect,
                border_color: Color::TRANSPARENT,
                border_width: 0.0,
                border_radius: 0.0,
            },
            fill_color,
        );

        // We'll overlay the label on the slider. To make it more readable (and because it looks
        // cool), the parts that overlap with the fill rect will be rendered in white while the rest
        // will be rendered in black.
        let display_value = self.param.to_string();
        let text_size = self.text_size.unwrap_or_else(|| renderer.default_size()) as f32;
        let text_bounds = Rectangle {
            x: bounds.center_x(),
            y: bounds.center_y(),
            ..bounds
        };
        renderer.fill_text(text::Text {
            content: &display_value,
            font: self.font.clone(),
            size: text_size,
            bounds: text_bounds,
            color: style.text_color,
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
        });

        // This will clip to the filled area
        renderer.with_layer(fill_rect, |renderer| {
            let filled_text_color = Color::from_rgb8(80, 80, 80);
            renderer.fill_text(text::Text {
                content: &display_value,
                font: self.font.clone(),
                size: text_size,
                bounds: text_bounds,
                color: filled_text_color,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
            });
        });
    }
}

impl<'a, P: Param, Renderer: text::Renderer> ParamSlider<'a, P, Renderer> {
    /// Set the normalized value for a parameter if that would change the parameter's plain value
    /// (to avoid unnecessary duplicate parameter changes). The begin- and end set parameter
    /// messages need to be sent before calling this function.
    fn set_normalized_value(&self, shell: &mut Shell<'_, ParamMessage>, normalized_value: f32) {
        // This snaps to the nearest plain value if the parameter is stepped in some way.
        // TODO: As an optimization, we could add a `const CONTINUOUS: bool` to the parameter to
        //       avoid this normalized->plain->normalized conversion for parameters that don't need
        //       it
        let plain_value = self.param.preview_plain(normalized_value);
        let current_plain_value = self.param.plain_value();
        if plain_value != current_plain_value {
            shell.publish(ParamMessage::SetParameterNormalized(
                self.param.as_ptr(),
                normalized_value,
            ));
        }
    }
}

impl<'a, P: Param> ParamSlider<'a, P, backend::Renderer> {
    /// Convert this [`ParamSlider`] into an [`Element`] with the correct message. You should have a
    /// variant on your own message type that wraps around [`ParamMessage`] so you can forward those
    /// messages to
    /// [`IcedEditor::handle_param_message()`][crate::IcedEditor::handle_param_message()].
    pub fn map<Message, F>(self, f: F) -> Element<'a, Message>
    where
        Message: 'static,
        F: Fn(ParamMessage) -> Message + 'static,
    {
        Element::from(self).map(f)
    }
}

impl<'a, P: Param> From<ParamSlider<'a, P, backend::Renderer>> for Element<'a, ParamMessage> {
    fn from(widget: ParamSlider<'a, P, backend::Renderer>) -> Self {
        Element::new(widget)
    }
}
