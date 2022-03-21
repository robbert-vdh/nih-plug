use nih_plug::prelude::*;
use parking_lot::RwLock;
use std::pin::Pin;
use std::sync::Arc;

struct Gain {
    params: Pin<Box<GainParams>>,
}

#[derive(Params)]
struct GainParams {
    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "stable"]
    pub but_field_names_can_change: BoolParam,

    /// This field isn't used in this exampleq, but anything written to the vector would be restored
    /// together with a preset/state file saved for this plugin. This can be useful for storign
    /// things like sample data.
    #[persist = "industry_secrets"]
    pub random_data: RwLock<Vec<f32>>,

    /// You can also nest parameter structs. These will appear as a separate nested group if your
    /// DAW displays parameters in a tree structure.
    #[nested = "Subparameters"]
    pub sub_params: SubParams,
}

#[derive(Params)]
struct SubParams {
    #[id = "thing"]
    pub nested_parameter: FloatParam,

    #[nested = "Sub-Subparameters"]
    pub sub_sub_params: SubSubParams,
}

#[derive(Params)]
struct SubSubParams {
    #[id = "noope"]
    pub nope: FloatParam,
}

impl Default for Gain {
    fn default() -> Self {
        Self {
            params: Box::pin(GainParams::default()),
        }
    }
}

impl Default for GainParams {
    fn default() -> Self {
        Self {
            // There are three ways to specify parameters:
            //
            // ...either manually specify all fields:
            gain: FloatParam {
                value: 0.0,
                default: 0.0,
                smoothed: Smoother::new(SmoothingStyle::Linear(50.0)),
                value_changed: None,
                range: FloatRange::Linear {
                    min: -30.0,
                    max: 30.0,
                },
                step_size: Some(0.01),
                name: "Gain",
                unit: " dB",
                // This is actually redundant, because a step size of two decimal places already
                // causes the parameter to shown rounded
                value_to_string: Some(formatters::f32_rounded(2)),
                string_to_value: None,
                // ...or specify the fields you want to initialize directly and leave the other
                // fields at their defaults:
                // // ..Default::default(),
            },
            // ...or use the builder interface:
            but_field_names_can_change: BoolParam::new("Important value", false).with_callback(
                Arc::new(|_new_value: bool| {
                    // If, for instance, updating this parameter would require other parts of the
                    // plugin's internal state to be updated other values to also be updated, then
                    // you can use this callback to for instance modify an atomic in the plugin.
                }),
            ),
            // Persisted fields can be intialized like any other fields, and they'll keep their when
            // restoring the plugin's state.
            random_data: RwLock::new(Vec::new()),
            sub_params: SubParams {
                nested_parameter: FloatParam::new(
                    "Unused Nested Parameter",
                    0.5,
                    FloatRange::Skewed {
                        min: 2.0,
                        max: 2.4,
                        factor: FloatRange::skew_factor(2.0),
                    },
                )
                .with_value_to_string(formatters::f32_rounded(2)),
                sub_sub_params: SubSubParams {
                    nope: FloatParam::new("Nope", 0.5, FloatRange::Linear { min: 1.0, max: 2.0 }),
                },
            },
        }
    }
}

impl Plugin for Gain {
    const NAME: &'static str = "Gain";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const ACCEPTS_MIDI: bool = false;
    // Setting this to `true` will tell the wrapper to split the buffer up into smaller blocks
    // whenever there are inter-buffer parameter changes. This way no changes to the plugin are
    // required to support sample accurate automation and the wrapper handles all of the boring
    // stuff like making sure transport and other timing information stays consistent between the
    // splits.
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This works with any symmetrical IO layout
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    // This plugin doesn't need any special initialization, but if you need to do anything expensive
    // then this would be the place. State is kept around while when the host reconfigures the
    // plugin. If we did need special initialization, we could implement the `initialize()` and/or
    // `reset()` methods

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_samples() {
            // Smoothing is optionally built into the parameters themselves
            let gain = self.params.gain.smoothed.next();

            for sample in channel_samples {
                *sample *= util::db_to_gain(gain);
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Gain {
    const CLAP_ID: &'static str = "com.moist-plugins-gmbh.gain";
    const CLAP_DESCRIPTION: &'static str = "A smoothed gain parameter example plugin";
    const CLAP_FEATURES: &'static [&'static str] = &["audio_effect", "mono", "stereo", "tool"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Gain {
    const VST3_CLASS_ID: [u8; 16] = *b"GainMoistestPlug";
    const VST3_CATEGORIES: &'static str = "Fx|Dynamics";
}

nih_export_clap!(Gain);
nih_export_vst3!(Gain);
