use iced_baseview::theme::palette::Pair;
use iced_baseview::{self as iced, alignment};
use nih_plug::prelude::Param;
use std::borrow::Borrow;

use iced::core::event::{self, Event};
use iced::core::layout::{self, Limits};
use iced::core::text::{self, Paragraph, Renderer as TextRenderer, Text};
use iced::core::widget::operation::{self};
use iced::core::widget::tree::{self, Tree};
use iced::core::{
    keyboard, mouse, renderer, touch, Border, Clipboard, Element, Font, Layout, Length, Padding,
    Pixels, Rectangle, Shell, Size, Vector, Widget,
};
use iced::widget::text_input;
use iced::widget::text_input::{Id, TextInput};

use super::{util, ParamMessage};

/// When shift+dragging a parameter, one pixel dragged corresponds to this much change in the
/// noramlized parameter.
const GRANULAR_DRAG_MULTIPLIER: f32 = 0.1;

/// The thickness of this widget's borders.
const BORDER_WIDTH: f32 = 1.0;

pub struct ParamSlider<'a, P, Theme = iced::Theme>
where
    P: Param,
    Theme: Catalog,
{
    param: &'a P,

    width: Length,
    height: Length,
    text_size: Option<Pixels>,
    font: Option<Font>,
    class: Theme::Class<'a>,
}

/// State for a [`ParamSlider`].
#[derive(Debug)]
struct State {
    keyboard_modifiers: keyboard::Modifiers,
    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,
    /// We keep track of the start coordinate and normalized value holding down Shift while dragging
    /// for higher precision dragging. This is a `None` value when granular dragging is not active.
    granular_drag_start_x_value: Option<(f32, f32)>,
    /// Track clicks for double clicks.
    last_click: Option<mouse::Click>,

    /// The text that's currently in the text input. If this is set to `None`, then the text input
    /// is not visible.
    text_input_value: Option<String>,
    text_input_id: Id,
}

impl Default for State {
    fn default() -> Self {
        Self {
            text_input_id: Id::unique(),
            keyboard_modifiers: Default::default(),
            drag_active: Default::default(),
            granular_drag_start_x_value: Default::default(),
            last_click: Default::default(),
            text_input_value: Default::default(),
        }
    }
}

/// The possible UI status of a [`ParamSlider`]. Enables drawing of different styles for each status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The [`ParamSlider`] can be interacted with.
    Active,
    /// The [`ParamSlider`] is being hovered.
    Hovered,
    /// The [`ParamSlider`] is being dragged.
    Dragged,
}

/// An internal message for intercep- I mean handling output from the embedded [`TextInput`] widget.
#[derive(Debug, Clone)]
enum TextInputMessage {
    /// A new value was entered in the text input dialog.
    Value(String),
    /// Enter was pressed.
    Submit,
}

impl<'a, P, Theme> ParamSlider<'a, P, Theme>
where
    P: Param,
    Theme: Catalog,
{
    pub const DEFAULT_WIDTH: Length = Length::Fixed(180.0);
    pub const DEFAULT_HEIGHT: Length = Length::Fixed(30.0);

    /// Creates a new [`ParamSlider`] for the given parameter.
    pub fn new(param: &'a P) -> Self {
        Self {
            param,

            width: Self::DEFAULT_WIDTH,
            height: Self::DEFAULT_HEIGHT,
            text_size: None,
            font: None,
            class: Theme::default(),
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
    pub fn text_size(mut self, size: Pixels) -> Self {
        self.text_size = Some(size);
        self
    }

    /// Sets the font of the [`ParamSlider`].
    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    /// Sets the style of the [`ParamSlider`].
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }
}

impl<'a, P, Theme, Renderer> Widget<ParamMessage, Theme, Renderer> for ParamSlider<'a, P, Theme>
where
    P: Param,
    Theme: Catalog + text_input::Catalog,
    Renderer: TextRenderer,
    Renderer::Font: From<iced::Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        // One child to store text input state.
        vec![Tree::empty()]
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(&self, _tree: &mut Tree, _renderer: &Renderer, limits: &Limits) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();

        let status = if state.drag_active {
            Status::Dragged
        } else if cursor.is_over(bounds) {
            Status::Hovered
        } else {
            Status::Active
        };
        let style = Catalog::style(theme, &self.class, status);

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: style.border,
                ..Default::default()
            },
            style.background.color,
        );

        // Shrink bounds to inside of the border
        let bounds = bounds.shrink(Padding::new(style.border.width));

        let Some(current_value) = &state.text_input_value else {
            return self.draw_bar(renderer, &style, &bounds, viewport);
        };

        self.with_text_input(
            layout,
            renderer,
            current_value,
            state,
            |text_input, layout, renderer| {
                text_input.draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    layout,
                    cursor,
                    None,
                    viewport,
                );
            },
        );
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, ParamMessage>,
        viewport: &Rectangle,
    ) -> event::Status {
        let state = tree.state.downcast_mut::<State>();

        // The pressence of a value in `self.state.text_input_value` indicates that the field should
        // be focussed. The field handles defocussing by itself
        // FIMXE: This is super hacky, I have no idea how you can reuse the text input widget
        //        otherwise. Widgets are not supposed to handle messages from other widgets, but
        //        we'll do so anyways by using a special `TextInputMessage` type and our own
        //        `Shell`.
        let text_input_status = if let Some(current_value) = &state.text_input_value {
            let event = event.clone();
            let mut messages = Vec::new();
            let mut text_input_shell = Shell::new(&mut messages);

            let status = self.with_text_input(
                layout,
                renderer,
                current_value,
                &state,
                |mut text_input, layout, renderer| {
                    if tree.children[0].tag != text_input.tag() {
                        tree.children[0] = Tree {
                            tag: text_input.tag(),
                            state: text_input.state(),
                            children: text_input.children(),
                        };
                    }

                    text_input.on_event(
                        &mut tree.children[0],
                        event,
                        layout,
                        cursor,
                        renderer,
                        clipboard,
                        &mut text_input_shell,
                        viewport,
                    )
                },
            );

            // Check if text input is focused.
            let text_input_state = tree.children[0]
                .state
                .downcast_ref::<text_input::State<Renderer::Paragraph>>();

            // Pressing escape will unfocus the text field, so we should propagate that change in
            // our own model
            if text_input_state.is_focused() {
                for message in messages {
                    match message {
                        TextInputMessage::Value(s) => state.text_input_value = Some(s),
                        TextInputMessage::Submit => {
                            if let Some(normalized_value) = state
                                .text_input_value
                                .as_ref()
                                .and_then(|s| self.param.string_to_normalized_value(s))
                            {
                                shell.publish(ParamMessage::BeginSetParameter(self.param.as_ptr()));
                                self.set_normalized_value(shell, normalized_value);
                                shell.publish(ParamMessage::EndSetParameter(self.param.as_ptr()));
                            }

                            // And defocus the text input widget again
                            state.text_input_value = None;
                        }
                    }
                }
            } else {
                state.text_input_value = None;
            }

            status
        } else {
            event::Status::Ignored
        };
        if text_input_status == event::Status::Captured {
            return event::Status::Captured;
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                self.handle_mouse_down_event(tree, layout, cursor, renderer, shell)
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. } | touch::Event::FingerLost { .. }) => {
                self.handle_mouse_up_event(shell, state)
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. }) => {
                self.handle_mouse_move_event(layout, cursor, shell, state)
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                self.handle_keyboard_modifiers_changed(layout, cursor, shell, modifiers, state)
            }
            _ => event::Status::Ignored,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

// ############################################################################
// Rendering
// ############################################################################
impl<'a, P, Theme> ParamSlider<'a, P, Theme>
where
    P: Param,
    Theme: Catalog + text_input::Catalog,
{
    /// Create a temporary [`TextInput`] hooked up to [`State::text_input_value`] and outputting
    /// [`TextInputMessage`] messages and do something with it. This can be used to
    fn with_text_input<T, Renderer, BorrowedRenderer, F>(
        &self,
        layout: Layout,
        renderer: BorrowedRenderer,
        current_value: &str,
        state: &State,
        f: F,
    ) -> T
    where
        F: FnOnce(TextInput<'_, TextInputMessage, Theme, Renderer>, Layout, BorrowedRenderer) -> T,
        Renderer: TextRenderer,
        Renderer::Font: From<iced::Font>,
        BorrowedRenderer: Borrow<Renderer>,
    {
        let font = self
            .font
            .map(Renderer::Font::from)
            .unwrap_or_else(|| renderer.borrow().default_font());

        let text_size = self
            .text_size
            .unwrap_or_else(|| renderer.borrow().default_size());
        let text_width = Renderer::Paragraph::with_text(Text {
            content: current_value,
            bounds: layout.bounds().size(),
            size: text_size,
            font,
            line_height: Default::default(),
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
            shaping: Default::default(),
            wrapping: Default::default(),
        })
        .min_width();

        let text_input = text_input("", current_value)
            .id(state.text_input_id.clone())
            .font(font)
            .size(text_size)
            .width(text_width)
            .on_input(TextInputMessage::Value)
            .on_submit(TextInputMessage::Submit);

        // Make sure to not draw over the borders, and center the text
        let offset_node = layout::Node::with_children(
            Size {
                width: text_width,
                height: layout.bounds().size().height - (BORDER_WIDTH * 2.0),
            },
            vec![layout::Node::new(layout.bounds().size())],
        );
        let offset_layout = Layout::with_offset(
            Vector {
                x: layout.bounds().center_x() - (text_width / 2.0),
                y: layout.position().y + BORDER_WIDTH,
            },
            &offset_node,
        );

        f(text_input, offset_layout, renderer)
    }

    /// Draw the bar + label for this slider
    fn draw_bar<Renderer>(
        &self,
        renderer: &mut Renderer,
        style: &Style,
        bounds: &Rectangle,
        viewport: &Rectangle,
    ) where
        Renderer: TextRenderer,
        Renderer::Font: From<iced::Font>,
    {
        // We'll visualize the difference between the current value and the default value if the
        // default value lies somewhere in the middle and the parameter is continuous. Otherwise
        // this appraoch looks a bit jarring.
        let current_value = self.param.modulated_normalized_value();
        let default_value = self.param.default_normalized_value();

        let fill_start_x = util::remap_rect_x_t(
            bounds,
            if self.param.step_count().is_none() && (0.45..=0.55).contains(&default_value) {
                default_value
            } else {
                0.0
            },
        );

        let fill_end_x = util::remap_rect_x_t(bounds, current_value);
        let fill_rect = Rectangle {
            x: fill_start_x.min(fill_end_x),
            width: (fill_end_x - fill_start_x).abs(),
            ..*bounds
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds: fill_rect,
                ..Default::default()
            },
            style.bar.color,
        );

        // To make it more readable (and because it looks cool), the parts that overlap with the
        // fill rect will be rendered in white while the rest will be rendered in black.
        let display_value = self.param.to_string();

        let text_size = self.text_size.unwrap_or_else(|| renderer.default_size());
        let font = self
            .font
            .map(Renderer::Font::from)
            .unwrap_or_else(|| renderer.default_font());

        let text_bounds = Rectangle {
            x: bounds.center_x(),
            y: bounds.center_y(),
            ..*bounds
        };
        renderer.fill_text(
            text::Text {
                content: display_value.clone(),
                font: font,
                size: text_size,
                bounds: text_bounds.size(),
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                line_height: text::LineHeight::Relative(1.0),
                shaping: Default::default(),
                wrapping: Default::default(),
            },
            text_bounds.position(),
            style.background.text,
            *viewport,
        );

        // This will clip to the filled area
        renderer.with_layer(fill_rect, |renderer| {
            renderer.fill_text(
                text::Text {
                    content: display_value,
                    font: font,
                    size: text_size,
                    bounds: text_bounds.size(),
                    horizontal_alignment: alignment::Horizontal::Center,
                    vertical_alignment: alignment::Vertical::Center,
                    line_height: text::LineHeight::Relative(1.0),
                    shaping: Default::default(),
                    wrapping: Default::default(),
                },
                text_bounds.position(),
                style.bar.text,
                *viewport,
            );
        });
    }
}

// ############################################################################
// Event Handling
// ############################################################################
impl<'a, P, Theme> ParamSlider<'a, P, Theme>
where
    P: Param,
    Theme: Catalog + text_input::Catalog,
{
    /// Set the normalized value for a parameter if that would change the parameter's plain value
    /// (to avoid unnecessary duplicate parameter changes). The begin- and end set parameter
    /// messages need to be sent before calling this function.
    fn set_normalized_value(&self, shell: &mut Shell<'_, ParamMessage>, normalized_value: f32) {
        // This snaps to the nearest plain value if the parameter is stepped in some way.
        // TODO: As an optimization, we could add a `const CONTINUOUS: bool` to the parameter to
        //       avoid this normalized->plain->normalized conversion for parameters that don't need
        //       it
        let plain_value = self.param.preview_plain(normalized_value);
        let current_plain_value = self.param.modulated_plain_value();
        if plain_value != current_plain_value {
            // For the aforementioned snapping
            let normalized_plain_value = self.param.preview_normalized(plain_value);
            shell.publish(ParamMessage::SetParameterNormalized(
                self.param.as_ptr(),
                normalized_plain_value,
            ));
        }
    }

    fn handle_mouse_down_event<Renderer>(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, ParamMessage>,
    ) -> event::Status
    where
        Renderer: TextRenderer,
        Renderer::Font: From<iced::Font>,
    {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();

        let Some(cursor_position) = cursor.position_over(bounds) else {
            return event::Status::Ignored;
        };

        let click = mouse::Click::new(cursor_position, mouse::Button::Left, state.last_click);
        state.last_click = Some(click);

        if state.keyboard_modifiers.alt() {
            // Alt+click should not start a drag, instead it should show the text entry
            // widget
            state.drag_active = false;

            let text_input_id = state.text_input_id.clone();

            let current_value = self.param.to_string();
            state.text_input_value = Some(current_value.clone());

            self.with_text_input(
                layout,
                renderer,
                &current_value,
                state,
                |text_input, layout, renderer| {
                    if tree.children[0].tag != text_input.tag() {
                        tree.children[0] = Tree {
                            tag: text_input.tag(),
                            state: text_input.state(),
                            children: text_input.children(),
                        };
                    }

                    let mut move_cursor_to_end =
                        operation::text_input::move_cursor_to_end(text_input_id.clone().into());
                    let mut select_all = operation::text_input::select_all(text_input_id.into());

                    text_input.operate(
                        &mut tree.children[0],
                        layout,
                        renderer,
                        &mut move_cursor_to_end,
                    );
                    text_input.operate(&mut tree.children[0], layout, renderer, &mut select_all);
                },
            );
        } else if state.keyboard_modifiers.command()
            || matches!(click.kind(), mouse::click::Kind::Double)
        {
            // Likewise resetting a parameter should not let you immediately drag it to a new value
            state.drag_active = false;

            shell.publish(ParamMessage::BeginSetParameter(self.param.as_ptr()));
            self.set_normalized_value(shell, self.param.default_normalized_value());
            shell.publish(ParamMessage::EndSetParameter(self.param.as_ptr()));
        } else if state.keyboard_modifiers.shift() {
            shell.publish(ParamMessage::BeginSetParameter(self.param.as_ptr()));
            state.drag_active = true;

            // When holding down shift while clicking on a parameter we want to
            // granuarly edit the parameter without jumping to a new value
            state.granular_drag_start_x_value =
                Some((cursor_position.x, self.param.modulated_normalized_value()));
        } else {
            shell.publish(ParamMessage::BeginSetParameter(self.param.as_ptr()));
            state.drag_active = true;

            self.set_normalized_value(
                shell,
                util::remap_rect_x_coordinate(&bounds, cursor_position.x),
            );
            state.granular_drag_start_x_value = None;
        }

        event::Status::Captured
    }

    fn handle_mouse_up_event(
        &mut self,
        shell: &mut Shell<'_, ParamMessage>,
        state: &mut State,
    ) -> event::Status {
        if !state.drag_active {
            return event::Status::Ignored;
        }

        shell.publish(ParamMessage::EndSetParameter(self.param.as_ptr()));
        state.drag_active = false;
        event::Status::Captured
    }

    fn handle_mouse_move_event(
        &mut self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, ParamMessage>,
        state: &mut State,
    ) -> event::Status {
        // Don't do anything when we just reset the parameter because that would be weird
        if !state.drag_active {
            return event::Status::Ignored;
        }

        let bounds = layout.bounds();

        // If shift is being held then the drag should be more granular instead of
        // absolute
        if let Some(cursor_position) = cursor.position() {
            if state.keyboard_modifiers.shift() {
                let (drag_start_x, drag_start_value) =
                    *state.granular_drag_start_x_value.get_or_insert_with(|| {
                        (cursor_position.x, self.param.modulated_normalized_value())
                    });

                self.set_normalized_value(
                    shell,
                    util::remap_rect_x_coordinate(
                        &bounds,
                        util::remap_rect_x_t(&bounds, drag_start_value)
                            + (cursor_position.x - drag_start_x) * GRANULAR_DRAG_MULTIPLIER,
                    ),
                );
            } else {
                state.granular_drag_start_x_value = None;

                self.set_normalized_value(
                    shell,
                    util::remap_rect_x_coordinate(&bounds, cursor_position.x),
                );
            }
        }

        event::Status::Captured
    }

    fn handle_keyboard_modifiers_changed(
        &mut self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, ParamMessage>,
        modifiers: keyboard::Modifiers,
        state: &mut State,
    ) -> event::Status {
        state.keyboard_modifiers = modifiers;
        let bounds = layout.bounds();

        // If this happens while dragging, snap back to reality uh I mean the current screen
        // position
        if state.drag_active && state.granular_drag_start_x_value.is_some() && !modifiers.shift() {
            state.granular_drag_start_x_value = None;

            if let Some(cursor_position) = cursor.position() {
                self.set_normalized_value(
                    shell,
                    util::remap_rect_x_coordinate(&bounds, cursor_position.x),
                );
            }
        }

        event::Status::Captured
    }
}

// ############################################################################
// Styling / Appearance
// ############################################################################

/// The appearance of a slider.
pub struct Style {
    /// The [`Color`] of the slider background.
    pub background: Pair,

    /// The [`Color`] of the slider bar.
    pub bar: Pair,
    /// The border of the slider.
    pub border: Border,
}

/// The theme catalog of a [`ParamSlider`].
pub trait Catalog: Sized {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

/// A styling function for a [`ParamSlider`].
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for iced::Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(|theme: &Self, status: Status| -> Style {
            use Status::*;
            let palette = theme.extended_palette();

            let (background, bar) = match status {
                Active => (palette.background.base, palette.primary.base),
                Hovered => (palette.background.strong, palette.primary.strong),
                Dragged => (palette.background.base, palette.primary.base),
            };

            let border = Border {
                color: palette.background.base.text,
                width: BORDER_WIDTH,
                radius: 0.0.into(),
            };

            Style {
                background,
                bar,
                border,
            }
        })
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

impl<'a, P, Theme> ParamSlider<'a, P, Theme>
where
    P: Param + 'a,
    Theme: Catalog + text_input::Catalog + 'a,
{
    /// Convert this [`ParamSlider`] into an [`Element`] with the correct message. You should have a
    /// variant on your own message type that wraps around [`ParamMessage`] so you can forward those
    /// messages to
    /// [`IcedEditor::handle_param_message()`][crate::IcedEditor::handle_param_message()].
    pub fn map<Message, Renderer, F>(self, f: F) -> Element<'a, Message, Theme, Renderer>
    where
        Message: 'static,
        F: Fn(ParamMessage) -> Message + 'static,
        Renderer: TextRenderer + 'a,
        Renderer::Font: From<iced::Font>,
    {
        Element::from(self).map(f)
    }
}

impl<'a, P, Theme, Renderer> From<ParamSlider<'a, P, Theme>>
    for Element<'a, ParamMessage, Theme, Renderer>
where
    P: Param + 'a,
    Theme: Catalog + text_input::Catalog + 'a,
    Renderer: TextRenderer + 'a,
    Renderer::Font: From<iced::Font>,
{
    fn from(widget: ParamSlider<'a, P, Theme>) -> Self {
        Element::new(widget)
    }
}
