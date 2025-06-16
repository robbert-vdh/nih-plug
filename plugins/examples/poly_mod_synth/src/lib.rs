use nih_plug::prelude::*;
use rand::Rng;
use rand_pcg::Pcg32;
use std::sync::Arc;

/// The number of simultaneous voices for this synth.
const NUM_VOICES: u32 = 16;
/// The maximum size of an audio block. We'll split up the audio in blocks and render smoothed
/// values to buffers since these values may need to be reused for multiple voices.
const MAX_BLOCK_SIZE: usize = 64;

// Polyphonic modulation works by assigning integer IDs to parameters. Pattern matching on these in
// `PolyModulation` and `MonoAutomation` events makes it possible to easily link these events to the
// correct parameter.
const GAIN_POLY_MOD_ID: u32 = 0;

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

#[derive(Params)]
struct PolyModSynthParams {
    /// A voice's gain. This can be polyphonically modulated.
    #[id = "gain"]
    gain: FloatParam,
    /// The amplitude envelope attack time. This is the same for every voice.
    #[id = "amp_atk"]
    amp_attack_ms: FloatParam,
    /// The amplitude envelope release time. This is the same for every voice.
    #[id = "amp_rel"]
    amp_release_ms: FloatParam,
}

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
    /// The square root of the note's velocity. This is used as a gain multiplier.
    velocity_sqrt: f32,

    /// The voice's current phase. This is randomized at the start of the voice
    phase: f32,
    /// The phase increment. This is based on the voice's frequency, derived from the note index.
    /// Since we don't support pitch expressions or pitch bend, this value stays constant for the
    /// duration of the voice.
    phase_delta: f32,
    /// Whether the key has been released and the voice is in its release stage. The voice will be
    /// terminated when the amplitude envelope hits 0 while the note is releasing.
    releasing: bool,
    /// Fades between 0 and 1 with timings based on the global attack and release settings.
    amp_envelope: Smoother<f32>,

    /// If this voice has polyphonic gain modulation applied, then this contains the normalized
    /// offset and a smoother.
    voice_gain: Option<(f32, Smoother<f32>)>,
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

impl Default for PolyModSynthParams {
    fn default() -> Self {
        Self {
            gain: FloatParam::new(
                "Gain",
                util::db_to_gain(-12.0),
                // Because we're representing gain as decibels the range is already logarithmic
                FloatRange::Linear {
                    min: util::db_to_gain(-36.0),
                    max: util::db_to_gain(0.0),
                },
            )
            // This enables polyphonic mdoulation for this parameter by representing all related
            // events with this ID. After enabling this, the plugin **must** start sending
            // `VoiceTerminated` events to the host whenever a voice has ended.
            .with_poly_modulation_id(GAIN_POLY_MOD_ID)
            .with_smoother(SmoothingStyle::Logarithmic(5.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            amp_attack_ms: FloatParam::new(
                "Attack",
                200.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            // These parameters are global (and they cannot be changed once the voice has started).
            // They also don't need any smoothing themselves because they affect smoothing
            // coefficients.
            .with_step_size(0.1)
            .with_unit(" ms"),
            amp_release_ms: FloatParam::new(
                "Release",
                100.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(0.1)
            .with_unit(" ms"),
        }
    }
}

impl Plugin for PolyModSynth {
    const NAME: &'static str = "Poly Mod Synth";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // We won't need any MIDI CCs here, we just want notes and polyphonic modulation
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

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
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // NIH-plug has a block-splitting adapter for `Buffer`. While this works great for effect
        // plugins, for polyphonic synths the block size should be `min(MAX_BLOCK_SIZE,
        // num_remaining_samples, next_event_idx - block_start_idx)`. Because blocks also need to be
        // split on note events, it's easier to work with raw audio here and to do the splitting by
        // hand.
        let num_samples = buffer.samples();
        let sample_rate = context.transport().sample_rate;
        let output = buffer.as_slice();

        let mut next_event = context.next_event();
        let mut block_start: usize = 0;
        let mut block_end: usize = MAX_BLOCK_SIZE.min(num_samples);
        while block_start < num_samples {
            // First of all, handle all note events that happen at the start of the block, and cut
            // the block short if another event happens before the end of it. To handle polyphonic
            // modulation for new notes properly, we'll keep track of the next internal note index
            // at the block's start. If we receive polyphonic modulation that matches a voice that
            // has an internal note ID that's great than or equal to this one, then we should start
            // the note's smoother at the new value instead of fading in from the global value.
            let this_sample_internal_voice_id_start = self.next_internal_voice_id;
            'events: loop {
                match next_event {
                    // If the event happens now, then we'll keep processing events
                    Some(event) if (event.timing() as usize) <= block_start => {
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
                                // This starts with the attack portion of the amplitude envelope
                                let amp_envelope = Smoother::new(SmoothingStyle::Exponential(
                                    self.params.amp_attack_ms.value(),
                                ));
                                amp_envelope.reset(0.0);
                                amp_envelope.set_target(sample_rate, 1.0);

                                let voice =
                                    self.start_voice(context, timing, voice_id, channel, note);
                                voice.velocity_sqrt = velocity.sqrt();
                                voice.phase = initial_phase;
                                voice.phase_delta = util::midi_note_to_freq(note) / sample_rate;
                                voice.amp_envelope = amp_envelope;
                            }
                            NoteEvent::NoteOff {
                                timing: _,
                                voice_id,
                                channel,
                                note,
                                velocity: _,
                            } => {
                                self.start_release_for_voices(sample_rate, voice_id, channel, note)
                            }
                            NoteEvent::Choke {
                                timing,
                                voice_id,
                                channel,
                                note,
                            } => {
                                self.choke_voices(context, timing, voice_id, channel, note);
                            }
                            NoteEvent::PolyModulation {
                                timing: _,
                                voice_id,
                                poly_modulation_id,
                                normalized_offset,
                            } => {
                                // Polyphonic modulation events are matched to voices using the
                                // voice ID, and to parameters using the poly modulation ID. The
                                // host will probably send a modulation event every N samples. This
                                // will happen before the voice is active, and of course also after
                                // it has been terminated (because the host doesn't know that it
                                // will be). Because of that, we won't print any assertion failures
                                // when we can't find the voice index here.
                                if let Some(voice_idx) = self.get_voice_idx(voice_id) {
                                    let voice = self.voices[voice_idx].as_mut().unwrap();

                                    match poly_modulation_id {
                                        GAIN_POLY_MOD_ID => {
                                            // This should either create a smoother for this
                                            // modulated parameter or update the existing one.
                                            // Notice how this uses the parameter's unmodulated
                                            // normalized value in combination with the normalized
                                            // offset to create the target plain value
                                            let target_plain_value = self
                                                .params
                                                .gain
                                                .preview_modulated(normalized_offset);
                                            let (_, smoother) =
                                                voice.voice_gain.get_or_insert_with(|| {
                                                    (
                                                        normalized_offset,
                                                        self.params.gain.smoothed.clone(),
                                                    )
                                                });

                                            // If this `PolyModulation` events happens on the
                                            // same sample as a voice's `NoteOn` event, then it
                                            // should immediately use the modulated value
                                            // instead of slowly fading in
                                            if voice.internal_voice_id
                                                >= this_sample_internal_voice_id_start
                                            {
                                                smoother.reset(target_plain_value);
                                            } else {
                                                smoother
                                                    .set_target(sample_rate, target_plain_value);
                                            }
                                        }
                                        n => nih_debug_assert_failure!(
                                            "Polyphonic modulation sent for unknown poly \
                                             modulation ID {}",
                                            n
                                        ),
                                    }
                                }
                            }
                            NoteEvent::MonoAutomation {
                                timing: _,
                                poly_modulation_id,
                                normalized_value,
                            } => {
                                // Modulation always acts as an offset to the parameter's current
                                // automated value. So if the host sends a new automation value for
                                // a modulated parameter, the modulated values/smoothing targets
                                // need to be updated for all polyphonically modulated voices.
                                for voice in self.voices.iter_mut().filter_map(|v| v.as_mut()) {
                                    match poly_modulation_id {
                                        GAIN_POLY_MOD_ID => {
                                            let (normalized_offset, smoother) =
                                                match voice.voice_gain.as_mut() {
                                                    Some((o, s)) => (o, s),
                                                    // If the voice does not have existing
                                                    // polyphonic modulation, then there's nothing
                                                    // to do here. The global automation/monophonic
                                                    // modulation has already been taken care of by
                                                    // the framework.
                                                    None => continue,
                                                };
                                            let target_plain_value =
                                                self.params.gain.preview_plain(
                                                    normalized_value + *normalized_offset,
                                                );
                                            smoother.set_target(sample_rate, target_plain_value);
                                        }
                                        n => nih_debug_assert_failure!(
                                            "Automation event sent for unknown poly modulation ID \
                                             {}",
                                            n
                                        ),
                                    }
                                }
                            }
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

            // These are the smoothed global parameter values. These are used for voices that do not
            // have polyphonic modulation applied to them. With a plugin as simple as this it would
            // be possible to avoid this completely by simply always copying the smoother into the
            // voice's struct, but that may not be realistic when the plugin has hundreds of
            // parameters. The `voice_*` arrays are scratch arrays that an individual voice can use.
            let block_len = block_end - block_start;
            let mut gain = [0.0; MAX_BLOCK_SIZE];
            let mut voice_gain = [0.0; MAX_BLOCK_SIZE];
            let mut voice_amp_envelope = [0.0; MAX_BLOCK_SIZE];
            self.params.gain.smoothed.next_block(&mut gain, block_len);

            // TODO: Some form of band limiting
            // TODO: Filter
            for voice in self.voices.iter_mut().filter_map(|v| v.as_mut()) {
                // Depending on whether the voice has polyphonic modulation applied to it,
                // either the global parameter values are used, or the voice's smoother is used
                // to generate unique modulated values for that voice
                let gain = match &voice.voice_gain {
                    Some((_, smoother)) => {
                        smoother.next_block(&mut voice_gain, block_len);
                        &voice_gain
                    }
                    None => &gain,
                };

                // This is an exponential smoother repurposed as an AR envelope with values between
                // 0 and 1. When a note off event is received, this envelope will start fading out
                // again. When it reaches 0, we will terminate the voice.
                voice
                    .amp_envelope
                    .next_block(&mut voice_amp_envelope, block_len);

                for (value_idx, sample_idx) in (block_start..block_end).enumerate() {
                    let amp = voice.velocity_sqrt * gain[value_idx] * voice_amp_envelope[value_idx];
                    let sample = (voice.phase * 2.0 - 1.0) * amp;

                    voice.phase += voice.phase_delta;
                    if voice.phase >= 1.0 {
                        voice.phase -= 1.0;
                    }

                    output[0][sample_idx] += sample;
                    output[1][sample_idx] += sample;
                }
            }

            // Terminate voices whose release period has fully ended. This could be done as part of
            // the previous loop but this is simpler.
            for voice in self.voices.iter_mut() {
                match voice {
                    Some(v) if v.releasing && v.amp_envelope.previous_value() == 0.0 => {
                        // This event is very important, as it allows the host to manage its own modulation
                        // voices
                        context.send_event(NoteEvent::VoiceTerminated {
                            timing: block_end as u32,
                            voice_id: Some(v.voice_id),
                            channel: v.channel,
                            note: v.note,
                        });
                        *voice = None;
                    }
                    _ => (),
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
    /// Get the index of a voice by its voice ID, if the voice exists. This does not immediately
    /// return a reference to the voice to avoid lifetime issues.
    fn get_voice_idx(&mut self, voice_id: i32) -> Option<usize> {
        self.voices
            .iter_mut()
            .position(|voice| matches!(voice, Some(voice) if voice.voice_id == voice_id))
    }

    /// Start a new voice with the given voice ID. If all voices are currently in use, the oldest
    /// voice will be stolen. Returns a reference to the new voice.
    fn start_voice(
        &mut self,
        context: &mut impl ProcessContext<Self>,
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
            releasing: false,
            amp_envelope: Smoother::none(),

            voice_gain: None,
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

    /// Start the release process for one or more voice by changing their amplitude envelope. If
    /// `voice_id` is not provided, then this will terminate all matching voices.
    fn start_release_for_voices(
        &mut self,
        sample_rate: f32,
        voice_id: Option<i32>,
        channel: u8,
        note: u8,
    ) {
        for voice in self.voices.iter_mut() {
            match voice {
                Some(Voice {
                    voice_id: candidate_voice_id,
                    channel: candidate_channel,
                    note: candidate_note,
                    releasing,
                    amp_envelope,
                    ..
                }) if voice_id == Some(*candidate_voice_id)
                    || (channel == *candidate_channel && note == *candidate_note) =>
                {
                    *releasing = true;
                    amp_envelope.style =
                        SmoothingStyle::Exponential(self.params.amp_release_ms.value());
                    amp_envelope.set_target(sample_rate, 0.0);

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

    /// Immediately terminate one or more voice, removing it from the pool and informing the host
    /// that the voice has ended. If `voice_id` is not provided, then this will terminate all
    /// matching voices.
    fn choke_voices(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        voice_id: Option<i32>,
        channel: u8,
        note: u8,
    ) {
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
                    context.send_event(NoteEvent::VoiceTerminated {
                        timing: sample_offset,
                        // Notice how we always send the terminated voice ID here
                        voice_id: Some(*candidate_voice_id),
                        channel,
                        note,
                    });
                    *voice = None;

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
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(PolyModSynth);
nih_export_vst3!(PolyModSynth);
