use egui::{vec2, Color32, Response, Sense, Stroke, TextStyle, Ui, Widget};

use nih_plug::{FloatParam, Param, ParamSetter};

/// A slider widget similar to [egui::widgets::Slider] that knows about NIH-plug parameters ranges
/// and can get values for it.
///
/// TODO: To keep things simple this currently only supports FloatParam, but it should be generic
///       over any kind of parameter
/// TODO: Vertical orientation
/// TODO: (before I forget) mouse scrolling, ctrl+click and double click to reset
pub struct ParamSlider<'a> {
    param: &'a FloatParam,
    setter: &'a ParamSetter<'a>,
}

impl<'a> ParamSlider<'a> {
    /// Create a new slider for a parameter. Use the other methods to modify the slider before
    /// passing it to [Ui::add()].
    pub fn for_param(param: &'a FloatParam, setter: &'a ParamSetter<'a>) -> Self {
        Self { param, setter }
    }

    fn normalized_value(&self) -> f32 {
        self.param.normalized_value()
    }

    fn set_normalized_value(&self, normalized: f32) {
        // TODO: The gesture should start on mouse down and end up mouse up
        self.setter.begin_set_parameter(self.param);
        self.setter.set_parameter_normalized(self.param, normalized);
        self.setter.end_set_parameter(self.param);
    }
}

impl Widget for ParamSlider<'_> {
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
        // TODO: We only show the position now
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

        // And finally draw the thing
        if ui.is_rect_visible(response.rect) {
            // We'll do a flat widget with background -> filled foreground -> slight border
            ui.painter()
                .rect_filled(response.rect, 0.0, ui.visuals().widgets.inactive.bg_fill);

            let filled_proportion = self.normalized_value();
            let mut filled_rect = response.rect;
            filled_rect.set_width(response.rect.width() * filled_proportion);
            let filled_bg = if response.dragged() {
                // TODO: Use something that works with a light theme
                Color32::DARK_GRAY
            } else {
                ui.visuals().selection.bg_fill
            };
            ui.painter().rect_filled(filled_rect, 0.0, filled_bg);

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
