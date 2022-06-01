use nih_plug::prelude::*;
use std::sync::Arc;

/// A plugin that inverts all MIDI note numbers, channels, CCs, velocitires, pressures, and
/// everything else you don't want to be inverted.
struct MidiInverter {
    params: Arc<MidiInverterParams>,
}

#[derive(Default, Params)]
struct MidiInverterParams {}

impl Default for MidiInverter {
    fn default() -> Self {
        Self {
            params: Arc::new(MidiInverterParams::default()),
        }
    }
}

impl Plugin for MidiInverter {
    const NAME: &'static str = "MIDI Inverter";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 0;
    const DEFAULT_NUM_OUTPUTS: u32 = 0;

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // We'll invert the channel, note index, velocity, pressure, CC value, pitch bend, and
        // anything else that is invertable for all events we receive
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn {
                    timing,
                    channel,
                    note,
                    velocity,
                } => context.send_event(NoteEvent::NoteOn {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    velocity: 1.0 - velocity,
                }),
                NoteEvent::NoteOff {
                    timing,
                    channel,
                    note,
                    velocity,
                } => context.send_event(NoteEvent::NoteOff {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    velocity: 1.0 - velocity,
                }),
                NoteEvent::PolyPressure {
                    timing,
                    channel,
                    note,
                    pressure,
                } => context.send_event(NoteEvent::PolyPressure {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    pressure: 1.0 - pressure,
                }),
                NoteEvent::PolyVolume {
                    timing,
                    channel,
                    note,
                    gain,
                } => context.send_event(NoteEvent::PolyVolume {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    gain: 1.0 - gain,
                }),
                NoteEvent::PolyPan {
                    timing,
                    channel,
                    note,
                    pan,
                } => context.send_event(NoteEvent::PolyPan {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    pan: 1.0 - pan,
                }),
                NoteEvent::PolyTuning {
                    timing,
                    channel,
                    note,
                    tuning,
                } => context.send_event(NoteEvent::PolyTuning {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    tuning: 1.0 - tuning,
                }),
                NoteEvent::PolyVibrato {
                    timing,
                    channel,
                    note,
                    vibrato,
                } => context.send_event(NoteEvent::PolyVibrato {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    vibrato: 1.0 - vibrato,
                }),
                NoteEvent::PolyExpression {
                    timing,
                    channel,
                    note,
                    expression,
                } => context.send_event(NoteEvent::PolyExpression {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    expression: 1.0 - expression,
                }),
                NoteEvent::PolyBrightness {
                    timing,
                    channel,
                    note,
                    brightness,
                } => context.send_event(NoteEvent::PolyBrightness {
                    timing,
                    channel: 15 - channel,
                    note: 127 - note,
                    brightness: 1.0 - brightness,
                }),
                NoteEvent::MidiChannelPressure {
                    timing,
                    channel,
                    pressure,
                } => context.send_event(NoteEvent::MidiChannelPressure {
                    timing,
                    channel: 15 - channel,
                    pressure: 1.0 - pressure,
                }),
                NoteEvent::MidiPitchBend {
                    timing,
                    channel,
                    value,
                } => context.send_event(NoteEvent::MidiPitchBend {
                    timing,
                    channel: 15 - channel,
                    value: 1.0 - value,
                }),
                NoteEvent::MidiCC {
                    timing,
                    channel,
                    cc,
                    value,
                } => context.send_event(NoteEvent::MidiCC {
                    timing,
                    channel: 15 - channel,
                    // The one thing we won't invert, because uuhhhh
                    cc,
                    value: 1.0 - value,
                }),
                _ => (),
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for MidiInverter {
    const CLAP_ID: &'static str = "com.moist-plugins-gmbh.midi-inverter";
    const CLAP_DESCRIPTION: &'static str =
        "Inverts all note and MIDI signals in ways you don't want to";
    const CLAP_FEATURES: &'static [&'static str] = &["note-effect", "utility"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for MidiInverter {
    const VST3_CLASS_ID: [u8; 16] = *b"M1d1Inv3r70rzAaA";
    const VST3_CATEGORIES: &'static str = "Instrument|Tools";
}

nih_export_clap!(MidiInverter);
nih_export_vst3!(MidiInverter);
