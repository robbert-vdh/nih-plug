// Diopser: a phase rotation plugin
// Copyright (C) 2021-2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use nih_plug::prelude::{Editor, GuiContext, Param};
use nih_plug_iced::widgets::ParamMessage;
use nih_plug_iced::{create_iced_editor, Command, Element, IcedEditor, IcedState};
use std::pin::Pin;
use std::sync::Arc;

use crate::DiopserParams;

// Makes sense to also define this here, makes it a bit easier to keep track of
pub fn default_state() -> Arc<IcedState> {
    IcedState::from_size(600, 400)
}

pub fn create(
    params: Pin<Arc<DiopserParams>>,
    editor_state: Arc<IcedState>,
) -> Option<Box<dyn Editor>> {
    create_iced_editor::<DiopserEditor>(editor_state, params)
}

struct DiopserEditor {
    params: Pin<Arc<DiopserParams>>,
    context: Arc<dyn GuiContext>,

    // FIXME: All of this is just to test the reactivity
    button_state: nih_plug_iced::widget::button::State,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    /// Update a parameter's value.
    ParamUpdate(ParamMessage),
}

impl IcedEditor for DiopserEditor {
    type Executor = nih_plug_iced::executor::Default;
    type Message = Message;
    type InitializationFlags = Pin<Arc<DiopserParams>>;

    fn new(
        params: Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>) {
        let editor = DiopserEditor {
            params,
            context,
            button_state: Default::default(),
        };

        (editor, Command::none())
    }

    fn context(&self) -> &dyn GuiContext {
        self.context.as_ref()
    }

    fn update(
        &mut self,
        _window: &mut nih_plug_iced::WindowQueue,
        message: Self::Message,
    ) -> Command<Self::Message> {
        match message {
            Message::ParamUpdate(message) => self.handle_param_message(message),
        }

        Command::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        nih_plug_iced::Row::new()
            .height(nih_plug_iced::Length::Fill)
            .align_items(nih_plug_iced::Alignment::Center)
            .push(
                nih_plug_iced::Column::new()
                    .width(nih_plug_iced::Length::Fill)
                    .align_items(nih_plug_iced::Alignment::Center)
                    .push(nih_plug_iced::Text::new(format!(
                        "{} filters active",
                        self.params.filter_stages.value
                    )))
                    .push(
                        nih_plug_iced::Button::new(
                            &mut self.button_state,
                            nih_plug_iced::Text::new("MAXIMUM POWAH"),
                        )
                        .on_press(Message::ParamUpdate(
                            ParamMessage::SetParameterNormalized(
                                self.params.filter_stages.as_ptr(),
                                1.0,
                            ),
                        )),
                    ),
            )
            .into()
    }
}
