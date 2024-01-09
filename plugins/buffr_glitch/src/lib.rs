// Buffr Glitch: a MIDI-controlled buffer repeater
// Copyright (C) 2022-2024 Robbert van der Helm
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

/// The number of channels supported by the plugin. We'll only do stereo for now.
const NUM_CHANNELS: u32 = 2;
/// The maximum size of an audio block. We'll split up the audio in blocks and render smoothed
/// values to buffers since these values may need to be reused for multiple voices.
const MAX_BLOCK_SIZE: usize = 64;

struct BuffrGlitch {
    params: Arc<BuffrGlitchParams>,

    sample_rate: f32,
    voices: [Voice; 8],
}

/// A single voice, Buffr Glitch can be used in polypnoic mode. And even if only a single note is
/// played at a time, this is needed for the amp envelope release to work correctly.
///
/// Use the [`Voice::is_active()`] method to determine whether a voice is still playing.
struct Voice {
    /// The ring buffer samples are recorded to and played back from when a key is held down.
    buffer: buffer::RingBuffer,

    /// The MIDI note ID of the last note, if a note is pressed.
    midi_note_id: Option<u8>,
    /// The gain scaling from the velocity. If velocity sensitive mode is enabled, then this is the `[0, 1]` velocity
    /// devided by `100/127` such that MIDI velocity 100 corresponds to 1.0 gain.
    velocity_gain: f32,
    /// The gain from the gain note expression.
    gain_expression_gain: Smoother<f32>,
    /// The envelope genrator used during playback. Produces a `[0, 1]` result.
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
            voices: Default::default(),
        }
    }
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            buffer: buffer::RingBuffer::default(),

            midi_note_id: None,
            velocity_gain: 1.0,
            // This is initialized in `initialize()` since this relies on the sample rate
            gain_expression_gain: Smoother::new(SmoothingStyle::Linear(5.0)),
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

    // We'll only do stereo for now
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(NUM_CHANNELS),
        main_output_channels: NonZeroU32::new(NUM_CHANNELS),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let num_output_channels = audio_io_layout
            .main_output_channels
            .expect("Plugin does not have a main output")
            .get() as usize;
        self.sample_rate = buffer_config.sample_rate;
        for voice in &mut self.voices {
            voice
                .buffer
                .resize(num_output_channels, buffer_config.sample_rate);
        }

        true
    }

    fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();
        let output = buffer.as_slice();

        let mut next_event = context.next_event();
        let mut block_start: usize = 0;
        let mut block_end: usize = MAX_BLOCK_SIZE.min(num_samples);
        while block_start < num_samples {
            // Keep processing events until all events at or before `block_start` have been
            // processed
            'events: loop {
                match next_event {
                    // If the event happens now, then we'll keep processing events
                    Some(event) if (event.timing() as usize) <= block_start => {
                        match event {
                            NoteEvent::NoteOn { note, velocity, .. } => {
                                let new_voice_id = self.new_voice_id();
                                self.voices[new_voice_id].note_on(&self.params, note, velocity);
                            }
                            NoteEvent::NoteOff { note, .. } => {
                                for voice in &mut self.voices {
                                    if voice.midi_note_id == Some(note) {
                                        // Playback still continues until the release is done.
                                        voice.note_off();
                                        break;
                                    }
                                }
                            }
                            NoteEvent::PolyVolume { note, gain, .. } => {
                                for voice in &mut self.voices {
                                    if voice.midi_note_id == Some(note) {
                                        voice
                                            .gain_expression_gain
                                            .set_target(self.sample_rate, gain);
                                        break;
                                    }
                                }
                            }
                            _ => (),
                        }

                        next_event = context.next_event();
                    }
                    // If the event happens before the end of the block, then the block should be cut
                    // short so the next block starts at the event
                    Some(event) if (event.timing() as usize) < block_end => {
                        block_end = event.timing() as usize;
                        break 'events;
                    }
                    _ => break 'events,
                }
            }

            // The output buffer is filled with the active voices, so we need to read the inptu
            // first
            let block_len = block_end - block_start;
            let mut input = [[0.0; MAX_BLOCK_SIZE]; 2];
            input[0][..block_len].copy_from_slice(&output[0][block_start..block_end]);
            input[1][..block_len].copy_from_slice(&output[1][block_start..block_end]);

            // The dry signal is mixed back in depending on th maximum voice amplitude envelope
            let mut max_voice_amp_envelope = [0.0f32; MAX_BLOCK_SIZE];

            // We'll empty the buffer, and then add the dry signal back in as needed
            output[0][block_start..block_end].fill(0.0);
            output[1][block_start..block_end].fill(0.0);
            for voice in self.voices.iter_mut().filter(|v| v.is_active()) {
                let mut voice_amp_envelope = [0.0; MAX_BLOCK_SIZE];
                voice
                    .amp_envelope
                    .set_attack_time(self.sample_rate, self.params.attack_ms.value());
                voice
                    .amp_envelope
                    .set_release_time(self.sample_rate, self.params.release_ms.value());
                voice
                    .amp_envelope
                    .next_block(&mut voice_amp_envelope, block_len);
                let mut voice_gain_expression_gain = [0.0; MAX_BLOCK_SIZE];
                voice
                    .gain_expression_gain
                    .next_block(&mut voice_gain_expression_gain, block_len);

                for (value_idx, sample_idx) in (block_start..block_end).enumerate() {
                    max_voice_amp_envelope[value_idx] =
                        max_voice_amp_envelope[value_idx].max(voice_amp_envelope[value_idx]);
                    let amp = voice.velocity_gain
                        * voice_gain_expression_gain[value_idx]
                        * voice_amp_envelope[value_idx];

                    // This will start recording on the first iteration, and then loop the recorded
                    // buffer afterwards
                    output[0][sample_idx] += voice.buffer.next_sample(0, input[0][value_idx]) * amp;
                    output[1][sample_idx] += voice.buffer.next_sample(1, input[1][value_idx]) * amp;
                }
            }

            // The dry signal is mixed back in depending on the amplitude of the currently playing
            // voices
            let mut dry_level = [0.0; MAX_BLOCK_SIZE];
            self.params
                .dry_level
                .smoothed
                .next_block(&mut dry_level, block_len);
            for (value_idx, sample_idx) in (block_start..block_end).enumerate() {
                let gain = (1.0 - max_voice_amp_envelope[value_idx]) * dry_level[value_idx];
                output[0][sample_idx] += input[0][value_idx] * gain;
                output[1][sample_idx] += input[1][value_idx] * gain;
            }

            // And then just keep processing blocks until we've run out of buffer to fill
            block_start = block_end;
            block_end = (block_start + MAX_BLOCK_SIZE).min(num_samples);
        }

        ProcessStatus::Normal
    }
}

impl BuffrGlitch {
    /// Find the ID of a voice that is either unused or that is quietest if all voices are in use.
    /// This does not do anything to the voice to end it.
    pub fn new_voice_id(&self) -> usize {
        for (voice_id, voice) in self.voices.iter().enumerate() {
            if !voice.is_active() {
                return voice_id;
            }
        }

        // Prefer stealing releasing voices if possible
        if let Some((quietest_voice_id, _)) = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, voice)| voice.amp_envelope.is_releasing())
            .min_by(|(_, voice_a), (_, voice_b)| {
                f32::total_cmp(
                    &voice_a.amp_envelope.current(),
                    &voice_b.amp_envelope.current(),
                )
            })
        {
            return quietest_voice_id;
        }

        let (quietest_voice_id, _) = self
            .voices
            .iter()
            .enumerate()
            .min_by(|(_, voice_a), (_, voice_b)| {
                f32::total_cmp(
                    &voice_a.amp_envelope.current(),
                    &voice_b.amp_envelope.current(),
                )
            })
            .unwrap();

        quietest_voice_id
    }
}

impl Voice {
    pub fn reset(&mut self) {
        self.buffer.reset();
        self.midi_note_id = None;
        self.amp_envelope.reset();
    }

    /// Prepare playback on note on.
    pub fn note_on(&mut self, params: &BuffrGlitchParams, midi_note_id: u8, velocity: f32) {
        self.midi_note_id = Some(midi_note_id);
        self.velocity_gain = if params.velocity_sensitive.value() {
            velocity / (100.0 / 127.0)
        } else {
            1.0
        };
        self.gain_expression_gain.reset(1.0);
        self.amp_envelope.reset();

        // We'll copy audio to the playback buffer to match the pitch of the note
        // that was just played. The octave shift parameter makes it possible to get
        // larger window sizes.
        let note_frequency =
            util::midi_note_to_freq(midi_note_id) * 2.0f32.powi(params.octave_shift.value());
        self.buffer
            .prepare_playback(note_frequency, params.crossfade_ms.value());
    }

    /// Start releasing the note.
    pub fn note_off(&mut self) {
        self.amp_envelope.start_release();
        self.midi_note_id = None;
    }

    /// Whether the voice is (still) active.
    pub fn is_active(&self) -> bool {
        self.midi_note_id.is_some() || self.amp_envelope.is_releasing()
    }
}

impl ClapPlugin for BuffrGlitch {
    const CLAP_ID: &'static str = "nl.robbertvanderhelm.buffr-glitch";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("MIDI-controller buffer repeat");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
        ClapFeature::Glitch,
    ];
}

impl Vst3Plugin for BuffrGlitch {
    const VST3_CLASS_ID: [u8; 16] = *b"BuffrGlitch.RvdH";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Stereo,
        Vst3SubCategory::Custom("Glitch"),
    ];
}

nih_export_clap!(BuffrGlitch);
nih_export_vst3!(BuffrGlitch);
