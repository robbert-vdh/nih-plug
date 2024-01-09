// Spectral Compressor: an FFT based compressor
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
use crossbeam::atomic::AtomicCell;
use nih_plug::prelude::*;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::*;
use nih_plug_vizia::{assets, create_vizia_editor, ViziaState, ViziaTheming};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use self::analyzer::Analyzer;
use self::mode_button::EditorModeButton;
use crate::analyzer::AnalyzerData;
use crate::{SpectralCompressor, SpectralCompressorParams};

mod analyzer;
mod mode_button;

/// The entire GUI's width, in logical pixels.
const EXPANDED_GUI_WIDTH: u32 = 1360;
/// The width of the GUI's main part containing the controls.
const COLLAPSED_GUI_WIDTH: u32 = 680;
/// The entire GUI's height, in logical pixels.
const GUI_HEIGHT: u32 = 530;
// I couldn't get `LayoutType::Grid` to work as expected, so we'll fake a 4x4 grid with
// hardcoded column widths
const COLUMN_WIDTH: Units = Pixels(330.0);

const DARKER_GRAY: Color = Color::rgb(0x69, 0x69, 0x69);

/// The editor's mode. Essentially just a boolean to indicate whether the analyzer is shown or
/// not.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorMode {
    // These serialization names are hardcoded so the variants can be renamed them later without
    // breaking preset compatibility
    #[serde(rename = "collapsed")]
    Collapsed,
    #[default]
    #[serde(rename = "analyzer-visible")]
    AnalyzerVisible,
}

#[derive(Clone, Lens)]
pub struct Data {
    pub(crate) params: Arc<SpectralCompressorParams>,

    /// Determines which parts of the GUI are visible, and in turn decides the GUI's size.
    pub(crate) editor_mode: Arc<AtomicCell<EditorMode>>,

    pub(crate) analyzer_data: Arc<Mutex<triple_buffer::Output<AnalyzerData>>>,
    /// Used by the analyzer to determine which FFT bins belong to which frequencies.
    pub(crate) sample_rate: Arc<AtomicF32>,
}

impl Model for Data {}

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state(editor_mode: Arc<AtomicCell<EditorMode>>) -> Arc<ViziaState> {
    ViziaState::new(move || match editor_mode.load() {
        EditorMode::Collapsed => (COLLAPSED_GUI_WIDTH, GUI_HEIGHT),
        EditorMode::AnalyzerVisible => (EXPANDED_GUI_WIDTH, GUI_HEIGHT),
    })
}

pub(crate) fn create(editor_state: Arc<ViziaState>, editor_data: Data) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, ViziaTheming::Custom, move |cx, _| {
        assets::register_noto_sans_light(cx);
        assets::register_noto_sans_thin(cx);

        if let Err(err) = cx.add_stylesheet(include_style!("src/editor/theme.css")) {
            nih_error!("Failed to load stylesheet: {err:?}")
        }

        editor_data.clone().build(cx);

        HStack::new(cx, |cx| {
            main_column(cx);

            let analyzer_visible = Data::editor_mode
                .map(|editor_mode| editor_mode.load() == EditorMode::AnalyzerVisible);
            Binding::new(cx, analyzer_visible, |cx, analyzer_visible| {
                if analyzer_visible.get(cx) {
                    analyzer_column(cx);
                }
            });
        });

        ResizeHandle::new(cx);
    })
}

fn main_column(cx: &mut Context) {
    VStack::new(cx, |cx| {
        HStack::new(cx, |cx| {
            EditorModeButton::new(cx, Data::editor_mode, "Show analyzer")
                // Makes this align a bit nicer with the plugin name
                .top(Pixels(2.0))
                .left(Pixels(2.0));

            HStack::new(cx, |cx| {
                Label::new(cx, "Spectral Compressor")
                    .font_family(vec![FamilyOwned::Name(String::from(assets::NOTO_SANS))])
                    .font_weight(FontWeightKeyword::Thin)
                    .font_size(30.0)
                    .on_mouse_down(|_, _| {
                        // FIXME: On Windows this blocks, and while this is blocking a timer may
                        //        proc which causes the window state to be mutably borrowed again,
                        //        resulting in a panic. This needs to be fixed in baseview first.
                        if cfg!(not(windows)) {
                            // Try to open the plugin's page when clicking on the title. If this
                            // fails then that's not a problem
                            let result = open::that(SpectralCompressor::URL);
                            if cfg!(debug) && result.is_err() {
                                nih_debug_assert_failure!(
                                    "Failed to open web browser: {:?}",
                                    result
                                );
                            }
                        }
                    });
                Label::new(cx, SpectralCompressor::VERSION)
                    .color(DARKER_GRAY)
                    .top(Stretch(1.0))
                    .bottom(Pixels(4.0))
                    .left(Pixels(2.0));
            })
            .size(Auto);
        })
        .height(Pixels(30.0))
        .right(Pixels(17.0))
        // Somehow this overrides the 'row-between' value now
        .bottom(Pixels(8.0))
        .left(Pixels(10.0))
        .top(Pixels(10.0))
        // This contains the editor mode buttom all the way on the left, and the plugin's name all the way on the right
        .col_between(Stretch(1.0));

        HStack::new(cx, |cx| {
            make_column(cx, "Globals", |cx| {
                GenericUi::new(cx, Data::params.map(|p| p.global.clone()));
            });

            make_column(cx, "Threshold", |cx| {
                GenericUi::new(cx, Data::params.map(|p| p.threshold.clone()));

                Label::new(
                    cx,
                    "Parameter ranges and overal gain staging are still subject to change. If you \
                     use this in a project, make sure to bounce things to audio just in case \
                     they'll sound different later.",
                )
                .text_wrap(true)
                .font_size(11.0)
                .left(Pixels(15.0))
                .right(Pixels(8.0))
                .width(Stretch(1.0));
            });
        })
        .size(Auto);

        HStack::new(cx, |cx| {
            make_column(cx, "Upwards", |cx| {
                // We don't want to show the 'Upwards' prefix here, but it should still be in
                // the parameter name so the parameter list makes sense
                let upwards_compressor_params = Data::params.map(|p| p.compressors.upwards.clone());
                GenericUi::new_custom(cx, upwards_compressor_params, |cx, param_ptr| {
                    HStack::new(cx, |cx| {
                        Label::new(
                            cx,
                            unsafe { param_ptr.name() }
                                .strip_prefix("Upwards ")
                                .expect("Expected parameter name prefix, this is a bug"),
                        )
                        .class("label");

                        GenericUi::draw_widget(cx, upwards_compressor_params, param_ptr);
                    })
                    .class("row");
                });
            });

            make_column(cx, "Downwards", |cx| {
                let downwards_compressor_params =
                    Data::params.map(|p| p.compressors.downwards.clone());
                GenericUi::new_custom(cx, downwards_compressor_params, |cx, param_ptr| {
                    HStack::new(cx, |cx| {
                        Label::new(
                            cx,
                            unsafe { param_ptr.name() }
                                .strip_prefix("Downwards ")
                                .expect("Expected parameter name prefix, this is a bug"),
                        )
                        .class("label");

                        GenericUi::draw_widget(cx, downwards_compressor_params, param_ptr);
                    })
                    .class("row");
                });
            });
        })
        .size(Auto);
    })
    .width(Pixels(COLLAPSED_GUI_WIDTH as f32))
    .row_between(Pixels(10.0))
    .child_left(Stretch(1.0))
    .child_right(Stretch(1.0));
}

fn analyzer_column(cx: &mut Context) {
    Analyzer::new(cx, Data::analyzer_data, Data::sample_rate)
        // These arbitrary 12 pixels are to align with the analyzer toggle botton
        .space(Pixels(12.0))
        .bottom(Pixels(12.0))
        .left(Pixels(2.0))
        .top(Pixels(12.0));
}

fn make_column(cx: &mut Context, title: &str, contents: impl FnOnce(&mut Context)) {
    VStack::new(cx, |cx| {
        Label::new(cx, title)
            .font_family(vec![FamilyOwned::Name(String::from(assets::NOTO_SANS))])
            .font_weight(FontWeightKeyword::Thin)
            .font_size(23.0)
            .left(Stretch(1.0))
            // This should align nicely with the right edge of the slider
            .right(Pixels(7.0))
            .bottom(Pixels(-10.0));

        contents(cx);
    })
    .width(COLUMN_WIDTH)
    .height(Auto);
}
