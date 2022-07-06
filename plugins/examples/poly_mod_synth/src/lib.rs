use nih_plug::prelude::*;
use rand::Rng;
use rand_pcg::Pcg32;
use std::sync::Arc;

/// The number of simultaneous voices for this synth.
const NUM_VOICES: u32 = 16;

/// The maximum size of an audio block. We'll split up the audio in blocks and render smoothed
/// values to buffers since these values may need to be reused for multiple voices.
const MAX_BLOCK_SIZE: usize = 64;

/// A simple polyphonic synthesizer with support for CLAP's polyphonic modulation. See
/// `NoteEvent::PolyModulation` for another source of information on how to use this.
struct PolyModSynth {
    params: Arc<PolyModSynthParams>,

    /// A pseudo-random number generator. This will always be reseeded with the same seed when the
    /// synth is reset. That way the output is deterministic when rendering multiple times.
    prng: Pcg32,
    /// The synth's voices. Inactive voices will be set to `None` values.
    voices: [Option<Voice>; NUM_VOICES as usize],
    /// The next internal voice ID, used only to figure out the oldest voice for voice stealing.
    /// This is incremented by one each time a voice is created.
    next_internal_voice_id: u64,
}

#[derive(Default, Params)]
struct PolyModSynthParams {}

/// Data for a single synth voice. In a real synth where performance matter, you may want to use a
/// struct of arrays instead of having a struct for each voice.
#[derive(Debug, Clone)]
struct Voice {
    /// The identifier for this voice. Polyphonic modulation events are linked to a voice based on
    /// these IDs. If the host doesn't provide these IDs, then this is computed through
    /// `compute_fallback_voice_id()`. In that case polyphonic modulation will not work, but the
    /// basic note events will still have an effect.
    voice_id: i32,
    /// The note's channel, in `0..16`. Only used for the voice terminated event.
    channel: u8,
    /// The note's key/note, in `0..128`. Only used for the voice terminated event.
    note: u8,
    /// The voices internal ID. Each voice has an internal voice ID one higher than the previous
    /// voice. This is used to steal the last voice in case all 16 voices are in use.
    internal_voice_id: u64,

    /// The voice's current phase. This is randomized at the start of the voice
    phase: f32,
    /// The phase increment. This is based on the voice's frequency, derived from the note index.
    /// Since we don't support pitch expressions or pitch bend, this value stays constant for the
    /// duration of the voice.
    phase_delta: f32,
    /// The square root of the note's velocity. This is used as a gain multiplier.
    velocity_sqrt: f32,
}

impl Default for PolyModSynth {
    fn default() -> Self {
        Self {
            params: Arc::new(PolyModSynthParams::default()),

            prng: Pcg32::new(420, 1337),
            // `[None; N]` requires the `Some(T)` to be `Copy`able
            voices: [0; NUM_VOICES as usize].map(|_| None),
            next_internal_voice_id: 0,
        }
    }
}

impl Plugin for PolyModSynth {
    const NAME: &'static str = "Poly Mod Synth";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    // We won't need any MIDI CCs here, we just want notes and polyphonic modulation
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    // If the synth as a variable number of voices, you will need to call
    // `context.set_current_voice_capacity()` in `initialize()` and in `process()` (when the
    // capacity changes) to inform the host about this.
    fn reset(&mut self) {
        // This ensures the output is at least somewhat deterministic when rendering to audio
        self.prng = Pcg32::new(420, 1337);

        self.voices.fill(None);
        self.next_internal_voice_id = 0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        // NIH-plug has a block-splitting adapter for `Buffer`. While this works great for effect
        // plugins, for polyphonic synths the block size should be `min(MAX_BLOCK_SIZE,
        // num_remaining_samples, next_event_idx - block_start_idx)`. Because blocks also need to be
        // split on note events, it's easier to work with raw audio here and to do the splitting by
        // hand.
        let num_samples = buffer.len();
        let output = buffer.as_slice();

        let mut next_event = context.next_event();
        let mut block_start: usize = 0;
        let mut block_end: usize = MAX_BLOCK_SIZE.min(num_samples);
        while block_start < num_samples {
            // First of all, handle all note events that happen at the start of the block, and cut
            // the block short if another event happens before the end of it
            'events: loop {
                match next_event {
                    // If the event happens now, then we'll keep processing events
                    Some(event) if (event.timing() as usize) == block_start => {
                        // This synth doesn't support any of the polyphonic expression events. A
                        // real synth plugin however will want to support those.
                        match event {
                            NoteEvent::NoteOn {
                                timing,
                                voice_id,
                                channel,
                                note,
                                velocity,
                            } => {
                                let initial_phase: f32 = self.prng.gen();
                                let voice =
                                    self.start_voice(context, timing, voice_id, channel, note);

                                // TODO: Add and set the other fields
                                voice.phase = initial_phase;
                                voice.phase_delta =
                                    util::midi_note_to_freq(note) / context.transport().sample_rate;
                                voice.velocity_sqrt = velocity.sqrt();
                            }
                            NoteEvent::NoteOff {
                                timing,
                                voice_id,
                                channel,
                                note,
                                velocity: _,
                            } => {
                                // TODO: This should not immediately terminate the voice. For
                                //       obvious reasons.
                                self.terminate_voice(context, timing, voice_id, channel, note);
                            }
                            NoteEvent::Choke {
                                timing,
                                voice_id,
                                channel,
                                note,
                            } => {
                                self.terminate_voice(context, timing, voice_id, channel, note);
                            }
                            // TODO: Handle poly modulation
                            NoteEvent::PolyModulation {
                                timing,
                                voice_id,
                                poly_modulation_id,
                                normalized_offset,
                            } => todo!(),
                            NoteEvent::MonoAutomation {
                                timing,
                                poly_modulation_id,
                                normalized_value,
                            } => todo!(),
                            _ => (),
                        };

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

            // We'll start with silence, and then add the output from the active voices
            output[0][block_start..block_end].fill(0.0);
            output[1][block_start..block_end].fill(0.0);

            // TODO: Poly modulation
            // TODO: Amp envelope
            // TODO: Some form of band limiting
            // TODO: Filter
            for voice in self.voices.iter_mut().filter_map(|v| v.as_mut()) {
                for sample_idx in block_start..block_end {
                    // TODO: This should of course take the envelope and probably a poly mod param into account
                    // TODO: And as mentioned above, basic PolyBLEP or something
                    let gain = voice.velocity_sqrt;
                    let sample = (voice.phase * 2.0 - 1.0) * gain;

                    voice.phase += voice.phase_delta;
                    if voice.phase >= 1.0 {
                        voice.phase -= 1.0;
                    }

                    output[0][sample_idx] += sample;
                    output[1][sample_idx] += sample;
                }
            }

            // And then just keep processing blocks until we've run out of buffer to fill
            block_start = block_end;
            block_end = (block_start + MAX_BLOCK_SIZE).min(num_samples);
        }

        ProcessStatus::Normal
    }
}

impl PolyModSynth {
    /// Get an active voice by its voice ID, if the voice exists
    fn get_voice_mut(&mut self, voice_id: i32) -> Option<&mut Voice> {
        self.voices.iter_mut().find_map(|voice| match voice {
            Some(voice) if voice.voice_id == voice_id => Some(voice),
            _ => None,
        })
    }

    /// Start a new voice with the given voice ID. If all voices are currently in use, the oldest
    /// voice will be stolen. Returns a reference to the new voice.
    fn start_voice(
        &mut self,
        context: &mut impl ProcessContext,
        sample_offset: u32,
        voice_id: Option<i32>,
        channel: u8,
        note: u8,
    ) -> &mut Voice {
        let new_voice = Voice {
            voice_id: voice_id.unwrap_or_else(|| compute_fallback_voice_id(note, channel)),
            internal_voice_id: self.next_internal_voice_id,
            channel,
            note,

            velocity_sqrt: 1.0,
            phase: 0.0,
            phase_delta: 0.0,
        };
        self.next_internal_voice_id = self.next_internal_voice_id.wrapping_add(1);

        // Can't use `.iter_mut().find()` here because nonlexical lifetimes don't apply to return
        // values
        match self.voices.iter().position(|voice| voice.is_none()) {
            Some(free_voice_idx) => {
                self.voices[free_voice_idx] = Some(new_voice);
                return self.voices[free_voice_idx].as_mut().unwrap();
            }
            None => {
                // If there is no free voice, find and steal the oldest one
                // SAFETY: We can skip a lot of checked unwraps here since we already know all voices are in
                //         use
                let oldest_voice = unsafe {
                    self.voices
                        .iter_mut()
                        .min_by_key(|voice| voice.as_ref().unwrap_unchecked().internal_voice_id)
                        .unwrap_unchecked()
                };

                // The stolen voice needs to be terminated so the host can reuse its modulation
                // resources
                {
                    let oldest_voice = oldest_voice.as_ref().unwrap();
                    context.send_event(NoteEvent::VoiceTerminated {
                        timing: sample_offset,
                        voice_id: Some(oldest_voice.voice_id),
                        channel: oldest_voice.channel,
                        note: oldest_voice.note,
                    });
                }

                *oldest_voice = Some(new_voice);
                return oldest_voice.as_mut().unwrap();
            }
        }
    }

    /// Terminate one or more voice, removing it from the pool and informing the host that the voice
    /// has ended. If `voice_id` is not provided, then this will terminate all matching voices.
    fn terminate_voice(
        &mut self,
        context: &mut impl ProcessContext,
        sample_offset: u32,
        voice_id: Option<i32>,
        channel: u8,
        note: u8,
    ) {
        // TODO: If voice ID = none, terminate all matching voices
        for voice in self.voices.iter_mut() {
            match voice {
                Some(Voice {
                    voice_id: candidate_voice_id,
                    channel: candidate_channel,
                    note: candidate_note,
                    ..
                }) if voice_id == Some(*candidate_voice_id)
                    || (channel == *candidate_channel && note == *candidate_note) =>
                {
                    // This event is very important, as it allows the host to manage its own modulation
                    // voices
                    context.send_event(NoteEvent::VoiceTerminated {
                        timing: sample_offset,
                        // Notice how we always send the terminated voice ID here
                        voice_id: Some(*candidate_voice_id),
                        channel,
                        note,
                    });
                    *voice = None;

                    // If this targetted a single voice ID, we're done here. Otherwise there may be
                    // multiple overlapping voices as we enabled support for that in the
                    // `PolyModulationConfig`.
                    if voice_id.is_some() {
                        return;
                    }
                }
                _ => (),
            }
        }
    }
}

/// Compute a voice ID in case the host doesn't provide them. Polyphonic modulation will not work in
/// this case, but playing notes will.
const fn compute_fallback_voice_id(note: u8, channel: u8) -> i32 {
    note as i32 | ((channel as i32) << 16)
}

impl ClapPlugin for PolyModSynth {
    const CLAP_ID: &'static str = "com.moist-plugins-gmbh.poly-mod-synth";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A simple polyphonic synthesizer with support for polyphonic modulation");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];

    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = Some(PolyModulationConfig {
        // If the plugin's voice capacity changes at runtime (for instance, when switching to a
        // monophonic mode), then the plugin should inform the host in the `initialize()` function
        // as well as in the `process()` function if it changes at runtime using
        // `context.set_current_voice_capacity()`
        max_voice_capacity: NUM_VOICES,
        // This enables voice stacking in Bitwig.
        supports_overlapping_voices: true,
    });
}

// The VST3 verison of this plugin isn't too interesting as it will not support polyphonic
// modulation
impl Vst3Plugin for PolyModSynth {
    const VST3_CLASS_ID: [u8; 16] = *b"PolyM0dSynth1337";
    const VST3_CATEGORIES: &'static str = "Instrument|Synth";
}

nih_export_clap!(PolyModSynth);
nih_export_vst3!(PolyModSynth);
