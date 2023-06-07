use nih_plug::prelude::*;
use std::sync::Arc;

#[derive(Default)]
struct SysEx {
    params: Arc<SysExParams>,
}

#[derive(Default, Params)]
struct SysExParams {}

// This is a struct or enum describing all MIDI SysEx messages this plugin will send or receive
#[derive(Debug, Clone, PartialEq)]
enum CoolSysExMessage {
    Foo(f32),
    Bar { x: u8, y: u8 },
}

// This trait is used to convert between `CoolSysExMessage` and the raw MIDI SysEx messages. That
// way the rest of the code doesn't need to bother with parsing or SysEx implementation details.
impl SysExMessage for CoolSysExMessage {
    // This is a byte array that is large enough to write all of the messages to
    type Buffer = [u8; 6];

    fn from_buffer(buffer: &[u8]) -> Option<Self> {
        // `buffer` contains the entire buffer, including headers and the 0xf7 End Of system
        // eXclusive byte
        match buffer {
            [0xf0, 0x69, 0x01, n, 0xf7] => Some(CoolSysExMessage::Foo(*n as f32 / 127.0)),
            [0xf0, 0x69, 0x02, x, y, 0xf7] => Some(CoolSysExMessage::Bar { x: *x, y: *y }),
            _ => None,
        }
    }

    fn to_buffer(self) -> (Self::Buffer, usize) {
        // `Self::Buffer` needs to have a fixed size, so the result needs to be padded, and we
        // return the message's actual length in bytes alongside it so the caller can trim the
        // excess padding
        match self {
            CoolSysExMessage::Foo(x) => ([0xf0, 0x69, 0x01, (x * 127.0).round() as u8, 0xf7, 0], 5),
            CoolSysExMessage::Bar { x, y } => ([0xf0, 0x69, 0x02, x, y, 0xf7], 6),
        }
    }
}

impl Plugin for SysEx {
    const NAME: &'static str = "SysEx Example";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // This plugin doesn't have any audio IO
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[];

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    // The plugin needs to be have a note port to be able to send SysEx
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = CoolSysExMessage;
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // This example converts one of the two messages into the other
        while let Some(event) = context.next_event() {
            if let NoteEvent::MidiSysEx { timing, message } = event {
                let new_message = match message {
                    CoolSysExMessage::Foo(x) => CoolSysExMessage::Bar {
                        x: (x * 127.0).round() as u8,
                        y: 69,
                    },
                    CoolSysExMessage::Bar { x, y: _ } => CoolSysExMessage::Foo(x as f32 / 127.0),
                };

                context.send_event(NoteEvent::MidiSysEx {
                    timing,
                    message: new_message,
                });
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for SysEx {
    const CLAP_ID: &'static str = "com.moist-plugins-gmbh.sysex";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("An example plugin to demonstrate sending and receiving SysEx");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];
}

impl Vst3Plugin for SysEx {
    const VST3_CLASS_ID: [u8; 16] = *b"SysExCoolPluginn";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Tools,
    ];
}

nih_export_clap!(SysEx);
nih_export_vst3!(SysEx);
