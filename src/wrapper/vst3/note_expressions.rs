//! Special handling for note expressions, because VST3 makes this a lot more complicated than it
//! needs to be. We only support the predefined expressions.

use vst3_sys::vst::{NoteExpressionValueEvent, NoteOnEvent};

use crate::midi::NoteEvent;

type MidiNote = u8;
type MidiChannel = u8;
type NoteId = i32;

/// The number of notes we'll keep track of for mapping note IDs to channel+note combinations.
const NOTE_IDS_LEN: usize = 32;

/// `kVolumeTypeID`
pub const VOLUME_EXPRESSION_ID: u32 = 0;
/// `kPanTypeId`
pub const PAN_EXPRESSION_ID: u32 = 1;
/// `kTuningTypeID`
pub const TUNING_EXPRESSION_ID: u32 = 2;
/// `kVibratoTypeID`
pub const VIBRATO_EXPRESSION_ID: u32 = 3;
/// `kExpressionTypeID`
pub const EXPRESSION_EXPRESSION_ID: u32 = 4;
/// `kBrightnessTypeID`
pub const BRIGHTNESS_EXPRESSION_ID: u32 = 5;

/// VST3 has predefined note expressions just like CLAP, but unlike the other note events these
/// expressions are identified only with a note ID. To account for that, we'll keep track of the
/// most recent note IDs we've encountered so we can later map those IDs back to a note and channel
/// combination.
#[derive(Debug, Default)]
pub struct NoteExpressionController {
    /// The last 32 note IDs we've seen. We'll do a linear search every time we receive a note
    /// expression value event to find the matching note and channel.
    note_ids: [(NoteId, MidiNote, MidiChannel); NOTE_IDS_LEN],
    /// The index in the `note_ids` ring buffer the next event should be inserted at, wraps back
    /// around to 0 when reaching the end.
    note_ids_idx: usize,
}

impl NoteExpressionController {
    /// Register the note ID from a note on event so it can later be retrieved when handling a note
    /// expression value event.
    pub fn register_note(&mut self, event: &NoteOnEvent) {
        self.note_ids[self.note_ids_idx] = (event.note_id, event.pitch as u8, event.channel as u8);
        self.note_ids_idx = (self.note_ids_idx + 1) % NOTE_IDS_LEN;
    }

    /// Translate the note expression value event into an internal NIH-plug event, if we handle the
    /// expression type from the note expression value event. The timing is provided here because we
    /// may be splitting buffers on inter-buffer parameter changes.
    pub fn translate_event(
        &self,
        timing: u32,
        event: &NoteExpressionValueEvent,
    ) -> Option<NoteEvent> {
        let (_, note, channel) = *self
            .note_ids
            .iter()
            .find(|(note_id, _, _)| *note_id == event.note_id)?;

        match event.type_id {
            VOLUME_EXPRESSION_ID => Some(NoteEvent::PolyVolume {
                timing,
                channel,
                note,
                // Because expression values in VST3 are always in the `[0, 1]` range, they added a
                // 4x scaling factor here to allow the values to go from -infinity to +12 dB
                gain: event.value as f32 * 4.0,
            }),
            PAN_EXPRESSION_ID => Some(NoteEvent::PolyPan {
                timing,
                channel,
                note,
                // Our panning expressions are symmetrical around 0
                pan: (event.value as f32 * 2.0) - 1.0,
            }),
            TUNING_EXPRESSION_ID => Some(NoteEvent::PolyTuning {
                timing,
                channel,
                note,
                // This denormalized to the same [-120, 120] range used by CLAP and our expression
                // events
                tuning: 240.0 * (event.value as f32 - 0.5),
            }),
            VIBRATO_EXPRESSION_ID => Some(NoteEvent::PolyVibrato {
                timing,
                channel,
                note,
                vibrato: event.value as f32,
            }),
            EXPRESSION_EXPRESSION_ID => Some(NoteEvent::PolyBrightness {
                timing,
                channel,
                note,
                brightness: event.value as f32,
            }),
            BRIGHTNESS_EXPRESSION_ID => Some(NoteEvent::PolyExpression {
                timing,
                channel,
                note,
                expression: event.value as f32,
            }),
            _ => None,
        }
    }

    /// Translate a NIH-plug note expression event a VST3 `NoteExpressionValueEvent`. Will return
    /// `None` if the event is not a polyphonic expression event, i.e. one of the events handled by
    /// `translate_event()`.
    pub fn translate_event_reverse(
        note_id: i32,
        event: &NoteEvent,
    ) -> Option<NoteExpressionValueEvent> {
        match &event {
            NoteEvent::PolyVolume { gain, .. } => Some(NoteExpressionValueEvent {
                type_id: VOLUME_EXPRESSION_ID,
                note_id,
                value: *gain as f64 / 4.0,
            }),
            NoteEvent::PolyPan { pan, .. } => Some(NoteExpressionValueEvent {
                type_id: PAN_EXPRESSION_ID,
                note_id,
                value: (*pan as f64 + 1.0) / 2.0,
            }),
            NoteEvent::PolyTuning { tuning, .. } => Some(NoteExpressionValueEvent {
                type_id: TUNING_EXPRESSION_ID,
                note_id,
                value: (*tuning as f64 / 240.0) + 0.5,
            }),
            NoteEvent::PolyVibrato { vibrato, .. } => Some(NoteExpressionValueEvent {
                type_id: VIBRATO_EXPRESSION_ID,
                note_id,
                value: *vibrato as f64,
            }),
            NoteEvent::PolyExpression { expression, .. } => Some(NoteExpressionValueEvent {
                type_id: EXPRESSION_EXPRESSION_ID,
                note_id,
                value: *expression as f64,
            }),
            NoteEvent::PolyBrightness { brightness, .. } => Some(NoteExpressionValueEvent {
                type_id: BRIGHTNESS_EXPRESSION_ID,
                note_id,
                value: *brightness as f64,
            }),
            _ => None,
        }
    }
}
