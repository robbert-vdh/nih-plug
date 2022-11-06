// Spectral Compressor: an FFT based compressor
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
use std::sync::Arc;

use crate::SpectralCompressorParams;

/// VIZIA uses points instead of pixels for text
const POINT_SCALE: f32 = 0.75;

#[derive(Lens)]
struct Data {
    params: Arc<SpectralCompressorParams>,
}

impl Model for Data {}

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::from_size(680, 535)
}

pub(crate) fn create(
    params: Arc<SpectralCompressorParams>,
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, ViziaTheming::Custom, move |cx, _| {
        assets::register_noto_sans_light(cx);
        assets::register_noto_sans_thin(cx);

        Data {
            params: params.clone(),
        }
        .build(cx);

        ResizeHandle::new(cx);

        // There's no real 'grid' layout, the 4x4 grid should have even column widths and row
        // heights
        const LEFT_COLUMN_WIDTH: Units = Pixels(330.0);
        const RIGHT_COLUMN_WIDTH: Units = Pixels(330.0);

        VStack::new(cx, |cx| {
            Label::new(cx, "Spectral Compressor")
                .font(assets::NOTO_SANS_THIN)
                .font_size(40.0 * POINT_SCALE)
                .height(Pixels(50.0))
                .child_top(Stretch(1.0))
                .child_bottom(Pixels(0.0))
                .right(Pixels(15.0))
                .bottom(Pixels(-5.0));

            HStack::new(cx, |cx| {
                VStack::new(cx, |cx| {
                    Label::new(cx, "Globals")
                        .font(assets::NOTO_SANS_THIN)
                        .font_size(30.0 * POINT_SCALE)
                        .left(Stretch(1.0))
                        .right(Pixels(10.0))
                        .bottom(Pixels(-10.0));

                    GenericUi::new(cx, Data::params.map(|p| p.global.clone()));
                })
                .width(LEFT_COLUMN_WIDTH)
                .height(Auto);

                VStack::new(cx, |cx| {
                    Label::new(cx, "Threshold")
                        .font(assets::NOTO_SANS_THIN)
                        .font_size(30.0 * POINT_SCALE)
                        .left(Stretch(1.0))
                        .right(Pixels(10.0))
                        .bottom(Pixels(-10.0));

                    GenericUi::new(cx, Data::params.map(|p| p.threshold.clone()));

                    Label::new(
                        cx,
                        "Parameter ranges and overal gain staging are still subject to change. If \
                         you use this in a project, make sure to bounce things to audio just in \
                         case they'll sound different later.",
                    )
                    .font_size(15.0 * POINT_SCALE)
                    .left(Pixels(15.0))
                    .right(Pixels(5.0))
                    .width(Stretch(1.0));
                })
                .width(RIGHT_COLUMN_WIDTH)
                .height(Auto);
            })
            .height(Auto)
            .width(Stretch(1.0));

            HStack::new(cx, |cx| {
                VStack::new(cx, |cx| {
                    Label::new(cx, "Upwards")
                        .font(assets::NOTO_SANS_THIN)
                        .font_size(30.0 * POINT_SCALE)
                        .left(Stretch(1.0))
                        .right(Pixels(10.0))
                        .bottom(Pixels(-10.0));

                    // We don't want to show the 'Upwards' prefix here, but it should still be in
                    // the parameter name so the parameter list makes sense
                    let upwards_compressor_params =
                        Data::params.map(|p| p.compressors.upwards.clone());
                    GenericUi::new_custom(
                        cx,
                        upwards_compressor_params.clone(),
                        move |cx, param_ptr| {
                            let upwards_compressor_params = upwards_compressor_params.clone();
                            HStack::new(cx, move |cx| {
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
                        },
                    );
                })
                .width(RIGHT_COLUMN_WIDTH)
                .height(Auto);

                VStack::new(cx, |cx| {
                    Label::new(cx, "Downwards")
                        .font(assets::NOTO_SANS_THIN)
                        .font_size(30.0 * POINT_SCALE)
                        .left(Stretch(1.0))
                        .right(Pixels(10.0))
                        .bottom(Pixels(-10.0));

                    let downwards_compressor_params =
                        Data::params.map(|p| p.compressors.downwards.clone());
                    GenericUi::new_custom(
                        cx,
                        downwards_compressor_params.clone(),
                        move |cx, param_ptr| {
                            let downwards_compressor_params = downwards_compressor_params.clone();
                            HStack::new(cx, move |cx| {
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
                        },
                    );
                })
                .width(LEFT_COLUMN_WIDTH)
                .height(Auto);
            })
            .height(Auto)
            .width(Stretch(1.0));
        })
        .row_between(Pixels(10.0))
        .child_left(Stretch(1.0))
        .child_right(Stretch(1.0));
    })
}

// /// A [`ParamSlider`] row very similar to what [`GenericUi`] would produce.
// fn param_row(cx: &mut Context) {
//     VStack::new(cx, content)
// }
