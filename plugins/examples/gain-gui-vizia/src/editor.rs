use atomic_float::AtomicF32;
use nih_plug::prelude::Editor;
use nih_plug_vizia::vizia::*;
use nih_plug_vizia::{assets, create_vizia_editor, ViziaState};
use std::pin::Pin;
use std::sync::Arc;

use crate::GainParams;

/// VIZIA uses points instead of pixels for text
const POINT_SCALE: f32 = 0.75;

const STYLE: &str = r#"
"#;

// Makes sense to also define this here, makes it a bit easier to keep track of
pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::from_size(200, 150)
}

pub(crate) fn create(
    params: Pin<Arc<GainParams>>,
    peak_meter: Arc<AtomicF32>,
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, |cx, setter| {
        // TOOD: `:root { background-color: #fafafa; }` in a stylesheet doesn't work
        cx.style
            .background_color
            .insert(Entity::root(), Color::rgb(250, 250, 250));
        // VIZIA uses points instead of pixels
        cx.style
            .font_size
            .insert(Entity::root(), 20.0 * POINT_SCALE);
        cx.add_theme(STYLE);

        // NOTE: vizia's font rendering looks way too dark and thick. Going one font weight lower
        //       seems to compensate for this.
        assets::register_fonts(cx);
        cx.set_default_font(assets::NOTO_SANS_LIGHT);

        VStack::new(cx, |cx| {
            Label::new(cx, "Gain GUI")
                .font(assets::NOTO_SANS_THIN)
                .font_size(40.0 * POINT_SCALE)
                .height(Pixels(50.0))
                .child_top(Stretch(1.0))
                .child_bottom(Pixels(0.0));
            Label::new(cx, "Gain");
        })
        .child_left(Stretch(1.0))
        .child_right(Stretch(1.0));
    })
}
