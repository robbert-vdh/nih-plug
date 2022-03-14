use nih_plug::prelude::{Editor, GuiContext};
use nih_plug_iced::widgets as nih_widgets;
use nih_plug_iced::*;
use std::pin::Pin;
use std::sync::Arc;

use crate::GainParams;

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<IcedState> {
    IcedState::from_size(200, 150)
}

pub(crate) fn create(
    params: Pin<Arc<GainParams>>,
    editor_state: Arc<IcedState>,
) -> Option<Box<dyn Editor>> {
    create_iced_editor::<GainEditor>(editor_state, params)
}

struct GainEditor {
    params: Pin<Arc<GainParams>>,
    context: Arc<dyn GuiContext>,

    gain_slider_state: nih_widgets::param_slider::State,
    meter_dummy_state: widget::button::State,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    /// Update a parameter's value.
    ParamUpdate(nih_widgets::ParamMessage),
}

impl IcedEditor for GainEditor {
    type Executor = executor::Default;
    type Message = Message;
    type InitializationFlags = Pin<Arc<GainParams>>;

    fn new(
        params: Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>) {
        let editor = GainEditor {
            params,
            context,

            gain_slider_state: Default::default(),
            meter_dummy_state: widget::button::State::new(),
        };

        (editor, Command::none())
    }

    fn context(&self) -> &dyn GuiContext {
        self.context.as_ref()
    }

    fn update(
        &mut self,
        _window: &mut WindowQueue,
        message: Self::Message,
    ) -> Command<Self::Message> {
        match message {
            Message::ParamUpdate(message) => self.handle_param_message(message),
        }

        Command::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        Column::new()
            .align_items(Alignment::Center)
            .push(
                Text::new("Gain GUI")
                    .font(assets::NOTO_SANS_LIGHT)
                    .size(40)
                    .height(50.into())
                    .width(Length::Fill)
                    .horizontal_alignment(alignment::Horizontal::Center)
                    .vertical_alignment(alignment::Vertical::Bottom),
            )
            .push(
                Text::new("Gain")
                    .height(20.into())
                    .width(Length::Fill)
                    .horizontal_alignment(alignment::Horizontal::Center)
                    .vertical_alignment(alignment::Vertical::Center),
            )
            .push(
                nih_widgets::ParamSlider::new(&mut self.gain_slider_state, &self.params.gain, self.context.as_ref()).map(Message::ParamUpdate)
                // Button::new(&mut self.gain_dummy_state, Text::new("Gain"))
                //     .height(30.into())
                //     .width(180.into()),
            )
            .push(Space::with_height(10.into()))
            .push(
                Button::new(&mut self.meter_dummy_state, Text::new("Meter"))
                    .height(15.into())
                    .width(180.into()),
            )
            .push(
                Text::new("Ticks 'n stuff")
                    .size(12)
                    .height(15.into())
                    .width(Length::Fill)
                    .horizontal_alignment(alignment::Horizontal::Center)
                    .vertical_alignment(alignment::Vertical::Center),
            )
            .into()
    }

    fn background_color(&self) -> nih_plug_iced::Color {
        nih_plug_iced::Color {
            r: 0.98,
            g: 0.98,
            b: 0.98,
            a: 1.0,
        }
    }
}
