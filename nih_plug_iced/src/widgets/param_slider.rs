//! A slider that integrates with NIH-plug's [`Param`] types.

use crate::backend;
use crate::event::{self, Event};
use crate::layout;
use crate::mouse;
use crate::renderer;
use crate::text;
use crate::{Clipboard, Color, Element, Layout, Length, Point, Rectangle, Shell, Size, Widget};
use nih_plug::prelude::Param;

use super::ParamMessage;

/// A slider that integrates with NIH-plug's [`Param`] types.
///
/// TODO: There are currently no styling options at all
pub struct ParamSlider<'a, P: Param, Renderer: text::Renderer> {
    param: &'a P,

    height: Length,
    width: Length,
    text_size: Option<u16>,
    font: Renderer::Font,
}

impl<'a, P: Param, Renderer: text::Renderer> ParamSlider<'a, P, Renderer> {
    /// Creates a new [`ParamSlider`] for the given parameter.
    pub fn new(param: &'a P) -> Self {
        Self {
            param,
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
        _event: Event,
        _layout: Layout<'_>,
        _cursor_position: Point,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        _shell: &mut Shell<'_, ParamMessage>,
    ) -> event::Status {
        // TODO: Handle interaction
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
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor_position: Point,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let is_mouse_over = bounds.contains(cursor_position);

        // TODO:
        let background_color = if is_mouse_over {
            Color::new(0.5, 0.5, 0.5, 0.2)
        } else {
            Color::TRANSPARENT
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border_color: Color::BLACK,
                border_width: 1.0,
                border_radius: 0.0,
            },
            background_color,
        );

        // TODO:

        // renderer.fill_text(Text {
        //     content: &Renderer::ARROW_DOWN_ICON.to_string(),
        //     font: Renderer::ICON_FONT,
        //     size: bounds.height * style.icon_size,
        //     bounds: Rectangle {
        //         x: bounds.x + bounds.width - f32::from(self.padding.horizontal()),
        //         y: bounds.center_y(),
        //         ..bounds
        //     },
        //     color: style.text_color,
        //     horizontal_alignment: alignment::Horizontal::Right,
        //     vertical_alignment: alignment::Vertical::Center,
        // });

        // if let Some(label) = self
        //     .selected
        //     .as_ref()
        //     .map(ToString::to_string)
        //     .as_ref()
        //     .or_else(|| self.placeholder.as_ref())
        // {
        //     renderer.fill_text(Text {
        //         content: label,
        //         size: f32::from(self.text_size.unwrap_or(renderer.default_size())),
        //         font: self.font.clone(),
        //         color: is_selected
        //             .then(|| style.text_color)
        //             .unwrap_or(style.placeholder_color),
        //         bounds: Rectangle {
        //             x: bounds.x + f32::from(self.padding.left),
        //             y: bounds.center_y(),
        //             ..bounds
        //         },
        //         horizontal_alignment: alignment::Horizontal::Left,
        //         vertical_alignment: alignment::Vertical::Center,
        //     })
        // }
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
