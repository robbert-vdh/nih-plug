use egui::{vec2, Response, Sense, Stroke, TextStyle, Ui, Widget};

use super::util;
use nih_plug::{Param, ParamSetter};

/// A slider widget similar to [egui::widgets::Slider] that knows about NIH-plug parameters ranges
/// and can get values for it.
///
/// TODO: Vertical orientation
/// TODO: (before I forget) mouse scrolling, ctrl+click and double click to reset
pub struct ParamSlider<'a, P: Param> {
    param: &'a P,
    setter: &'a ParamSetter<'a>,
}

impl<'a, P: Param> ParamSlider<'a, P> {
    /// Create a new slider for a parameter. Use the other methods to modify the slider before
    /// passing it to [Ui::add()].
    pub fn for_param(param: &'a P, setter: &'a ParamSetter<'a>) -> Self {
        Self { param, setter }
    }

    fn normalized_value(&self) -> f32 {
        self.param.normalized_value()
    }

    fn begin_drag(&self) {
        self.setter.begin_set_parameter(self.param);
    }

    fn set_normalized_value(&self, normalized: f32) {
        // This snaps to the nearest plain value if the parameter is stepped in some wayA
        // TODO: As an optimization, we could add a `const CONTINUOUS: bool` to the parameter to
        //       avoid this normalized->plain->normalized conversion for parameters that don't need
        //       it
        let value = self.param.preview_plain(normalized);
        self.setter.set_parameter(self.param, value);
    }

    fn end_drag(&self) {
        self.setter.end_set_parameter(self.param);
    }
}

impl<P: Param> Widget for ParamSlider<'_, P> {
    fn ui(self, ui: &mut Ui) -> Response {
        // Allocate space, but add some padding on the top and bottom to make it look a bit slimmer.
        let height = ui
            .fonts()
            .row_height(TextStyle::Body)
            .max(ui.spacing().interact_size.y);
        let slider_height = ui.painter().round_to_pixel(height * 0.65);
        let response = ui
            .vertical(|ui| {
                ui.allocate_space(vec2(
                    ui.spacing().slider_width,
                    (height - slider_height) / 2.0,
                ));
                let response = ui.allocate_response(
                    vec2(ui.spacing().slider_width, slider_height),
                    Sense::click_and_drag(),
                );
                ui.allocate_space(vec2(
                    ui.spacing().slider_width,
                    (height - slider_height) / 2.0,
                ));
                response
            })
            .inner;

        // Handle user input
        // TODO: As mentioned above, handle double click and ctrl+click, maybe also value entry
        if response.drag_started() {
            self.begin_drag();
        }
        if let Some(click_pos) = response.interact_pointer_pos() {
            let aim_radius = ui.input().aim_radius();
            let proportion = egui::emath::smart_aim::best_in_range_f64(
                egui::emath::remap_clamp(
                    click_pos.x - aim_radius,
                    response.rect.x_range(),
                    0.0..=1.0,
                ) as f64,
                egui::emath::remap_clamp(
                    click_pos.x + aim_radius,
                    response.rect.x_range(),
                    0.0..=1.0,
                ) as f64,
            );

            self.set_normalized_value(proportion as f32);
        }
        if response.drag_released() {
            self.end_drag();
        }

        // And finally draw the thing
        if ui.is_rect_visible(response.rect) {
            // We'll do a flat widget with background -> filled foreground -> slight border
            ui.painter()
                .rect_filled(response.rect, 0.0, ui.visuals().widgets.inactive.bg_fill);

            let filled_proportion = self.normalized_value();
            if filled_proportion > 0.0 {
                let mut filled_rect = response.rect;
                filled_rect.set_width(response.rect.width() * filled_proportion);
                let filled_bg = if response.dragged() {
                    util::add_hsv(ui.visuals().selection.bg_fill, 0.0, -0.1, 0.1)
                } else {
                    ui.visuals().selection.bg_fill
                };
                ui.painter().rect_filled(filled_rect, 0.0, filled_bg);
            }

            ui.painter().rect_stroke(
                response.rect,
                0.0,
                Stroke::new(1.0, ui.visuals().widgets.active.bg_fill),
            );

            // TODO: Render the text
        }

        response
    }
}
