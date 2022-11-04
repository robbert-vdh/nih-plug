use nih_plug_vizia::vizia::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A custom toggleable button coupled to an `Arc<AtomicBool`. Otherwise this is very similar to the
/// param button.
#[derive(Lens)]
pub struct SafeModeButton<L: Lens<Target = Arc<AtomicBool>>> {
    lens: L,
}

impl<L: Lens<Target = Arc<AtomicBool>>> SafeModeButton<L> {
    /// Creates a new button bound to the `Arc<AtomicBool>`.
    pub fn new<T>(cx: &mut Context, lens: L, label: impl Res<T>) -> Handle<Self>
    where
        T: ToString,
    {
        Self { lens: lens.clone() }
            .build(cx, move |cx| {
                Label::new(cx, label);
            })
            .checked(lens.map(|v| v.load(Ordering::Relaxed)))
            // We'll pretend this is a param-button, so this class is used for assigning a unique
            // color
            .class("safe-mode")
    }
}

impl<L: Lens<Target = Arc<AtomicBool>>> View for SafeModeButton<L> {
    fn element(&self) -> Option<&'static str> {
        // Reuse the styling from param-button
        Some("param-button")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|window_event, meta| match window_event {
            WindowEvent::MouseDown(MouseButton::Left)
            // We don't need special double and triple click handling
            | WindowEvent::MouseDoubleClick(MouseButton::Left)
            | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                // We can just unconditionally toggle the boolean here
                let atomic = self.lens.get(cx);
                atomic.fetch_xor(true, Ordering::AcqRel);

                meta.consume();
            }
            _ => {}
        });
    }
}
