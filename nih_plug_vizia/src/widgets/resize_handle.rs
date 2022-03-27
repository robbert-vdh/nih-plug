//! A resize handle for uniformly scaling a plugin GUI.

use femtovg::{Paint, Path};
use vizia::*;

use super::WindowModel;

/// A resize handle placed at the bottom right of the window that lets you resize the window.
pub struct ResizeHandle {
    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,

    /// The scale factor when we started dragging. This is kept track of separately to avoid
    /// accumulating rounding errors.
    start_scale_factor: f64,
    /// The cursor position in physical screen pixels when the drag started.
    start_physical_coordinates: (f32, f32),
}

impl ResizeHandle {
    /// Create a resize handle at the bottom right of the window. This should be created at the top
    /// level. Dragging this handle around will cause the window to be resized.
    pub fn new(cx: &mut Context) -> Handle<Self> {
        // Styling is done in the style sheet
        ResizeHandle {
            drag_active: false,
            start_scale_factor: 1.0,
            start_physical_coordinates: (0.0, 0.0),
        }
        .build(cx)
    }
}

impl View for ResizeHandle {
    fn element(&self) -> Option<String> {
        Some(String::from("resize-handle"))
    }

    fn event(&mut self, cx: &mut Context, event: &mut Event) {
        if let Some(window_event) = event.message.downcast() {
            match *window_event {
                WindowEvent::MouseDown(MouseButton::Left) => {
                    cx.capture();
                    cx.current.set_active(cx, true);

                    let vizia_state = WindowModel::vizia_state.get(cx);
                    self.drag_active = true;
                    self.start_scale_factor = vizia_state.user_scale_factor();
                    self.start_physical_coordinates = (
                        cx.mouse.cursorx * cx.style.dpi_factor as f32,
                        cx.mouse.cursory * cx.style.dpi_factor as f32,
                    );
                }
                WindowEvent::MouseUp(MouseButton::Left) => {
                    if self.drag_active {
                        cx.release();
                        cx.current.set_active(cx, false);

                        self.drag_active = false;
                    }
                }
                WindowEvent::MouseMove(x, y) => {
                    // TODO: Filter the hover color and dragging to the actual triangle
                    if self.drag_active {
                        let vizia_state = WindowModel::vizia_state.get(cx);

                        // We need to convert our measurements into physical pixels relative to the
                        // initial drag to be able to keep a consistent ratio. This 'relative to the
                        // start' bit is important because otherwise we would be comparing the
                        // position to the same absoltue screen spotion.
                        // TODO: This may start doing fun things when the window grows so large that
                        //       it gets pushed upwards or leftwards
                        let (compensated_physical_x, compensated_physical_y) = (
                            x * self.start_scale_factor as f32,
                            y * self.start_scale_factor as f32,
                        );
                        let (start_physical_x, start_physical_y) = self.start_physical_coordinates;
                        let new_scale_factor = (self.start_scale_factor
                            * (compensated_physical_x / start_physical_x)
                                .max(compensated_physical_y / start_physical_y)
                                as f64)
                            // Prevent approaching zero here because uh
                            .max(0.25);
                        if new_scale_factor != vizia_state.user_scale_factor() {
                            cx.emit(WindowEvent::SetScale(new_scale_factor));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn draw(&self, cx: &mut Context, canvas: &mut Canvas) {
        // We'll draw the handle directly as styling elements for this is going to be a bit tricky

        // These basics are taken directly from the default implementation of this function
        let entity = cx.current;
        let bounds = cx.cache.get_bounds(entity);
        if bounds.w == 0.0 || bounds.h == 0.0 {
            return;
        }

        let background_color = cx
            .style
            .background_color
            .get(entity)
            .cloned()
            .unwrap_or_default();
        let border_color = cx
            .style
            .border_color
            .get(entity)
            .cloned()
            .unwrap_or_default();
        let opacity = cx.cache.get_opacity(entity);
        let mut background_color: femtovg::Color = background_color.into();
        background_color.set_alphaf(background_color.a * opacity);
        let mut border_color: femtovg::Color = border_color.into();
        border_color.set_alphaf(border_color.a * opacity);

        let border_width = match cx
            .style
            .border_width
            .get(entity)
            .cloned()
            .unwrap_or_default()
        {
            Units::Pixels(val) => val,
            Units::Percentage(val) => bounds.w.min(bounds.h) * (val / 100.0),
            _ => 0.0,
        };

        let mut path = Path::new();
        let x = bounds.x + border_width / 2.0;
        let y = bounds.y + border_width / 2.0;
        let w = bounds.w - border_width;
        let h = bounds.h - border_width;
        path.move_to(x, y);
        path.line_to(x, y + h);
        path.line_to(x + w, y + h);
        path.line_to(x + w, y);
        path.line_to(x, y);
        path.close();

        // Fill with background color
        let paint = Paint::color(background_color);
        canvas.fill_path(&mut path, paint);

        // Borders are only supported to make debugging easier
        let mut paint = Paint::color(border_color);
        paint.set_line_width(border_width);
        canvas.stroke_path(&mut path, paint);

        // We'll draw a simple triangle, since we're going flat everywhere anyways and that style
        // tends to not look too tacky
        let mut path = Path::new();
        let x = bounds.x + border_width / 2.0;
        let y = bounds.y + border_width / 2.0;
        let w = bounds.w - border_width;
        let h = bounds.h - border_width;
        path.move_to(x, y + h);
        path.line_to(x + w, y + h);
        path.line_to(x + w, y);
        path.move_to(x, y + h);
        path.close();

        // Yeah this looks nowhere as good
        // path.move_to(x, y + h);
        // path.line_to(x + (w / 3.0), y + h);
        // path.line_to(x + w, y + h / 3.0);
        // path.line_to(x + w, y);
        // path.move_to(x, y + h);
        // path.close();

        // path.move_to(x + (w / 3.0 * 1.5), y + h);
        // path.line_to(x + (w / 3.0 * 2.5), y + h);
        // path.line_to(x + w, y + (h / 3.0 * 2.5));
        // path.line_to(x + w, y + (h / 3.0 * 1.5));
        // path.move_to(x + (w / 3.0 * 1.5), y + h);
        // path.close();

        let mut color: femtovg::Color = cx
            .style
            .font_color
            .get(entity)
            .cloned()
            .unwrap_or(crate::Color::white())
            .into();
        color.set_alphaf(color.a * opacity);
        let paint = Paint::color(color);
        canvas.fill_path(&mut path, paint);
    }
}
