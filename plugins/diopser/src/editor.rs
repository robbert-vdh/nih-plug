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

use nih_plug::prelude::Editor;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::*;
use nih_plug_vizia::{assets, create_vizia_editor, ViziaState, ViziaTheming};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use self::button::SafeModeButton;
use crate::DiopserParams;

mod button;

/// VIZIA uses points instead of pixels for text
const POINT_SCALE: f32 = 0.75;

#[derive(Lens)]
struct Data {
    params: Arc<DiopserParams>,

    /// Whether the safe mode button is enabled. The number of filter stages is capped at 40 while
    /// this is active.
    safe_mode: Arc<AtomicBool>,
}

impl Model for Data {}

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::from_size(600, 490)
}

pub(crate) fn create(
    params: Arc<DiopserParams>,
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, ViziaTheming::Custom, move |cx, _| {
        assets::register_noto_sans_light(cx);
        assets::register_noto_sans_thin(cx);

        cx.add_theme(include_str!("theme.css"));

        Data {
            params: params.clone(),

            safe_mode: params.safe_mode.clone(),
        }
        .build(cx);

        ResizeHandle::new(cx);

        VStack::new(cx, |cx| {
            top_bar(cx);
            spectrum_analyzer(cx);
            other_params(cx);
        });
    })
}

/// This contain's the plugin's name, a bypass button, and some other controls.
fn top_bar(cx: &mut Context) {
    HStack::new(cx, |cx| {
        Label::new(cx, "Diopser")
            .font(assets::NOTO_SANS_THIN)
            .font_size(50.0 * POINT_SCALE)
            .top(Pixels(-2.0))
            .left(Pixels(7.0));

        HStack::new(cx, |cx| {
            ParamSlider::new(cx, Data::params, |params| &params.automation_precision)
                .with_label("Automation Precision");

            SafeModeButton::new(cx, Data::safe_mode, "Safe mode").left(Pixels(10.0));

            ParamButton::new(cx, Data::params, |params| &params.bypass)
                .for_bypass()
                .left(Pixels(10.0));
        })
        .child_space(Pixels(10.0))
        .left(Stretch(1.0));
    })
    .id("top-bar");
}

/// This shows a spectrum analyzer for the plugin's output, and also acts as an X-Y pad for the
/// frequency and resonance parameters.
fn spectrum_analyzer(cx: &mut Context) {
    Label::new(cx, "When I grow up, I want to be a spectrum analyzer!");
}

/// The area below the spectrum analyzer that contains all of the other parameters.
fn other_params(cx: &mut Context) {
    // Just to center the old generic UI for now
    ZStack::new(cx, |cx| {
        GenericUi::new(cx, Data::params).width(Auto).height(Auto);
    })
    .width(Percentage(100.0))
    .height(Stretch(1.0))
    .child_space(Stretch(1.0));
}
