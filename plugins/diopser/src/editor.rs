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
}

impl IcedEditor for DiopserEditor {
    type Executor = nih_plug_iced::executor::Default;
    // TODO:
    type Message = ();
    type InitializationFlags = Pin<Arc<DiopserParams>>;

    fn new(
        params: Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>) {
        let editor = DiopserEditor { params, context };

        (editor, Command::none())
    }

    fn update(
        &mut self,
        window: &mut nih_plug_iced::WindowQueue,
        message: Self::Message,
    ) -> Command<Self::Message> {
        // TODO:
        Command::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        nih_plug_iced::Text::new("Hello, world!").into()
    }
}
