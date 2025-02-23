//! Resizable window wrapper for Egui editor.

use egui_baseview::egui::emath::GuiRounding;
use egui_baseview::egui::{InnerResponse, UiBuilder};

use crate::egui::{pos2, CentralPanel, Context, Id, Rect, Response, Sense, Ui, Vec2};
use crate::EguiState;

/// Adds a corner to the plugin window that can be dragged in order to resize it.
/// Resizing happens through plugin API, hence a custom implementation is needed.
pub struct ResizableWindow {
    id: Id,
    min_size: Vec2,
}

impl ResizableWindow {
    pub fn new(id_source: impl std::hash::Hash) -> Self {
        Self {
            id: Id::new(id_source),
            min_size: Vec2::splat(16.0),
        }
    }

    /// Won't shrink to smaller than this
    #[inline]
    pub fn min_size(mut self, min_size: impl Into<Vec2>) -> Self {
        self.min_size = min_size.into();
        self
    }

    pub fn show<R>(
        self,
        context: &Context,
        egui_state: &EguiState,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        CentralPanel::default().show(context, move |ui| {
            let ui_rect = ui.clip_rect();
            let mut content_ui =
                ui.new_child(UiBuilder::new().max_rect(ui_rect).layout(*ui.layout()));

            let ret = add_contents(&mut content_ui);

            let corner_size = Vec2::splat(ui.visuals().resize_corner_size);
            let corner_rect = Rect::from_min_size(ui_rect.max - corner_size, corner_size);

            let corner_response = ui.interact(corner_rect, self.id.with("corner"), Sense::drag());

            if let Some(pointer_pos) = corner_response.interact_pointer_pos() {
                let desired_size = (pointer_pos - ui_rect.min + 0.5 * corner_response.rect.size())
                    .max(self.min_size);

                if corner_response.dragged() {
                    egui_state.set_requested_size((
                        desired_size.x.round() as u32,
                        desired_size.y.round() as u32,
                    ));
                }
            }

            paint_resize_corner(&content_ui, &corner_response);

            ret
        })
    }
}

pub fn paint_resize_corner(ui: &Ui, response: &Response) {
    let stroke = ui.style().interact(response).fg_stroke;

    let painter = ui.painter();
    let rect = response.rect.translate(-Vec2::splat(2.0)); // move away from the corner
    let cp = rect.max.round_to_pixels(painter.pixels_per_point());

    let mut w = 2.0;

    while w <= rect.width() && w <= rect.height() {
        painter.line_segment([pos2(cp.x - w, cp.y), pos2(cp.x, cp.y - w)], stroke);
        w += 4.0;
    }
}
