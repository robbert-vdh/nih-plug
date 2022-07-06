use nih_plug::prelude::*;
use std::sync::Arc;

/// A simple polyphonic synthesizer with support for CLAP's polyphonic modulation. See
/// `NoteEvent::PolyModulation` for another source of information on how to use this.
struct PolyModSynth {
    params: Arc<PolyModSynthParams>,
}

#[derive(Default, Params)]
struct PolyModSynthParams {}

impl Default for PolyModSynth {
    fn default() -> Self {
        Self {
            params: Arc::new(PolyModSynthParams::default()),
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
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;
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
        // TODO: Split blocks, so something cool
        while let Some(event) = context.next_event() {
            match event {
                _ => (),
            }
        }

        ProcessStatus::Normal
    }
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
}

// The VST3 verison of this plugin isn't too interesting as it will not support polyphonic
// modulation
impl Vst3Plugin for PolyModSynth {
    const VST3_CLASS_ID: [u8; 16] = *b"PolyM0dSynth1337";
    const VST3_CATEGORIES: &'static str = "Instrument|Synth";
}

nih_export_clap!(PolyModSynth);
nih_export_vst3!(PolyModSynth);
