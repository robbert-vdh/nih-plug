// Buffr Glitch: a MIDI-controlled buffer repeater
// Copyright (C) 2022-2023 Robbert van der Helm
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
mod envelope;

/// The maximum number of octaves the sample can be pitched down. This is used in calculating the
/// recording buffer's size.
pub const MAX_OCTAVE_SHIFT: u32 = 2;

struct BuffrGlitch {
    params: Arc<BuffrGlitchParams>,

    sample_rate: f32,
    /// The ring buffer samples are recorded to and played back from when a key is held down.
    buffer: buffer::RingBuffer,

    /// The MIDI note ID of the last note, if a note pas pressed.
    //
    // TODO: Add polyphony support, this is just a quick proof of concept.
    midi_note_id: Option<u8>,
    /// The gain scaling from the velocity. If velocity sensitive mode is enabled, then this is the `[0, 1]` velocity
    /// devided by `100/127` such that MIDI velocity 100 corresponds to 1.0 gain.
    velocity_gain: f32,
    /// The envelope genrator used during playback. This handles both gain smoothing as well as fade
    /// ins and outs to prevent clicks.
    amp_envelope: envelope::AREnvelope,
}

#[derive(Params)]
struct BuffrGlitchParams {
    /// From 0 to 1, how much of the dry signal to mix in. This defaults to 1 but it can be turned
    /// down to use Buffr Glitch as more of a synth.
    #[id = "dry_mix"]
    dry_level: FloatParam,
    /// Makes the effect velocity sensitive. `100/127` corresponds to `1.0` gain.
    #[id = "velocity_sensitive"]
    velocity_sensitive: BoolParam,
    /// The number of octaves the input signal should be increased or decreased by. Useful to allow
    /// larger grain sizes.
    #[id = "octave_shift"]
    octave_shift: IntParam,

    /// The attack time in milliseconds. Useful to avoid clicks. Or to introduce them if that's
    /// aesthetically pleasing.
    #[id = "attack_ms"]
    attack_ms: FloatParam,
    /// The attack time in milliseconds. Useful to avoid clicks. Or to introduce them if that's
    /// aesthetically pleasing.
    #[id = "release_ms"]
    release_ms: FloatParam,
    /// The length of the loop crossfade to use, in milliseconds. This will cause the start of the
    /// loop to be faded into the last `(crossfade_ms/2)` ms of the loop region, and the part after
    /// the end to be faded into the first `(crossfade_ms/2)` ms of the loop after the first
    /// ieration.
    #[id = "crossfade_ms"]
    crossfade_ms: FloatParam,
}

impl Default for BuffrGlitch {
    fn default() -> Self {
        Self {
            params: Arc::new(BuffrGlitchParams::default()),

            sample_rate: 1.0,
            buffer: buffer::RingBuffer::default(),

            midi_note_id: None,
            velocity_gain: 1.0,
            amp_envelope: envelope::AREnvelope::default(),
        }
    }
}

impl Default for BuffrGlitchParams {
    fn default() -> Self {
        Self {
            dry_level: FloatParam::new(
                "Dry Level",
                1.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 1.0,
                    factor: FloatRange::gain_skew_factor(util::MINUS_INFINITY_DB, 0.0),
                },
            )
            .with_smoother(SmoothingStyle::Exponential(10.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            velocity_sensitive: BoolParam::new("Velocity Sensitive", false),
            octave_shift: IntParam::new(
                "Octave Shift",
                0,
                IntRange::Linear {
                    min: -(MAX_OCTAVE_SHIFT as i32),
                    max: MAX_OCTAVE_SHIFT as i32,
                },
            ),

            attack_ms: FloatParam::new(
                "Attack",
                2.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.001),
            release_ms: FloatParam::new(
                "Release",
                2.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.001),
            crossfade_ms: FloatParam::new(
                "Crossfade",
                2.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            // This doesn't need smoothing because the value is set when the note is held down and cannot be changed afterwards
            .with_unit(" ms")
            .with_step_size(0.001),
        }
    }
}

impl Plugin for BuffrGlitch {
    const NAME: &'static str = "Buffr Glitch";
    const VENDOR: &'static str = "Robbert van der Helm";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
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
        self.amp_envelope.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();
        for (sample_idx, channel_samples) in buffer.iter_samples().enumerate() {
            let dry_amount = self.params.dry_level.smoothed.next();

            // TODO: Split blocks based on events when adding polyphony, this is just a simple proof
            //       of concept
            while let Some(event) = next_event {
                if event.timing() > sample_idx as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        // We don't keep a stack of notes right now. At some point we'll want to
                        // make this polyphonic anyways.
                        // TOOD: Also add an option to use velocity or poly pressure
                        self.midi_note_id = Some(note);
                        self.velocity_gain = if self.params.velocity_sensitive.value() {
                            velocity / (100.0 / 127.0)
                        } else {
                            1.0
                        };

                        self.amp_envelope.soft_reset();
                        self.amp_envelope.set_target(self.velocity_gain);

                        // We'll copy audio to the playback buffer to match the pitch of the note
                        // that was just played. The octave shift parameter makes it possible to get
                        // larger window sizes.
                        let note_frequency = util::midi_note_to_freq(note)
                            * 2.0f32.powi(self.params.octave_shift.value());
                        self.buffer
                            .prepare_playback(note_frequency, self.params.crossfade_ms.value());
                    }
                    NoteEvent::NoteOff { note, .. } if self.midi_note_id == Some(note) => {
                        // Playback still continues until the release is done.
                        self.amp_envelope.start_release();
                        self.midi_note_id = None;
                    }
                    NoteEvent::PolyVolume { note, gain, .. } if self.midi_note_id == Some(note) => {
                        self.amp_envelope.set_target(self.velocity_gain * gain);
                    }
                    _ => (),
                }

                next_event = context.next_event();
            }

            // When a note is being held, we'll replace the input audio with the looping contents of
            // the playback buffer
            // TODO: At some point also handle polyphony here
            if self.midi_note_id.is_some() || self.amp_envelope.is_releasing() {
                self.amp_envelope
                    .set_attack_time(self.sample_rate, self.params.attack_ms.value());
                self.amp_envelope
                    .set_release_time(self.sample_rate, self.params.release_ms.value());

                // FIXME: This should fade in and out from the dry buffer
                let gain = self.amp_envelope.next();
                for (channel_idx, sample) in channel_samples.into_iter().enumerate() {
                    // This will start recording on the first iteration, and then loop the recorded
                    // buffer afterwards
                    let result = self.buffer.next_sample(channel_idx, *sample);

                    *sample = result * gain;
                }
            } else {
                for sample in channel_samples.into_iter() {
                    *sample *= dry_amount;
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
