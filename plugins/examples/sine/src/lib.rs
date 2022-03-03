#[macro_use]
extern crate nih_plug;

use nih_plug::{
    formatters, util, Buffer, BufferConfig, BusConfig, ClapPlugin, Plugin, ProcessContext,
    ProcessStatus, Vst3Plugin,
};
use nih_plug::{BoolParam, FloatParam, FloatRange, Params, Smoother, SmoothingStyle};
use std::f32::consts;
use std::pin::Pin;

/// A test tone generator that can either generate a sine wave based on the plugin's parameters or
/// based on the current MIDI input.
struct Sine {
    params: Pin<Box<SineParams>>,
    sample_rate: f32,

    /// The current phase of the sine wave, always kept between in `[0, 1]`.
    phase: f32,

    /// The frequency if the active note, if triggered by MIDI.
    midi_note_freq: f32,
    /// A simple attack and release envelope to avoid clicks.
    ///
    /// Smoothing is built into the parameters, but you can also use them manually if you need to
    /// smooth soemthing that isn't a parameter.
    midi_note_gain: Smoother<f32>,
}

#[derive(Params)]
struct SineParams {
    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "freq"]
    pub frequency: FloatParam,

    #[id = "usemid"]
    pub use_midi: BoolParam,
}

impl Default for Sine {
    fn default() -> Self {
        Self {
            params: Box::pin(SineParams::default()),
            sample_rate: 1.0,

            phase: 0.0,
            midi_note_freq: 1.0,
            midi_note_gain: Smoother::new(SmoothingStyle::Linear(5.0)),
        }
    }
}

impl Default for SineParams {
    fn default() -> Self {
        Self {
            gain: FloatParam::new(
                "Gain",
                -10.0,
                FloatRange::Linear {
                    min: -30.0,
                    max: 0.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(3.0))
            .with_step_size(0.01)
            .with_unit(" dB"),
            frequency: FloatParam::new(
                "Frequency",
                420.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 20_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" Hz")
            // We purposely don't specify a step size here, but the parameter should still be
            // displaeyd as rounded
            .with_value_to_string(formatters::f32_rounded(0)),
            use_midi: BoolParam::new("Use MIDI", false),
        }
    }
}

impl Sine {
    fn calculate_sine(&mut self, frequency: f32) -> f32 {
        let phase_delta = frequency / self.sample_rate;
        let sine = (self.phase * consts::TAU).sin();

        self.phase += phase_delta;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        sine
    }
}

impl Plugin for Sine {
    const NAME: &'static str = "Sine Test Tone";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 0;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const ACCEPTS_MIDI: bool = true;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This can output to any number of channels, but it doesn't take any audio inputs
        config.num_input_channels == 0 && config.num_input_channels > 0
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl ProcessContext,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;

        true
    }

    fn process(&mut self, buffer: &mut Buffer, context: &mut impl ProcessContext) -> ProcessStatus {
        let mut next_event = context.next_midi_event();
        for (sample_id, samples) in buffer.iter_mut().enumerate() {
            // Smoothing is optionally built into the parameters themselves
            let gain = self.params.gain.smoothed.next();

            // This plugin can be either triggered by MIDI or controleld by a parameter
            let sine = if self.params.use_midi.value {
                // Act on the next MIDI event
                'midi_events: loop {
                    match next_event {
                        Some(event) if event.timing() == sample_id as u32 => match event {
                            nih_plug::NoteEvent::NoteOn { note, .. } => {
                                self.midi_note_freq = util::midi_note_to_freq(note);
                                self.midi_note_gain.set_target(self.sample_rate, 1.0);
                            }
                            nih_plug::NoteEvent::NoteOff { note, .. } => {
                                if self.midi_note_freq == util::midi_note_to_freq(note) {
                                    self.midi_note_gain.set_target(self.sample_rate, 0.0);
                                }
                            }
                        },
                        _ => break 'midi_events,
                    }

                    next_event = context.next_midi_event();
                }

                // This gain envelope prevents clicks with new notes and with released notes
                self.calculate_sine(self.midi_note_freq) * self.midi_note_gain.next()
            } else {
                let frequency = self.params.frequency.smoothed.next();
                self.calculate_sine(frequency)
            };

            for sample in samples {
                *sample = sine * util::db_to_gain(gain);
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Sine {
    const CLAP_ID: &'static str = "com.moist-plugins-gmbh.sine";
    const CLAP_DESCRIPTION: &'static str = "An optionally MIDI controlled sine test tone";
    const CLAP_FEATURES: &'static [&'static str] = &["instrument", "mono", "stereo", "utility"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Sine {
    const VST3_CLASS_ID: [u8; 16] = *b"SineMoistestPlug";
    const VST3_CATEGORIES: &'static str = "Instrument|Synth|Tools";
}

nih_export_clap!(Sine);
nih_export_vst3!(Sine);
