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

use nih_plug::prelude::{Editor, GuiContext};
use nih_plug_iced::widgets as nih_widgets;
use nih_plug_iced::widgets::generic_ui::GenericUi;
use nih_plug_iced::*;
use std::pin::Pin;
use std::sync::Arc;

use crate::DiopserParams;

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<IcedState> {
    IcedState::from_size(360, 300)
}

pub(crate) fn create(
    params: Pin<Arc<DiopserParams>>,
    editor_state: Arc<IcedState>,
) -> Option<Box<dyn Editor>> {
    create_iced_editor::<DiopserEditor>(editor_state, params)
}

struct DiopserEditor {
    params: Pin<Arc<DiopserParams>>,
    context: Arc<dyn GuiContext>,

    generic_ui_state: nih_widgets::generic_ui::State<nih_widgets::generic_ui::GenericSlider>,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    /// Update a parameter's value.
    ParamUpdate(nih_widgets::ParamMessage),
}

impl IcedEditor for DiopserEditor {
    type Executor = executor::Default;
    type Message = Message;
    type InitializationFlags = Pin<Arc<DiopserParams>>;

    fn new(
        params: Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>) {
        let editor = DiopserEditor {
            params,
            context,

            generic_ui_state: Default::default(),
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
        GenericUi::new(
            &mut self.generic_ui_state,
            self.params.as_ref(),
            self.context.as_ref(),
        )
        .map(Message::ParamUpdate)
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
