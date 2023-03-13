//! Features a plugin supports. This is essentially the same thing as tags, keyword, or categories.
//! Hosts may use these to organize plugins.

/// A keyword for a CLAP plugin. See
/// <https://github.com/free-audio/clap/blob/main/include/clap/plugin-features.h> for more
/// information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClapFeature {
    // These are the main categories, every plugin should have at least one of these
    Instrument,
    AudioEffect,
    NoteDetector,
    NoteEffect,
    // These are optional
    Analyzer,
    Synthesizer,
    Sampler,
    Drum,
    DrumMachine,
    Filter,
    Phaser,
    Equalizer,
    Deesser,
    PhaseVocoder,
    Granular,
    FrequencyShifter,
    PitchShifter,
    Distortion,
    TransientShaper,
    Compressor,
    Expander,
    Gate,
    Limiter,
    Flanger,
    Chorus,
    Delay,
    Reverb,
    Tremolo,
    Glitch,
    Utility,
    PitchCorrection,
    Restoration,
    MultiEffects,
    Mixing,
    Mastering,
    Mono,
    Stereo,
    Surround,
    Ambisonic,
    /// A non-predefined feature. Hosts may display this among its plugin categories. Custom
    /// features _must_ be prefixed by a namespace in the format `namespace:feature_name`.
    Custom(&'static str),
}

impl ClapFeature {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClapFeature::Instrument => "instrument",
            ClapFeature::AudioEffect => "audio-effect",
            ClapFeature::NoteDetector => "note-detector",
            ClapFeature::NoteEffect => "note-effect",
            ClapFeature::Analyzer => "analyzer",
            ClapFeature::Synthesizer => "synthesizer",
            ClapFeature::Sampler => "sampler",
            ClapFeature::Drum => "drum",
            ClapFeature::DrumMachine => "drum-machine",
            ClapFeature::Filter => "filter",
            ClapFeature::Phaser => "phaser",
            ClapFeature::Equalizer => "equalizer",
            ClapFeature::Deesser => "de-esser",
            ClapFeature::PhaseVocoder => "phase-vocoder",
            ClapFeature::Granular => "granular",
            ClapFeature::FrequencyShifter => "frequency-shifter",
            ClapFeature::PitchShifter => "pitch-shifter",
            ClapFeature::Distortion => "distortion",
            ClapFeature::TransientShaper => "transient-shaper",
            ClapFeature::Compressor => "compressor",
            ClapFeature::Expander => "expander",
            ClapFeature::Gate => "gate",
            ClapFeature::Limiter => "limiter",
            ClapFeature::Flanger => "flanger",
            ClapFeature::Chorus => "chorus",
            ClapFeature::Delay => "delay",
            ClapFeature::Reverb => "reverb",
            ClapFeature::Tremolo => "tremolo",
            ClapFeature::Glitch => "glitch",
            ClapFeature::Utility => "utility",
            ClapFeature::PitchCorrection => "pitch-correction",
            ClapFeature::Restoration => "restoration",
            ClapFeature::MultiEffects => "multi-effects",
            ClapFeature::Mixing => "mixing",
            ClapFeature::Mastering => "mastering",
            ClapFeature::Mono => "mono",
            ClapFeature::Stereo => "stereo",
            ClapFeature::Surround => "surround",
            ClapFeature::Ambisonic => "ambisonic",
            ClapFeature::Custom(s) => {
                // Custom features must be prefixed with a namespace. We'll use `.split(':').all()`
                // here instead of `.split_once()` in case the user for whatever reason uses more
                // than one colon (which the docs don't say anything about, but uh yeah).
                nih_debug_assert!(
                    s.contains(':') && s.split(':').all(|x| !x.is_empty()),
                    "'{s}' is not a valid feature, custom features must be namespaced (e.g. \
                     'nih:{s}')",
                    s = s
                );

                s
            }
        }
    }
}
