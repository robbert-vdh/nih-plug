//! A simple generic UI widget that renders all parameters in a [`Params`] object as a scrollable
//! list of sliders and labels.

use atomic_refcell::AtomicRefCell;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use nih_plug::prelude::{Param, ParamFlags, ParamPtr, Params};

use super::{ParamMessage, ParamSlider};
use crate::backend::Renderer;
use crate::text::Renderer as TextRenderer;
use crate::{
    alignment, event, layout, renderer, widget, Alignment, Clipboard, Element, Event, Layout,
    Length, Point, Rectangle, Row, Scrollable, Shell, Space, Text, Widget,
};

/// A widget that can be used to create a generic UI with. This is used in conjuction with empty
/// structs to emulate existential types.
pub trait ParamWidget {
    /// The type of state stores by this parameter type.
    type State: Default;

    /// Create an [`Element`] for a widget for the specified parameter.
    fn into_widget_element<'a, P: Param>(
        param: &'a P,
        state: &'a mut Self::State,
    ) -> Element<'a, ParamMessage>;

    /// The same as [`into_widget_element()`][Self::into_widget_element()], but for a `ParamPtr`.
    ///
    /// # Safety
    ///
    /// Undefined behavior of the `ParamPtr` does not point to a valid parameter.
    unsafe fn into_widget_element_raw<'a>(
        param: &ParamPtr,
        state: &'a mut Self::State,
    ) -> Element<'a, ParamMessage> {
        match param {
            ParamPtr::FloatParam(p) => Self::into_widget_element(&**p, state),
            ParamPtr::IntParam(p) => Self::into_widget_element(&**p, state),
            ParamPtr::BoolParam(p) => Self::into_widget_element(&**p, state),
            ParamPtr::EnumParam(p) => Self::into_widget_element(&**p, state),
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
pub struct GenericUi<'a, W: ParamWidget> {
    state: &'a mut State<W>,

    params: Arc<dyn Params>,

    width: Length,
    height: Length,
    max_width: u32,
    max_height: u32,
    pad_scrollbar: bool,

    /// We don't emit any messages or store the actual widgets, but iced requires us to define some
    /// message type anyways.
    _phantom: PhantomData<W>,
}

/// State for a [`GenericUi`].
#[derive(Debug, Default)]
pub struct State<W: ParamWidget> {
    /// The internal state for each parameter's widget.
    scrollable_state: AtomicRefCell<widget::scrollable::State>,
    /// The internal state for each parameter's widget.
    widget_state: AtomicRefCell<HashMap<ParamPtr, W::State>>,
}

impl<'a, W> GenericUi<'a, W>
where
    W: ParamWidget,
{
    /// Creates a new [`GenericUi`] for all provided parameters.
    pub fn new(state: &'a mut State<W>, params: Arc<dyn Params>) -> Self {
        Self {
            state,

            params,

            width: Length::Fill,
            height: Length::Fill,
            max_width: u32::MAX,
            max_height: u32::MAX,
            pad_scrollbar: false,

            _phantom: PhantomData,
        }
    }

    /// Sets the width of the [`GenericUi`].
    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    /// Sets the height of the [`GenericUi`].
    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    /// Sets the maximum width of the [`GenericUi`].
    pub fn max_width(mut self, width: u32) -> Self {
        self.max_width = width;
        self
    }

    /// Sets the maximum height of the [`GenericUi`].
    pub fn max_height(mut self, height: u32) -> Self {
        self.max_height = height;
        self
    }

    /// Include additional room on the right for the scroll bar.
    pub fn pad_scrollbar(mut self) -> Self {
        self.pad_scrollbar = true;
        self
    }

    /// Create a temporary [`Scrollable`]. This needs to be created on demand because it needs to
    /// mutably borrow the `Scrollable`'s widget state.
    fn with_scrollable_widget<T, R, F>(
        &'a self,
        scrollable_state: &'a mut widget::scrollable::State,
        widget_state: &'a mut HashMap<ParamPtr, W::State>,
        renderer: R,
        f: F,
    ) -> T
    where
        F: FnOnce(Scrollable<'a, ParamMessage>, R) -> T,
        R: Borrow<Renderer>,
    {
        let text_size = renderer.borrow().default_size();
        let spacing = (text_size as f32 * 0.2).round() as u16;
        let padding = (text_size as f32 * 0.5).round() as u16;

        let mut scrollable = Scrollable::new(scrollable_state)
            .width(self.width)
            .height(self.height)
            .max_width(self.max_width)
            .max_height(self.max_height)
            .spacing(spacing)
            .padding(padding)
            .align_items(Alignment::Center);

        // Make sure we already have widget state for each widget
        let param_map = self.params.param_map();
        for (_, param_ptr, _) in &param_map {
            let flags = unsafe { param_ptr.flags() };
            if flags.contains(ParamFlags::HIDE_IN_GENERIC_UI) {
                continue;
            }

            if !widget_state.contains_key(param_ptr) {
                widget_state.insert(*param_ptr, Default::default());
            }
        }

        for (_, param_ptr, _) in param_map {
            let flags = unsafe { param_ptr.flags() };
            if flags.contains(ParamFlags::HIDE_IN_GENERIC_UI) {
                continue;
            }

            // SAFETY: We only borrow each item once, and the plugin framework statically asserted
            //         that parameter indices are unique and this widget state cannot outlive this
            //         function
            let widget_state: &'a mut W::State =
                unsafe { &mut *(widget_state.get_mut(&param_ptr).unwrap() as *mut _) };

            // Show the label next to the parameter for better use of the space
            let mut row = Row::new()
                .width(Length::Fill)
                .align_items(Alignment::Center)
                .spacing(spacing * 2)
                .push(
                    Text::new(unsafe { param_ptr.name() })
                        .height(20.into())
                        .width(Length::Fill)
                        .horizontal_alignment(alignment::Horizontal::Right)
                        .vertical_alignment(alignment::Vertical::Center),
                )
                .push(unsafe { W::into_widget_element_raw(&param_ptr, widget_state) });
            if self.pad_scrollbar {
                // There's already spacing applied, so this element doesn't actually need to hae any
                // size of its own
                row = row.push(Space::with_width(Length::Units(0)));
            }

            scrollable = scrollable.push(row);
        }

        f(scrollable, renderer)
    }
}

impl<'a, W> Widget<ParamMessage, Renderer> for GenericUi<'a, W>
where
    W: ParamWidget,
{
    fn width(&self) -> Length {
        self.width
    }

    fn height(&self) -> Length {
        self.height
    }

    fn layout(&self, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let mut scrollable_state = self.state.scrollable_state.borrow_mut();
        let mut widget_state = self.state.widget_state.borrow_mut();
        self.with_scrollable_widget(
            &mut scrollable_state,
            &mut widget_state,
            renderer,
            |scrollable, _| scrollable.layout(renderer, limits),
        )
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor_position: Point,
        viewport: &Rectangle,
    ) {
        let mut scrollable_state = self.state.scrollable_state.borrow_mut();
        let mut widget_state = self.state.widget_state.borrow_mut();
        self.with_scrollable_widget(
            &mut scrollable_state,
            &mut widget_state,
            renderer,
            |scrollable, renderer| {
                scrollable.draw(renderer, style, layout, cursor_position, viewport)
            },
        )
    }

    fn on_event(
        &mut self,
        event: Event,
        layout: Layout<'_>,
        cursor_position: Point,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, ParamMessage>,
    ) -> event::Status {
        let mut scrollable_state = self.state.scrollable_state.borrow_mut();
        let mut widget_state = self.state.widget_state.borrow_mut();
        self.with_scrollable_widget(
            &mut scrollable_state,
            &mut widget_state,
            renderer,
            |mut scrollable, _| {
                scrollable.on_event(event, layout, cursor_position, renderer, clipboard, shell)
            },
        )
    }
}

impl ParamWidget for GenericSlider {
    type State = super::param_slider::State;

    fn into_widget_element<'a, P: Param>(
        param: &'a P,
        state: &'a mut Self::State,
    ) -> Element<'a, ParamMessage> {
        ParamSlider::new(state, param).into()
    }
}

impl<'a, W: ParamWidget> GenericUi<'a, W> {
    /// Convert this [`GenericUi`] into an [`Element`] with the correct message. You should have a
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

impl<'a, W> From<GenericUi<'a, W>> for Element<'a, ParamMessage>
where
    W: ParamWidget,
{
    fn from(widget: GenericUi<'a, W>) -> Self {
        Element::new(widget)
    }
}
