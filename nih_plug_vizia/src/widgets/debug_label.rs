use nih_plug::nih_log;
use vizia::prelude::*;

use crate::editor::DebugMessage;

#[cfg(feature = "debug")]
#[derive(Lens)]
pub struct DebugLabel {
    label_text: String,
    num: u32,
}

#[cfg(feature = "debug")]
impl DebugLabel {
    pub fn new(cx: &mut Context) -> Handle<Self> {
        Self {
            label_text: String::from("None"),
            num: 0,
        }
        .build(cx, |cx| {
            Label::new(cx, Self::label_text).width(Pixels(300_f32));
        })
    }
}

#[cfg(feature = "debug")]
impl View for DebugLabel {
    fn element(&self) -> Option<&'static str> {
        Some("debug-text-view")
    }

    fn event(&mut self, _cx: &mut EventContext, event: &mut Event) {
        event.map(|event, meta| match event {
            DebugMessage::RequestedSize(x, y) => {
                self.label_text =
                    format!("{} - Requested size- width: {}; height: {}", self.num, x, y);
                self.num += 1;
                meta.consume();
            }
            DebugMessage::ChangedScaleFactor(f) => {
                self.label_text = format!("{} - Requested to set scale factor: {}", self.num, f);
                self.num += 1;
                meta.consume();
            }
            DebugMessage::SpawnedApp => {
                self.label_text = format!("{} - Spawned editor", self.num);
                self.num += 1;
                meta.consume();
            }

            DebugMessage::Other(s) => {
                self.label_text = format!("{} - {}", self.num, *s);
                self.num += 1;
                meta.consume();
            }
        })
    }
}
