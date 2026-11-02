//! A resize handle for uniformly scaling a plugin GUI.

use vizia::prelude::*;
use vizia::vg;

/// A resize handle placed at the bottom right of the window that lets you resize the window.
///
/// Needs to be the last element in the GUI because of how event targetting in Vizia works right
/// now.
pub struct ResizeHandle {
    /// Will be set to `true` if we're dragging the parameter. Resetting the parameter or entering a
    /// text value should not initiate a drag.
    drag_active: bool,

    /// The scale factor when we started dragging. This is kept track of separately to avoid
    /// accumulating rounding errors.
    start_scale_factor: f64,
    /// The DPI factor when we started dragging, includes both the HiDPI scaling and the user
    /// scaling factor. This is kept track of separately to avoid accumulating rounding errors.
    start_dpi_factor: f32,
    /// The cursor position in physical screen pixels when the drag started.
    start_physical_coordinates: (f32, f32),
    /// The window size in logical pixels when the drag started.
    ///
    /// The drag ratio has to be measured against the window itself instead of against the absolute
    /// cursor position on the screen. Using the absolute position meant that the handle only behaved
    /// correctly when the window happened to sit in the top-left corner of the screen: starting a
    /// drag at the bottom-right of a window placed further away produced a wildly wrong ratio, and
    /// dragging leftwards/upwards made the ratio negative.
    start_logical_size: (f32, f32),
}

impl ResizeHandle {
    /// Create a resize handle at the bottom right of the window. This should be created at the top
    /// level. Dragging this handle around will cause the window to be resized.
    pub fn new(cx: &mut Context) -> Handle<Self> {
        // Styling is done in the style sheet
        ResizeHandle {
            drag_active: false,
            start_scale_factor: 1.0,
            start_dpi_factor: 1.0,
            start_physical_coordinates: (0.0, 0.0),
            start_logical_size: (1.0, 1.0),
        }
        .build(cx, |_| {})
    }
}

impl View for ResizeHandle {
    fn element(&self) -> Option<&'static str> {
        Some("resize-handle")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|window_event, meta| match *window_event {
            WindowEvent::MouseDown(MouseButton::Left) => {
                // The handle is a triangle, so we should also interact with it as if it was a
                // triangle
                if intersects_triangle(
                    cx.cache.get_bounds(cx.current()),
                    (cx.mouse().cursorx, cx.mouse().cursory),
                ) {
                    cx.capture();
                    cx.set_active(true);

                    self.drag_active = true;
                    self.start_scale_factor = cx.user_scale_factor();
                    self.start_dpi_factor = cx.scale_factor();
                    self.start_physical_coordinates = (
                        cx.mouse().cursorx * self.start_dpi_factor,
                        cx.mouse().cursory * self.start_dpi_factor,
                    );
                    // Remember how big the window was when the drag started. The drag ratio is
                    // measured against this instead of against the absolute cursor position.
                    let window_size = cx.window_size();
                    self.start_logical_size = (window_size.width as f32, window_size.height as f32);

                    meta.consume();
                } else {
                    // TODO: The click should be forwarded to the element behind the triangle
                }
            }
            WindowEvent::MouseUp(MouseButton::Left) => {
                if self.drag_active {
                    cx.release();
                    cx.set_active(false);

                    self.drag_active = false;
                }
            }
            WindowEvent::MouseMove(x, y) => {
                cx.set_hover(intersects_triangle(
                    cx.cache.get_bounds(cx.current()),
                    (x, y),
                ));

                if self.drag_active {
                    // Convert the cursor position into physical pixels. The drag ratio is then
                    // measured as "how much larger is the window compared to when the drag
                    // started", using the window's own size as the reference instead of the
                    // absolute on-screen cursor position. Using the absolute position made the
                    // handle behave differently depending on where the window sat on the screen,
                    // and produced negative ratios when dragging up/leftwards.
                    let (compensated_physical_x, compensated_physical_y) =
                        (x * self.start_dpi_factor, y * self.start_dpi_factor);
                    let (start_physical_x, start_physical_y) = self.start_physical_coordinates;
                    let (start_logical_width, start_logical_height) = self.start_logical_size;

                    // Size of the window implied by the current cursor position, in physical
                    // pixels: the handle sits in the bottom-right corner, so the cursor position
                    // relative to the window's top-left corner is the window's physical size.
                    //
                    // The top-left corner of the window in physical pixels is the cursor position
                    // at drag start minus the window's physical size at drag start.
                    let start_physical_width = start_logical_width * self.start_dpi_factor;
                    let start_physical_height = start_logical_height * self.start_dpi_factor;
                    let window_physical_origin = (
                        start_physical_x - start_physical_width,
                        start_physical_y - start_physical_height,
                    );

                    let physical_width = (compensated_physical_x - window_physical_origin.0)
                        .max(start_physical_width);
                    let physical_height = (compensated_physical_y - window_physical_origin.1)
                        .max(start_physical_height);

                    let new_scale_factor = (self.start_scale_factor
                        * (physical_width / start_physical_width)
                            .max(physical_height / start_physical_height)
                            as f64)
                        // Vizia rounds borders to integer pixels, and at <0.5 scaling one pixel
                        // borders will simply disappear
                        .max(0.5);

                    // If this is different then the window will automatically be resized at the end
                    // of the frame
                    cx.set_user_scale_factor(new_scale_factor);
                }
            }
            _ => {}
        });
    }

    fn draw(&self, cx: &mut DrawContext, canvas: &mut Canvas) {
        // We'll draw the handle directly as styling elements for this is going to be a bit tricky

        // These basics are taken directly from the default implementation of this function
        let bounds = cx.bounds();
        if bounds.w == 0.0 || bounds.h == 0.0 {
            return;
        }

        let background_color = cx.background_color();
        let border_color = cx.border_color();
        let opacity = cx.opacity();
        let mut background_color: vg::Color = background_color.into();
        background_color.set_alphaf(background_color.a * opacity);
        let mut border_color: vg::Color = border_color.into();
        border_color.set_alphaf(border_color.a * opacity);
        let border_width = cx.border_width();

        let mut path = vg::Path::new();
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
        let paint = vg::Paint::color(background_color);
        canvas.fill_path(&path, &paint);

        // Borders are only supported to make debugging easier
        let mut paint = vg::Paint::color(border_color);
        paint.set_line_width(border_width);
        canvas.stroke_path(&path, &paint);

        // We'll draw a simple triangle, since we're going flat everywhere anyways and that style
        // tends to not look too tacky
        let mut path = vg::Path::new();
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

        let mut color: vg::Color = cx.font_color().into();
        color.set_alphaf(color.a * opacity);
        let paint = vg::Paint::color(color);
        canvas.fill_path(&path, &paint);
    }
}

/// Test whether a point intersects with the triangle of this resize handle.
fn intersects_triangle(bounds: BoundingBox, (x, y): (f32, f32)) -> bool {
    // We could also compute Barycentric coordinates, but this is simple and I like not having to
    // think. Just check if (going clockwise), the point is on the right of each of all of the
    // triangle's edges. We can compute this using the determinant of the 2x2 matrix formed by two
    // column vectors, aka the perp dot product, aka the wedge product.
    // NOTE: Since this element is positioned in the bottom right corner we would technically only
    //       have to calculate this for `v1`
    let (p1x, p1y) = bounds.bottom_left();
    let (p2x, p2y) = bounds.top_right();
    // let (p3x, p3y) = bounds.bottom_right();

    let (v1x, v1y) = (p2x - p1x, p2y - p1y);
    // let (v2x, v2y) = (p3x - p2x, p3y - p2y);
    // let (v3x, v3y) = (p1x - p3x, p1y - p3y);

    ((x - p1x) * v1y) - ((y - p1y) * v1x) >= 0.0
    // && ((x - p2x) * v2y) - ((y - p2y) * v2x) >= 0.0
    // && ((x - p3x) * v3y) - ((y - p3y) * v3x) >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_intersection() {
        let bbox = BoundingBox {
            x: 10.0,
            y: 10.0,
            w: 10.0,
            h: 10.0,
        };

        assert!(!intersects_triangle(bbox, (10.0, 10.0)));
        assert!(intersects_triangle(bbox, (20.0, 10.0)));
        assert!(intersects_triangle(bbox, (10.0, 20.0)));
        assert!(intersects_triangle(bbox, (20.0, 20.0)));

        assert!(intersects_triangle(bbox, (15.0, 15.0)));
        assert!(!intersects_triangle(bbox, (14.9, 15.0)));
        assert!(!intersects_triangle(bbox, (15.0, 14.9)));
    }
}
