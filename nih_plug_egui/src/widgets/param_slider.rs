use egui::{vec2, Response, Sense, Stroke, TextStyle, Ui, Vec2, Widget};
use lazy_static::lazy_static;

use super::util;
use nih_plug::{Param, ParamSetter};

/// When shift+dragging a parameter, one pixel dragged corresponds to this much change in the
/// noramlized parameter.
const GRANULAR_DRAG_MULTIPLIER: f32 = 0.0015;

lazy_static! {
    static ref DRAG_NORMALIZED_START_VALUE_MEMORY_ID: egui::Id = egui::Id::new((file!(), 0));
    static ref DRAG_AMOUNT_MEMORY_ID: egui::Id = egui::Id::new((file!(), 1));
}

/// A slider widget similar to [egui::widgets::Slider] that knows about NIH-plug parameters ranges
/// and can get values for it.
///
/// TODO: Vertical orientation
/// TODO: Check below for more input methods that should be added
/// TODO: Decouple the logic from the drawing so we can also do things like nobs without having to
///       repeat everything
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
        if normalized != self.normalized_value() {
            // This snaps to the nearest plain value if the parameter is stepped in some wayA
            // TODO: As an optimization, we could add a `const CONTINUOUS: bool` to the parameter to
            //       avoid this normalized->plain->normalized conversion for parameters that don't need
            //       it
            let value = self.param.preview_plain(normalized);
            self.setter.set_parameter(self.param, value);
        }
    }

    // This still needs to be part of a drag gestur
    fn reset_param(&self) {
        let normalized_default = self.setter.default_normalized_param_value(self.param);
        self.setter
            .set_parameter_normalized(self.param, normalized_default);
    }

    fn granular_drag(&self, ui: &Ui, drag_delta: Vec2) {
        // Remember the intial position when we started with the granular drag. This value gets
        // reset whenever we have a normal itneraction with the slider.
        let start_value = if Self::get_drag_amount_memory(ui) == 0.0 {
            Self::set_drag_normalized_start_value_memory(ui, self.normalized_value());
            self.normalized_value()
        } else {
            Self::get_drag_normalized_start_value_memory(ui)
        };

        let total_drag_distance = drag_delta.x + Self::get_drag_amount_memory(ui);
        Self::set_drag_amount_memory(ui, total_drag_distance);

        self.set_normalized_value(
            (start_value + (total_drag_distance * GRANULAR_DRAG_MULTIPLIER)).clamp(0.0, 1.0),
        );
    }

    fn end_drag(&self) {
        self.setter.end_set_parameter(self.param);
    }

    fn get_drag_normalized_start_value_memory(ui: &Ui) -> f32 {
        ui.memory()
            .data
            .get_temp(*DRAG_NORMALIZED_START_VALUE_MEMORY_ID)
            .unwrap_or(0.0)
    }

    fn set_drag_normalized_start_value_memory(ui: &Ui, amount: f32) {
        ui.memory()
            .data
            .insert_temp(*DRAG_NORMALIZED_START_VALUE_MEMORY_ID, amount);
    }

    fn get_drag_amount_memory(ui: &Ui) -> f32 {
        ui.memory()
            .data
            .get_temp(*DRAG_AMOUNT_MEMORY_ID)
            .unwrap_or(0.0)
    }

    fn set_drag_amount_memory(ui: &Ui, amount: f32) {
        ui.memory().data.insert_temp(*DRAG_AMOUNT_MEMORY_ID, amount);
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
        // TODO: Optionally (since it can be annoying) add scrolling behind a builder option
        // TODO: Optionally add alt+click for value entry?
        // TODO: Handle shift+drag being more granular
        if response.drag_started() {
            // When beginning a drag or dragging normally, reset the memory used to keep track of
            // our granular drag
            self.begin_drag();
            Self::set_drag_amount_memory(ui, 0.0);
        }
        if let Some(click_pos) = response.interact_pointer_pos() {
            if ui.input().modifiers.command {
                // Like double clicking, Ctrl+Click should reset the parameter
                self.reset_param();
            } else if ui.input().modifiers.shift {
                // And shift dragging should switch to a more granulra input method
                self.granular_drag(ui, response.drag_delta());
            } else {
                let proportion =
                    egui::emath::remap_clamp(click_pos.x, response.rect.x_range(), 0.0..=1.0)
                        as f64;
                self.set_normalized_value(proportion as f32);
                Self::set_drag_amount_memory(ui, 0.0);
            }
        }
        if response.double_clicked() {
            self.reset_param();
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
