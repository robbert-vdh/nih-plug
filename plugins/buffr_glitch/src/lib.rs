// Buffr Glitch: a MIDI-controlled buffer repeater
// Copyright (C) 2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use nih_plug::prelude::*;
use std::sync::Arc;

mod buffer;

struct BuffrGlitch {
    params: Arc<BuffrGlitchParams>,

    sample_rate: f32,
    /// The ring buffer we'll write samples to. When a key is held down, we'll stop writing samples
    /// and instead keep reading from this buffer until the key is released.
    buffer: buffer::RingBuffer,

    /// The MIDI note ID of the last note, if a note pas pressed.
    //
    // TODO: Add polyphony support, this is just a quick proof of concept.
    midi_note_id: Option<u8>,
}

// TODO: Normalize option
#[derive(Params)]
struct BuffrGlitchParams {}

impl Default for BuffrGlitch {
    fn default() -> Self {
        Self {
            params: Arc::new(BuffrGlitchParams::default()),

            sample_rate: 1.0,
            buffer: buffer::RingBuffer::default(),

            midi_note_id: None,
        }
    }
}

impl Default for BuffrGlitchParams {
    fn default() -> Self {
        Self {}
    }
}

impl Plugin for BuffrGlitch {
    const NAME: &'static str = "Buffr Glitch";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = "https://github.com/robbert-vdh/nih-plug";
    const EMAIL: &'static str = "mail@robbertvanderhelm.nl";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const DEFAULT_INPUT_CHANNELS: u32 = 2;
    const DEFAULT_OUTPUT_CHANNELS: u32 = 2;

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn initialize(
        &mut self,
        bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.buffer.resize(
            bus_config.num_input_channels as usize,
            buffer_config.sample_rate,
        );

        true
    }

    fn reset(&mut self) {
        self.buffer.reset();
        self.midi_note_id = None;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();

        for (sample_idx, channel_samples) in buffer.iter_samples().enumerate() {
            // TODO: Split blocks based on events when adding polyphony, this is just a simple proof
            //       of concept
            while let Some(event) = next_event {
                if event.timing() > sample_idx as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, .. } => {
                        // We don't keep a stack of notes right now. At some point we'll want to
                        // make this polyphonic anyways.
                        // TOOD: Also add an option to use velocity or poly pressure
                        self.midi_note_id = Some(note);

                        // We'll copy audio to the playback buffer to match the pitch of the note
                        // that was just played
                        self.buffer.prepare_playback(util::midi_note_to_freq(note));
                    }
                    NoteEvent::NoteOff { note, .. } if self.midi_note_id == Some(note) => {
                        // A NoteOff for the currently playing note immediately ends playback
                        self.midi_note_id = None;
                    }
                    _ => (),
                }

                next_event = context.next_event();
            }

            // When a note is being held, we'll replace the input audio with the looping contents of
            // the playback buffer
            if self.midi_note_id.is_some() {
                for (channel_idx, sample) in channel_samples.into_iter().enumerate() {
                    // New audio still needs to be recorded when the note is held to prepare for new
                    // notes
                    // TODO: At some point also handle polyphony here
                    self.buffer.push(channel_idx, *sample);

                    *sample = self.buffer.next_playback_sample(channel_idx);
                }
            } else {
                for (channel_idx, sample) in channel_samples.into_iter().enumerate() {
                    self.buffer.push(channel_idx, *sample);
                }
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for BuffrGlitch {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.buffr-glitch";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("MIDI-controller buffer repeat");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Glitch,
    ];
}

impl Vst3Plugin for BuffrGlitch {
    const VST3_CLASS_ID: [u8; 16] = *b"BuffrGlitch.RvdH";
    const VST3_CATEGORIES: &'static str = "Fx";
}

nih_export_clap!(BuffrGlitch);
nih_export_vst3!(BuffrGlitch);
