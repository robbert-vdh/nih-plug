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

use nih_plug::prelude::Param;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::param_base::ParamWidgetBase;

// TODO: Vizia doesn't let you do this -50% translation programmatically yet, so this is hardcoded
//       for now
const HANDLE_WIDTH_PX: f32 = 20.0;

/// An X-Y pad that controlers two parameters at the same time by binding them to one of the two
/// axes. This specific implementation has a tooltip for the X-axis parmaeter and allows
/// Alt+clicking to enter a specific value.
pub struct XyPad {
    x_param_base: ParamWidgetBase,
    y_param_base: ParamWidgetBase,
}

/// The [`XyPad`]'s handle. This is a separate eleemnt to allow easier positioning.
struct XyPadHandle;

impl XyPad {
    /// Creates a new [`XyPad`] for the given parameter. See
    /// [`ParamSlider`][nih_plug_vizia::widgets::ParamSlider] for more information on this
    /// function's arguments.
    pub fn new<L, Params, P1, P2, FMap1, FMap2>(
        cx: &mut Context,
        params: L,
        params_to_x_param: FMap1,
        params_to_y_param: FMap2,
    ) -> Handle<Self>
    where
        L: Lens<Target = Params> + Clone,
        Params: 'static,
        P1: Param + 'static,
        P2: Param + 'static,
        FMap1: Fn(&Params) -> &P1 + Copy + 'static,
        FMap2: Fn(&Params) -> &P2 + Copy + 'static,
    {
        Self {
            x_param_base: ParamWidgetBase::new(cx, params.clone(), params_to_x_param),
            y_param_base: ParamWidgetBase::new(cx, params.clone(), params_to_y_param),
        }
        .build(
            cx,
            // We need to create lenses for both the x-parameter's values and the y-parameter's
            // values
            ParamWidgetBase::build_view(
                params.clone(),
                params_to_x_param,
                move |cx, x_param_data| {
                    ParamWidgetBase::view(
                        cx,
                        params,
                        params_to_y_param,
                        move |cx, y_param_data| {
                            let x_position_lens = x_param_data.make_lens(|param| {
                                Percentage(param.unmodulated_normalized_value() * 100.0)
                            });
                            let y_position_lens = y_param_data.make_lens(|param| {
                                // NOTE: The y-axis increments downards, and we want high values at
                                //       the top and low values at the bottom
                                Percentage((1.0 - param.unmodulated_normalized_value()) * 100.0)
                            });

                            XyPadHandle::new(cx)
                                .top(y_position_lens)
                                .left(x_position_lens)
                                // TODO: It would be much nicer if this could be set in the
                                //       stylesheet, but Vizia doesn't support that right now
                                .translate((-(HANDLE_WIDTH_PX / 2.0), -(HANDLE_WIDTH_PX / 2.0)))
                                .width(Pixels(HANDLE_WIDTH_PX))
                                .height(Pixels(HANDLE_WIDTH_PX));
                        },
                    );
                },
            ),
        )
    }
}

impl XyPadHandle {
    fn new(cx: &mut Context) -> Handle<Self> {
        // This doesn't have or need any special behavior, it's just a marker element used for
        // positioning he handle
        Self.build(cx, |_| ())
    }
}

impl View for XyPad {
    fn element(&self) -> Option<&'static str> {
        Some("xy-pad")
    }
}

impl View for XyPadHandle {
    fn element(&self) -> Option<&'static str> {
        Some("xy-pad__handle")
    }

    // TODO: Add a draw() implementation that draws at a -50% translation
}
