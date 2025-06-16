use atomic_float::AtomicF32;
use nih_plug::prelude::{util, Editor, GuiContext};
use nih_plug_iced::assets::noto_sans_fonts_data;
use nih_plug_iced::widget::{column, text, Space};
use nih_plug_iced::widgets as nih_widgets;
use nih_plug_iced::*;
use std::sync::Arc;
use std::time::Duration;

use crate::GainParams;

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<IcedState> {
    IcedState::from_size(200, 150)
}

pub(crate) fn create(
    params: Arc<GainParams>,
    peak_meter: Arc<AtomicF32>,
    editor_state: Arc<IcedState>,
) -> Option<Box<dyn Editor>> {
    create_iced_editor::<GainEditor>(
        editor_state,
        (params, peak_meter),
        noto_sans_fonts_data().into(),
    )
}

struct GainEditor {
    params: Arc<GainParams>,
    context: Arc<dyn GuiContext>,

    peak_meter: Arc<AtomicF32>,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    /// Update a parameter's value.
    ParamUpdate(nih_widgets::ParamMessage),
}

impl IcedEditor for GainEditor {
    type Executor = executor::Default;
    type Message = Message;
    type InitializationFlags = (Arc<GainParams>, Arc<AtomicF32>);
    type Theme = Theme;

    fn new(
        (params, peak_meter): Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Task<Self::Message>) {
        let editor = GainEditor {
            params,
            context,

            peak_meter,
        };

        (editor, Task::none())
    }

    fn context(&self) -> &dyn GuiContext {
        self.context.as_ref()
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::ParamUpdate(message) => self.handle_param_message(message),
        }

        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let title = text("Gain GUI")
            .font(assets::NOTO_SANS_LIGHT)
            .size(40)
            .height(50)
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Bottom);

        let text = text("Gain").height(20).width(Length::Fill).center();

        column![
            title,
            text,
            nih_widgets::ParamSlider::new(&self.params.gain).map(Message::ParamUpdate),
            Space::with_height(10),
            nih_widgets::PeakMeter::new(util::gain_to_db(
                self.peak_meter.load(std::sync::atomic::Ordering::Relaxed),
            ))
            .hold_time(Duration::from_millis(600))
        ]
        .align_x(alignment::Horizontal::Center)
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
