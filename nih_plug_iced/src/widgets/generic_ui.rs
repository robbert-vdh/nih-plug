//! A simple generic UI widget that renders all parameters in a [`Params`] object as a scrollable
//! list of sliders and labels.

use crate::core::widget::{Id, Operation, Tree};
use crate::core::{
    alignment, event, layout, renderer, text, Clipboard, Element, Layout, Length, Rectangle, Shell,
    Size, Widget,
};
use crate::widget::{self, row, scrollable, Column, Scrollable, Space};
use std::marker::PhantomData;
use std::sync::Arc;

use nih_plug::prelude::{Param, ParamFlags, ParamPtr, Params};

use super::{ParamMessage, ParamSlider};

/// A widget that can be used to create a generic UI with. This is used in conjuction with empty
/// structs to emulate existential types.
pub trait ParamWidget<Theme = crate::Theme, Renderer = crate::Renderer> {
    /// Create an [`Element`] for a widget for the specified parameter.
    fn into_widget_element<'a, P: Param>(
        param: &'a P,
    ) -> Element<'a, ParamMessage, Theme, Renderer>
    where
        Theme: 'a,
        Renderer: 'a;

    /// The same as [`into_widget_element()`][Self::into_widget_element()], but for a `ParamPtr`.
    ///
    /// # Safety
    ///
    /// Undefined behavior of the `ParamPtr` does not point to a valid parameter.
    unsafe fn into_widget_element_raw<'a>(
        param: &'a ParamPtr,
    ) -> Element<'a, ParamMessage, Theme, Renderer>
    where
        Theme: 'a,
        Renderer: 'a,
    {
        match param {
            ParamPtr::FloatParam(p) => Self::into_widget_element(&**p),
            ParamPtr::IntParam(p) => Self::into_widget_element(&**p),
            ParamPtr::BoolParam(p) => Self::into_widget_element(&**p),
            ParamPtr::EnumParam(p) => Self::into_widget_element(&**p),
        }
    }
}

/// Create a generic UI using [`ParamSlider`]s.
#[derive(Default)]
pub struct GenericSlider;

/// A list of scrollable widgets for every paramter in a [`Params`] object. The [`ParamWidget`] type
/// determines what widget to use for this.
///
/// TODO: There's no way to configure the individual widgets.
pub struct GenericUi<W, Theme = crate::Theme, Renderer = crate::Renderer> {
    // Hacky work around so we can borrow &ParamPtr and ensure references
    // stay alive for the lifetime of this object.
    params: Vec<ParamPtr>,

    id: Option<Id>,
    width: Length,
    height: Length,
    max_width: u16,
    max_height: u16,

    pad_scrollbar: bool,

    /// We don't emit any messages or store the actual widgets, but iced requires us to define some
    /// message type anyways.
    _phantom: PhantomData<(W, Theme, Renderer)>,
}

impl<W, Theme, Renderer> GenericUi<W, Theme, Renderer> {
    /// Creates a new [`GenericUi`] for all provided parameters.
    pub fn new(params: Arc<dyn Params>) -> Self {
        let params = params
            .param_map()
            .into_iter()
            .map(|(_, ptr, _)| ptr)
            .collect();
        Self {
            id: None,
            params,

            width: Length::Fill,
            height: Length::Fill,
            max_width: u16::MAX,
            max_height: u16::MAX,
            pad_scrollbar: false,

            _phantom: PhantomData,
        }
    }

    /// Sets the [`Id`] of the [`Container`].
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the width of the [`GenericUi`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`GenericUi`].
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets the maximum width of the [`GenericUi`].
    pub fn max_width(mut self, width: u16) -> Self {
        self.max_width = width;
        self
    }

    /// Sets the maximum height of the [`GenericUi`].
    pub fn max_height(mut self, height: u16) -> Self {
        self.max_height = height;
        self
    }

    /// Include additional room on the right for the scroll bar.
    pub fn pad_scrollbar(mut self) -> Self {
        self.pad_scrollbar = true;
        self
    }
}

impl<'a, W, Theme, Renderer> GenericUi<W, Theme, Renderer>
where
    W: ParamWidget<Theme, Renderer>,
    Theme: scrollable::Catalog + widget::text::Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn content(
        &'a self,
        renderer: Option<&Renderer>,
    ) -> Scrollable<'a, ParamMessage, Theme, Renderer> {
        let (spacing, padding) = match renderer {
            Some(renderer) => (
                (renderer.default_size() * 0.2).0.round(),
                (renderer.default_size() * 0.5).0.round(),
            ),
            None => (0.0, 0.0),
        };

        let content = Column::with_children(
            self.params
                .iter()
                .filter(|param| is_hidden(*param))
                .map(|param| {
                    let row = row![
                        widget::text(unsafe { param.name() })
                            .height(20)
                            .width(Length::Fill)
                            .align_x(alignment::Horizontal::Right)
                            .align_y(alignment::Vertical::Center),
                        unsafe { W::into_widget_element_raw(param) }
                    ]
                    .width(Length::Fill)
                    .align_y(alignment::Vertical::Center)
                    .spacing(spacing * 2.0);

                    if self.pad_scrollbar {
                        row.push(Space::with_width(0))
                    } else {
                        row
                    }
                })
                .map(Element::from),
        )
        .align_x(alignment::Horizontal::Center)
        .spacing(spacing)
        .padding(padding)
        .width(self.width)
        .height(self.height)
        .max_width(self.max_width);

        scrollable(content).spacing(spacing)
    }
}

impl<'a, W, Theme, Renderer> Widget<ParamMessage, Theme, Renderer> for GenericUi<W, Theme, Renderer>
where
    W: ParamWidget<Theme, Renderer>,
    Theme: scrollable::Catalog + widget::text::Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn size(&self) -> iced_baseview::Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn children(&self) -> Vec<Tree> {
        let content = self.content(None);

        vec![Tree::new(
            &content as &dyn Widget<ParamMessage, Theme, Renderer>,
        )]
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content(Some(renderer))
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn draw(
        &self,
        tree: &iced_baseview::core::widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: iced_baseview::core::mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content(Some(renderer)).draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        )
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(self.id.as_ref(), layout.bounds(), &mut |operation| {
            self.content(Some(renderer)).operate(
                tree,
                layout.children().next().unwrap(),
                renderer,
                operation,
            )
        });
    }

    fn on_event(
        &mut self,
        tree: &mut iced_baseview::core::widget::Tree,
        event: event::Event,
        layout: Layout<'_>,
        cursor: iced_baseview::core::mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, ParamMessage>,
        viewport: &Rectangle,
    ) -> event::Status {
        self.content(Some(renderer)).on_event(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: iced_baseview::core::mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> iced_baseview::core::mouse::Interaction {
        self.content(Some(renderer)).mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
}

fn is_hidden(param_ptr: &ParamPtr) -> bool {
    let flags = unsafe { param_ptr.flags() };
    flags.contains(ParamFlags::HIDE_IN_GENERIC_UI)
}

impl<Theme, Renderer> ParamWidget<Theme, Renderer> for GenericSlider
where
    Theme: widget::text_input::Catalog,
    Renderer: text::Renderer,
    Renderer::Font: From<crate::Font>,
{
    fn into_widget_element<'a, P: Param>(param: &'a P) -> Element<'a, ParamMessage, Theme, Renderer>
    where
        Theme: 'a,
        Renderer: 'a,
    {
        ParamSlider::new(param).into()
    }
}

impl<'a, W, Theme, Renderer> GenericUi<W, Theme, Renderer>
where
    W: ParamWidget<Theme, Renderer> + 'a,
    Theme: scrollable::Catalog + widget::text::Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    /// Convert this [`GenericUi`] into an [`Element`] with the correct message. You should have a
    /// variant on your own message type that wraps around [`ParamMessage`] so you can forward those
    /// messages to
    /// [`IcedEditor::handle_param_message()`][crate::IcedEditor::handle_param_message()].
    pub fn map<Message, F>(self, f: F) -> Element<'a, Message, Theme, Renderer>
    where
        Message: 'static,
        F: Fn(ParamMessage) -> Message + 'static,
    {
        Element::from(self).map(f)
    }
}

impl<'a, W, Theme, Renderer> From<GenericUi<W, Theme, Renderer>>
    for Element<'a, ParamMessage, Theme, Renderer>
where
    W: ParamWidget<Theme, Renderer> + 'a,
    Theme: scrollable::Catalog + widget::text::Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(widget: GenericUi<W, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}
