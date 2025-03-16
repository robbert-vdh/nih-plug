use nih_plug::prelude::*;
use std::sync::Arc;

/// A plugin that inverts all MIDI note numbers, channels, CCs, velocities, pressures, and
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

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // This plugin doesn't have any audio IO
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
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
        // We'll invert the channel, note index, velocity, pressure, CC value, pitch bend, and
        // anything else that is invertable for all events we receive
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn {
                    timing,
                    voice_id,
                    channel,
                    note,
                    velocity,
                } => context.send_event(NoteEvent::NoteOn {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                    velocity: 1.0 - velocity,
                }),
                NoteEvent::NoteOff {
                    timing,
                    voice_id,
                    channel,
                    note,
                    velocity,
                } => context.send_event(NoteEvent::NoteOff {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                    velocity: 1.0 - velocity,
                }),
                NoteEvent::Choke {
                    timing,
                    voice_id,
                    channel,
                    note,
                } => context.send_event(NoteEvent::Choke {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                }),
                NoteEvent::PolyPressure {
                    timing,
                    voice_id,
                    channel,
                    note,
                    pressure,
                } => context.send_event(NoteEvent::PolyPressure {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                    pressure: 1.0 - pressure,
                }),
                NoteEvent::PolyVolume {
                    timing,
                    voice_id,
                    channel,
                    note,
                    gain,
                } => context.send_event(NoteEvent::PolyVolume {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                    gain: 1.0 - gain,
                }),
                NoteEvent::PolyPan {
                    timing,
                    voice_id,
                    channel,
                    note,
                    pan,
                } => context.send_event(NoteEvent::PolyPan {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                    pan: 1.0 - pan,
                }),
                NoteEvent::PolyTuning {
                    timing,
                    voice_id,
                    channel,
                    note,
                    tuning,
                } => context.send_event(NoteEvent::PolyTuning {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                    tuning: 1.0 - tuning,
                }),
                NoteEvent::PolyVibrato {
                    timing,
                    voice_id,
                    channel,
                    note,
                    vibrato,
                } => context.send_event(NoteEvent::PolyVibrato {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                    vibrato: 1.0 - vibrato,
                }),
                NoteEvent::PolyExpression {
                    timing,
                    voice_id,
                    channel,
                    note,
                    expression,
                } => context.send_event(NoteEvent::PolyExpression {
                    timing,
                    voice_id,
                    channel: 15 - channel,
                    note: 127 - note,
                    expression: 1.0 - expression,
                }),
                NoteEvent::PolyBrightness {
                    timing,
                    voice_id,
                    channel,
                    note,
                    brightness,
                } => context.send_event(NoteEvent::PolyBrightness {
                    timing,
                    voice_id,
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
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Inverts all note and MIDI signals in ways you don't want to");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];
}

impl Vst3Plugin for MidiInverter {
    const VST3_CLASS_ID: [u8; 16] = *b"M1d1Inv3r70rzAaA";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Tools];
}

nih_export_clap!(MidiInverter);
nih_export_vst3!(MidiInverter);
