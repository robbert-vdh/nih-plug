// Diopser: a phase rotation plugin
// Copyright (C) 2021-2024 Robbert van der Helm
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

use atomic_float::AtomicF32;
use nih_plug::debug::*;
use nih_plug::prelude::{Editor, Plugin};
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::*;
use nih_plug_vizia::{assets, create_vizia_editor, ViziaState, ViziaTheming};
use std::sync::{Arc, Mutex};

use self::button::SafeModeButton;
use self::slider::RestrictedParamSlider;
use crate::params::DiopserParams;
use crate::spectrum::SpectrumOutput;
use crate::Diopser;

mod analyzer;
mod button;
mod safe_mode;
mod slider;
mod xy_pad;

pub use safe_mode::SafeModeClamper;

const EDITOR_WIDTH: u32 = 600;
const EDITOR_HEIGHT: u32 = 490;

const SPECTRUM_ANALYZER_HEIGHT: f32 = 260.0;

const DARK_GRAY: Color = Color::rgb(0xc4, 0xc4, 0xc4);
const DARKER_GRAY: Color = Color::rgb(0x69, 0x69, 0x69);

#[derive(Lens, Clone)]
pub(crate) struct Data {
    pub(crate) params: Arc<DiopserParams>,

    /// The plugin's current sample rate.
    pub(crate) sample_rate: Arc<AtomicF32>,
    pub(crate) spectrum: Arc<Mutex<SpectrumOutput>>,
    /// Whether the safe mode button is enabled. The number of filter stages is capped at 40 while
    /// this is active.
    pub(crate) safe_mode_clamper: SafeModeClamper,
}

impl Model for Data {}

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::new(|| (EDITOR_WIDTH, EDITOR_HEIGHT))
}

pub(crate) fn create(editor_data: Data, editor_state: Arc<ViziaState>) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, ViziaTheming::Custom, move |cx, _| {
        assets::register_noto_sans_light(cx);
        assets::register_noto_sans_thin(cx);

        if let Err(err) = cx.add_stylesheet(include_style!("src/editor/theme.css")) {
            nih_error!("Failed to load stylesheet: {err:?}")
        }

        editor_data.clone().build(cx);

        VStack::new(cx, |cx| {
            top_bar(cx);
            spectrum_analyzer(cx);
            other_params(cx);
        });

        ResizeHandle::new(cx);
    })
}

/// This contain's the plugin's name, a bypass button, and some other controls.
fn top_bar(cx: &mut Context) {
    HStack::new(cx, |cx| {
        Label::new(cx, "Diopser")
            .font_family(vec![FamilyOwned::Name(String::from(assets::NOTO_SANS))])
            .font_weight(FontWeightKeyword::Thin)
            .font_size(37.0)
            .top(Pixels(2.0))
            .left(Pixels(8.0))
            .on_mouse_down(|_, _| {
                // FIXME: On Windows this blocks, and while this is blocking a timer may proc which
                //        causes the window state to be mutably borrowed again, resulting in a
                //        panic. This needs to be fixed in baseview first.
                if cfg!(not(windows)) {
                    // Try to open the Diopser plugin's page when clicking on the title. If this
                    // fails then that's not a problem
                    let result = open::that(Diopser::URL);
                    if cfg!(debug_assertions) && result.is_err() {
                        nih_debug_assert_failure!("Failed to open web browser: {:?}", result);
                    }
                }
            });
        Label::new(cx, Diopser::VERSION)
            .color(DARKER_GRAY)
            .top(Stretch(1.0))
            .bottom(Pixels(7.5))
            .left(Pixels(2.0));

        HStack::new(cx, |cx| {
            ParamSlider::new(cx, Data::params, |params| &params.automation_precision)
                .with_label("Automation Precision")
                .id("automation-precision");

            SafeModeButton::new(cx, Data::safe_mode_clamper, "Safe mode").left(Pixels(10.0));

            ParamButton::new(cx, Data::params, |params| &params.bypass)
                .for_bypass()
                .left(Pixels(10.0));
        })
        .width(Auto)
        .child_space(Pixels(10.0))
        .left(Stretch(1.0));
    })
    .id("top-bar");
}

/// This shows a spectrum analyzer for the plugin's output, and also acts as an X-Y pad for the
/// frequency and resonance parameters.
fn spectrum_analyzer(cx: &mut Context) {
    const LABEL_HEIGHT: f32 = 20.0;

    HStack::new(cx, |cx| {
        Label::new(cx, "Resonance")
            .font_size(18.0)
            .rotate(Angle::Deg(270.0f32))
            .width(Pixels(LABEL_HEIGHT))
            .height(Pixels(SPECTRUM_ANALYZER_HEIGHT))
            // HACK: The `.space()` on the HStack doesn't seem to work correctly here
            .left(Pixels(10.0))
            .right(Pixels(-5.0))
            .child_space(Stretch(1.0));

        VStack::new(cx, |cx| {
            ZStack::new(cx, |cx| {
                analyzer::SpectrumAnalyzer::new(cx, Data::spectrum, Data::sample_rate, {
                    let safe_mode_clamper = Data::safe_mode_clamper.get(cx);
                    move |t| safe_mode_clamper.filter_frequency_renormalize_display(t)
                })
                .width(Percentage(100.0))
                .height(Percentage(100.0));

                xy_pad::XyPad::new(
                    cx,
                    Data::params,
                    |params| &params.filter_frequency,
                    |params| &params.filter_resonance,
                    {
                        let safe_mode_clamper = Data::safe_mode_clamper.get(cx);
                        move |t| safe_mode_clamper.filter_frequency_renormalize_display(t)
                    },
                    {
                        let safe_mode_clamper = Data::safe_mode_clamper.get(cx);
                        move |t| safe_mode_clamper.filter_frequency_renormalize_event(t)
                    },
                )
                .width(Percentage(100.0))
                .height(Percentage(100.0));
            })
            .width(Percentage(100.0))
            .background_color(DARK_GRAY)
            .height(Pixels(SPECTRUM_ANALYZER_HEIGHT));

            Label::new(cx, "Frequency")
                .font_size(18.0)
                .top(Pixels(2.0))
                .width(Stretch(1.0))
                .height(Pixels(20.0))
                .child_space(Stretch(1.0));
        })
        .left(Pixels(10.0))
        .right(Pixels(10.0))
        .top(Pixels(10.0))
        .height(Auto)
        .width(Stretch(1.0));
    })
    .height(Auto);
}

/// The area below the spectrum analyzer that contains all of the other parameters.
fn other_params(cx: &mut Context) {
    VStack::new(cx, |cx| {
        HStack::new(cx, |cx| {
            Label::new(cx, "Filter Stages").class("param-label");
            RestrictedParamSlider::new(
                cx,
                Data::params,
                |params| &params.filter_stages,
                {
                    let safe_mode_clamper = Data::safe_mode_clamper.get(cx);
                    move |t| safe_mode_clamper.filter_stages_renormalize_display(t)
                },
                {
                    let safe_mode_clamper = Data::safe_mode_clamper.get(cx);
                    move |t| safe_mode_clamper.filter_stages_renormalize_event(t)
                },
            );
        })
        .size(Auto)
        .bottom(Pixels(10.0));

        HStack::new(cx, |cx| {
            Label::new(cx, "Frequency Spread").class("param-label");
            ParamSlider::new(cx, Data::params, |params| &params.filter_spread_octaves);
        })
        .size(Auto)
        .bottom(Pixels(10.0));

        HStack::new(cx, |cx| {
            Label::new(cx, "Spread Style").class("param-label");
            ParamSlider::new(cx, Data::params, |params| &params.filter_spread_style)
                .set_style(ParamSliderStyle::CurrentStepLabeled { even: true });
        })
        .size(Auto);
    })
    .id("param-sliders")
    .width(Percentage(100.0))
    // This should take up all remaining space
    .height(Stretch(1.0))
    .child_space(Stretch(1.0));
}
