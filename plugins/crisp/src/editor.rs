// Crisp: a distortion plugin but not quite
// Copyright (C) 2022 Robbert van der Helm
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

use nih_plug::prelude::Editor;
use nih_plug_egui::widgets::generic_ui;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::pin::Pin;
use std::sync::Arc;

use crate::CrispParams;

// Makes sense to also define this here, makes it a bit easier to keep track of
pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(250, 350)
}

pub fn create(
    params: Pin<Arc<CrispParams>>,
    editor_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(editor_state, (), move |egui_ctx, setter, _state| {
        egui::CentralPanel::default().show(egui_ctx, |ui| {
            generic_ui::create(ui, params.as_ref(), setter, generic_ui::GenericSlider);
        });
    })
}
