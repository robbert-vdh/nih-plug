//! Constants and definitions surrounding MIDI support.

use midi_consts::channel_event as midi;

pub mod sysex;

pub use midi_consts::channel_event::control_change;

/// Determines which note events a plugin receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MidiConfig {
    /// The plugin will not have a note input port and will thus not receive any not events.
    None,
    /// The plugin receives note on/off/choke events, pressure, and potentially a couple
    /// standardized expression types depending on the plugin standard and host. If the plugin sets
    /// up configuration for polyphonic modulation (see [`ClapPlugin`][crate::prelude::ClapPlugin])
    /// and assigns polyphonic modulation IDs to some of its parameters, then it will also receive
    /// polyphonic modulation events.
    Basic,
    /// The plugin receives full MIDI CCs as well as pitch bend information. For VST3 plugins this
    /// involves adding 130*16 parameters to bind to the the 128 MIDI CCs, pitch bend, and channel
    /// pressure.
    MidiCCs,
}

/// Event for (incoming) notes. The set of supported note events depends on the value of
/// [`Plugin::MIDI_INPUT`][crate::prelude::Plugin::MIDI_INPUT]. Also check out the
/// [`util`][crate::util] module for convenient conversion functions.
///
/// All of the timings are sample offsets withing the current buffer. All sample, channel and note
/// numbers are zero-indexed.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum NoteEvent {
    /// A note on event, available on [`MidiConfig::Basic`] and up.
    NoteOn {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's velocity, in `[0, 1]`. Some plugin APIs may allow higher precision than the
        /// 128 levels available in MIDI.
        velocity: f32,
    },
    /// A note off event, available on [`MidiConfig::Basic`] and up. Bitwig Studio does not provide
    /// a voice ID for this event.
    NoteOff {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's velocity, in `[0, 1]`. Some plugin APIs may allow higher precision than the
        /// 128 levels available in MIDI.
        velocity: f32,
    },
    /// A note choke event, available on [`MidiConfig::Basic`] and up. When the host sends this to
    /// the plugin, it indicates that a voice or all sound associated with a note should immediately
    /// stop playing.
    Choke {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
    },

    /// Sent by the plugin to the host to indicate that a voice has ended. This **needs** to be sent
    /// when a voice terminates when using polyphonic modulation. Otherwise you can ignore this
    /// event.
    VoiceTerminated {
        timing: u32,
        /// The voice's unique identifier. Setting this allows a single voice to be terminated if
        /// the plugin allows multiple overlapping voices for a single key.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
    },
    /// A polyphonic modulation event, available on [`MidiConfig::Basic`] and up. This will only be
    /// sent for parameters that were decorated with the `.with_poly_modulation_id()` modifier, and
    /// only by supported hosts. This event contains a _normalized offset value_ for the parameter's
    /// current, **unmodulated** value. That is, an offset for the current value before monophonic
    /// modulation is applied, as polyphonic modulation overrides monophonic modulation. There are
    /// multiple ways to incorporate this polyphonic modulation into a synthesizer, but a simple way
    /// to incorporate this would work as follows:
    ///
    /// - By default, a voice uses the parameter's global value, which may or may not include
    ///   monophonic modulation. This is `parameter.value` for unsmoothed parameters, and smoothed
    ///   parameters should use block smoothing so the smoothed values can be reused by multiple
    ///   voices.
    /// - If a `PolyModulation` event is emitted for the voice, that voice should use the the
    ///   _normalized offset_ contained within the event to compute the voice's modulated value and
    ///   use that in place of the global value.
    ///   - This value can be obtained by calling `param.preview_plain(param.normalized_value() +
    ///     event.normalized_offset)`. These functions automatically clamp the values as necessary.
    ///   - If the parameter uses smoothing, then the parameter's smoother can be copied to the
    ///     voice. [`Smoother::set_target()`][crate::prelude::Smoother::set_target()] can then be
    ///     used to have the smoother use the modulated value.
    ///   - One caveat with smoothing is that copying the smoother like this only works correctly if it last
    ///     produced a value during the sample before the `PolyModulation` event. Otherwise there
    ///     may still be an audible jump in parameter values. A solution for this would be to first
    ///     call the [`Smoother::reset()`][crate::prelude::Smoother::reset()] with the current
    ///     sample's global value before calling `set_target()`.
    ///   - Finally, if the polyphonic modulation happens on the same sample as the `NoteOn` event,
    ///     then the smoothing should not start at the current global value. In this case, `reset()`
    ///     should be called with the voice's modulated value.
    /// - If a `MonoAutomation` event is emitted for a parameter, then the values or target values
    ///   (if the parameter uses smoothing) for all voices must be updated. The normalized value
    ///   from the `MonoAutomation` and the voice's normalized modulation offset must be added and
    ///   converted back to a plain value. This value can be used directly for unsmoothed
    ///   parameters, or passed to `set_target()` for smoothed parameters. The global value will
    ///   have already been updated, so this event only serves as a notification to update
    ///   polyphonic modulation.
    /// - When a voice ends, either because the amplitude envelope has hit zero or because the voice
    ///   was stolen, the plugin must send a `VoiceTerminated` to the host to let it know that it
    ///   can reuse the resources it used to modulate the value.
    PolyModulation {
        timing: u32,
        /// The identifier of the voice this polyphonic modulation event should affect. This voice
        /// should use the values from this and subsequent polyphonic modulation events instead of
        /// the global value.
        voice_id: i32,
        /// The ID that was set for the modulated parameter using the `.with_poly_modulation_id()`
        /// method.
        poly_modulation_id: u32,
        /// The normalized offset value. See the event's docstring for more information.
        normalized_offset: f32,
    },
    /// A notification to inform the plugin that a polyphonically modulated parameter has received a
    /// new automation value. This is used in conjunction with the `PolyModulation` event. See that
    /// event's documentation for more details. The parameter's global value has already been
    /// updated when this event is emitted.
    MonoAutomation {
        timing: u32,
        /// The ID that was set for the modulated parameter using the `.with_poly_modulation_id()`
        /// method.
        poly_modulation_id: u32,
        /// The parameter's new normalized value. This needs to be added to a voice's normalized
        /// offset to get that voice's modulated normalized value. See the `PolyModulation` event's
        /// docstring for more information.
        normalized_value: f32,
    },

    /// A polyphonic note pressure/aftertouch event, available on [`MidiConfig::Basic`] and up. Not
    /// all hosts may support polyphonic aftertouch.
    ///
    /// # Note
    ///
    /// When implementing MPE support you should use MIDI channel pressure instead as polyphonic key
    /// pressure + MPE is undefined as per the MPE specification. Or as a more generic catch all,
    /// you may manually combine the polyphonic key pressure and MPE channel pressure.
    PolyPressure {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's pressure, in `[0, 1]`.
        pressure: f32,
    },
    /// A volume expression event, available on [`MidiConfig::Basic`] and up. Not all hosts may
    /// support these expressions.
    PolyVolume {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's voltage gain ratio, where 1.0 is unity gain.
        gain: f32,
    },
    /// A panning expression event, available on [`MidiConfig::Basic`] and up. Not all hosts may
    /// support these expressions.
    PolyPan {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's panning from, in `[-1, 1]`, with -1 being panned hard left, and 1
        /// being panned hard right.
        pan: f32,
    },
    /// A tuning expression event, available on [`MidiConfig::Basic`] and up. Not all hosts may support
    /// these expressions.
    PolyTuning {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's tuning in semitones, in `[-128, 128]`.
        tuning: f32,
    },
    /// A vibrato expression event, available on [`MidiConfig::Basic`] and up. Not all hosts may support
    /// these expressions.
    PolyVibrato {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's vibrato amount, in `[0, 1]`.
        vibrato: f32,
    },
    /// A expression expression (yes, expression expression) event, available on
    /// [`MidiConfig::Basic`] and up. Not all hosts may support these expressions.
    PolyExpression {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's expression amount, in `[0, 1]`.
        expression: f32,
    },
    /// A brightness expression event, available on [`MidiConfig::Basic`] and up. Not all hosts may support
    /// these expressions.
    PolyBrightness {
        timing: u32,
        /// A unique identifier for this note, if available. Using this to refer to a note is
        /// required when allowing overlapping voices for CLAP plugins.
        voice_id: Option<i32>,
        /// The note's channel, in `0..16`.
        channel: u8,
        /// The note's MIDI key number, in `0..128`.
        note: u8,
        /// The note's brightness amount, in `[0, 1]`.
        brightness: f32,
    },
    /// A MIDI channel pressure event, available on [`MidiConfig::MidiCCs`] and up.
    MidiChannelPressure {
        timing: u32,
        /// The affected channel, in `0..16`.
        channel: u8,
        /// The pressure, normalized to `[0, 1]` to match the poly pressure event.
        pressure: f32,
    },
    /// A MIDI pitch bend, available on [`MidiConfig::MidiCCs`] and up.
    MidiPitchBend {
        timing: u32,
        /// The affected channel, in `0..16`.
        channel: u8,
        /// The pressure, normalized to `[0, 1]`. `0.5` means no pitch bend.
        value: f32,
    },
    /// A MIDI control change event, available on [`MidiConfig::MidiCCs`] and up.
    ///
    /// # Note
    ///
    /// The wrapper does not perform any special handling for two message 14-bit CCs (where the CC
    /// number is in `0..32`, and the next CC is that number plus 32) or for four message RPN
    /// messages. For now you will need to handle these CCs yourself.
    MidiCC {
        timing: u32,
        /// The affected channel, in `0..16`.
        channel: u8,
        /// The control change number. See [`control_change`] for a list of CC numbers.
        cc: u8,
        /// The CC's value, normalized to `[0, 1]`. Multiply by 127 to get the original raw value.
        value: f32,
    },
    /// A MIDI program change event, available on [`MidiConfig::MidiCCs`] and up. VST3 plugins
    /// cannot receive these events.
    MidiProgramChange {
        timing: u32,
        /// The affected channel, in `0..16`.
        channel: u8,
        /// The program number, in `0..128`.
        program: u8,
    },
}

impl NoteEvent {
    /// Returns the sample within the current buffer this event belongs to.
    pub fn timing(&self) -> u32 {
        match self {
            NoteEvent::NoteOn { timing, .. } => *timing,
            NoteEvent::NoteOff { timing, .. } => *timing,
            NoteEvent::Choke { timing, .. } => *timing,
            NoteEvent::VoiceTerminated { timing, .. } => *timing,
            NoteEvent::PolyModulation { timing, .. } => *timing,
            NoteEvent::MonoAutomation { timing, .. } => *timing,
            NoteEvent::PolyPressure { timing, .. } => *timing,
            NoteEvent::PolyVolume { timing, .. } => *timing,
            NoteEvent::PolyPan { timing, .. } => *timing,
            NoteEvent::PolyTuning { timing, .. } => *timing,
            NoteEvent::PolyVibrato { timing, .. } => *timing,
            NoteEvent::PolyExpression { timing, .. } => *timing,
            NoteEvent::PolyBrightness { timing, .. } => *timing,
            NoteEvent::MidiChannelPressure { timing, .. } => *timing,
            NoteEvent::MidiPitchBend { timing, .. } => *timing,
            NoteEvent::MidiCC { timing, .. } => *timing,
            NoteEvent::MidiProgramChange { timing, .. } => *timing,
        }
    }

    /// Returns the event's voice ID, if it has any.
    pub fn voice_id(&self) -> Option<i32> {
        match self {
            NoteEvent::NoteOn { voice_id, .. } => *voice_id,
            NoteEvent::NoteOff { voice_id, .. } => *voice_id,
            NoteEvent::Choke { voice_id, .. } => *voice_id,
            NoteEvent::VoiceTerminated { voice_id, .. } => *voice_id,
            NoteEvent::PolyModulation { voice_id, .. } => Some(*voice_id),
            NoteEvent::MonoAutomation { .. } => None,
            NoteEvent::PolyPressure { voice_id, .. } => *voice_id,
            NoteEvent::PolyVolume { voice_id, .. } => *voice_id,
            NoteEvent::PolyPan { voice_id, .. } => *voice_id,
            NoteEvent::PolyTuning { voice_id, .. } => *voice_id,
            NoteEvent::PolyVibrato { voice_id, .. } => *voice_id,
            NoteEvent::PolyExpression { voice_id, .. } => *voice_id,
            NoteEvent::PolyBrightness { voice_id, .. } => *voice_id,
            NoteEvent::MidiChannelPressure { .. } => None,
            NoteEvent::MidiPitchBend { .. } => None,
            NoteEvent::MidiCC { .. } => None,
            NoteEvent::MidiProgramChange { .. } => None,
        }
    }

    /// Parse MIDI into a [`NoteEvent`]. Will return `Err(event_type)` if the parsing failed.
    pub fn from_midi(timing: u32, midi_data: [u8; 3]) -> Result<Self, u8> {
        // TODO: Maybe add special handling for 14-bit CCs and RPN messages at some
        //       point, right now the plugin has to figure it out for itself
        let event_type = midi_data[0] & midi::EVENT_TYPE_MASK;
        let channel = midi_data[0] & midi::MIDI_CHANNEL_MASK;
        match event_type {
            // You thought this was a note on? Think again! This is a cleverly disguised note off
            // event straight from the 80s when Baud rate was still a limiting factor!
            midi::NOTE_ON if midi_data[2] == 0 => Ok(NoteEvent::NoteOff {
                timing,
                voice_id: None,
                channel,
                note: midi_data[1],
                // Few things use release velocity. Just having this be zero here is fine, right?
                velocity: 0.0,
            }),
            midi::NOTE_ON => Ok(NoteEvent::NoteOn {
                timing,
                voice_id: None,
                channel,
                note: midi_data[1],
                velocity: midi_data[2] as f32 / 127.0,
            }),
            midi::NOTE_OFF => Ok(NoteEvent::NoteOff {
                timing,
                voice_id: None,
                channel,
                note: midi_data[1],
                velocity: midi_data[2] as f32 / 127.0,
            }),
            midi::POLYPHONIC_KEY_PRESSURE => Ok(NoteEvent::PolyPressure {
                timing,
                voice_id: None,
                channel,
                note: midi_data[1],
                pressure: midi_data[2] as f32 / 127.0,
            }),
            midi::CHANNEL_KEY_PRESSURE => Ok(NoteEvent::MidiChannelPressure {
                timing,
                channel,
                pressure: midi_data[1] as f32 / 127.0,
            }),
            midi::PITCH_BEND_CHANGE => Ok(NoteEvent::MidiPitchBend {
                timing,
                channel,
                value: (midi_data[1] as u16 + ((midi_data[2] as u16) << 7)) as f32
                    / ((1 << 14) - 1) as f32,
            }),
            midi::CONTROL_CHANGE => Ok(NoteEvent::MidiCC {
                timing,
                channel,
                cc: midi_data[1],
                value: midi_data[2] as f32 / 127.0,
            }),
            midi::PROGRAM_CHANGE => Ok(NoteEvent::MidiProgramChange {
                timing,
                channel,
                program: midi_data[1],
            }),
            n => Err(n),
        }
    }

    /// Create a MIDI message from this note event. Return `None` if this even does not have a
    /// direct MIDI equivalent. `PolyPressure` will be converted to polyphonic key pressure, but the
    /// other polyphonic note expression types will not be converted to MIDI CC messages.
    pub fn as_midi(self) -> Option<[u8; 3]> {
        match self {
            NoteEvent::NoteOn {
                timing: _,
                voice_id: _,
                channel,
                note,
                velocity,
            } => Some([
                midi::NOTE_ON | channel,
                note,
                (velocity * 127.0).round().clamp(0.0, 127.0) as u8,
            ]),
            NoteEvent::NoteOff {
                timing: _,
                voice_id: _,
                channel,
                note,
                velocity,
            } => Some([
                midi::NOTE_OFF | channel,
                note,
                (velocity * 127.0).round().clamp(0.0, 127.0) as u8,
            ]),
            NoteEvent::PolyPressure {
                timing: _,
                voice_id: _,
                channel,
                note,
                pressure,
            } => Some([
                midi::POLYPHONIC_KEY_PRESSURE | channel,
                note,
                (pressure * 127.0).round().clamp(0.0, 127.0) as u8,
            ]),
            NoteEvent::MidiChannelPressure {
                timing: _,
                channel,
                pressure,
            } => Some([
                midi::CHANNEL_KEY_PRESSURE | channel,
                (pressure * 127.0).round().clamp(0.0, 127.0) as u8,
                0,
            ]),
            NoteEvent::MidiPitchBend {
                timing: _,
                channel,
                value,
            } => {
                const PITCH_BEND_RANGE: f32 = ((1 << 14) - 1) as f32;
                let midi_value = (value * PITCH_BEND_RANGE)
                    .round()
                    .clamp(0.0, PITCH_BEND_RANGE) as u16;

                Some([
                    midi::PITCH_BEND_CHANGE | channel,
                    (midi_value & ((1 << 7) - 1)) as u8,
                    (midi_value >> 7) as u8,
                ])
            }
            NoteEvent::MidiCC {
                timing: _,
                channel,
                cc,
                value,
            } => Some([
                midi::CONTROL_CHANGE | channel,
                cc,
                (value * 127.0).round().clamp(0.0, 127.0) as u8,
            ]),
            NoteEvent::MidiProgramChange {
                timing: _,
                channel,
                program,
            } => Some([midi::PROGRAM_CHANGE | channel, program, 0]),
            NoteEvent::Choke { .. }
            | NoteEvent::VoiceTerminated { .. }
            | NoteEvent::PolyModulation { .. }
            | NoteEvent::MonoAutomation { .. }
            | NoteEvent::PolyVolume { .. }
            | NoteEvent::PolyPan { .. }
            | NoteEvent::PolyTuning { .. }
            | NoteEvent::PolyVibrato { .. }
            | NoteEvent::PolyExpression { .. }
            | NoteEvent::PolyBrightness { .. } => None,
        }
    }

    /// Subtract a sample offset from this event's timing, needed to compensate for the block
    /// splitting in the VST3 wrapper implementation because all events have to be read upfront.
    pub(crate) fn subtract_timing(&mut self, samples: u32) {
        match self {
            NoteEvent::NoteOn { timing, .. } => *timing -= samples,
            NoteEvent::NoteOff { timing, .. } => *timing -= samples,
            NoteEvent::Choke { timing, .. } => *timing -= samples,
            NoteEvent::VoiceTerminated { timing, .. } => *timing -= samples,
            NoteEvent::PolyModulation { timing, .. } => *timing -= samples,
            NoteEvent::MonoAutomation { timing, .. } => *timing -= samples,
            NoteEvent::PolyPressure { timing, .. } => *timing -= samples,
            NoteEvent::PolyVolume { timing, .. } => *timing -= samples,
            NoteEvent::PolyPan { timing, .. } => *timing -= samples,
            NoteEvent::PolyTuning { timing, .. } => *timing -= samples,
            NoteEvent::PolyVibrato { timing, .. } => *timing -= samples,
            NoteEvent::PolyExpression { timing, .. } => *timing -= samples,
            NoteEvent::PolyBrightness { timing, .. } => *timing -= samples,
            NoteEvent::MidiChannelPressure { timing, .. } => *timing -= samples,
            NoteEvent::MidiPitchBend { timing, .. } => *timing -= samples,
            NoteEvent::MidiCC { timing, .. } => *timing -= samples,
            NoteEvent::MidiProgramChange { timing, .. } => *timing -= samples,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMING: u32 = 5;

    #[test]
    fn test_note_on_midi_conversion() {
        let event = NoteEvent::NoteOn {
            timing: TIMING,
            voice_id: None,
            channel: 1,
            note: 2,
            // The value will be rounded in the conversion to MIDI, hence this overly specific value
            velocity: 0.6929134,
        };

        assert_eq!(
            NoteEvent::from_midi(TIMING, event.as_midi().unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn test_note_off_midi_conversion() {
        let event = NoteEvent::NoteOff {
            timing: TIMING,
            voice_id: None,
            channel: 1,
            note: 2,
            velocity: 0.6929134,
        };

        assert_eq!(
            NoteEvent::from_midi(TIMING, event.as_midi().unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn test_poly_pressure_midi_conversion() {
        let event = NoteEvent::PolyPressure {
            timing: TIMING,
            voice_id: None,
            channel: 1,
            note: 2,
            pressure: 0.6929134,
        };

        assert_eq!(
            NoteEvent::from_midi(TIMING, event.as_midi().unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn test_channel_pressure_midi_conversion() {
        let event = NoteEvent::MidiChannelPressure {
            timing: TIMING,
            channel: 1,
            pressure: 0.6929134,
        };

        assert_eq!(
            NoteEvent::from_midi(TIMING, event.as_midi().unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn test_pitch_bend_midi_conversion() {
        let event = NoteEvent::MidiPitchBend {
            timing: TIMING,
            channel: 1,
            value: 0.6929134,
        };

        assert_eq!(
            NoteEvent::from_midi(TIMING, event.as_midi().unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn test_cc_midi_conversion() {
        let event = NoteEvent::MidiCC {
            timing: TIMING,
            channel: 1,
            cc: 2,
            value: 0.6929134,
        };

        assert_eq!(
            NoteEvent::from_midi(TIMING, event.as_midi().unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn test_program_change_midi_conversion() {
        let event = NoteEvent::MidiProgramChange {
            timing: TIMING,
            channel: 1,
            program: 42,
        };

        assert_eq!(
            NoteEvent::from_midi(TIMING, event.as_midi().unwrap()).unwrap(),
            event
        );
    }
}
