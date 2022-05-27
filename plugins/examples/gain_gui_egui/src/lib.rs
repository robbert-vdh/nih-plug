use atomic_float::AtomicF32;
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets, EguiState};
use std::sync::Arc;

/// This is mostly identical to the gain example, minus some fluff, and with a GUI.
struct Gain {
    params: Arc<GainParams>,
    editor_state: Arc<EguiState>,

    /// Needed to normalize the peak meter's response based on the sample rate.
    peak_meter_decay_weight: f32,
    /// The current data for the peak meter. This is stored as an [`Arc`] so we can share it between
    /// the GUI and the audio processing parts. If you have more state to share, then it's a good
    /// idea to put all of that in a struct behind a single `Arc`.
    ///
    /// This is stored as voltage gain.
    peak_meter: Arc<AtomicF32>,
}

#[derive(Params)]
struct GainParams {
    #[id = "gain"]
    pub gain: FloatParam,

    // TODO: Remove this parameter when we're done implementing the widgets
    #[id = "foobar"]
    pub some_int: IntParam,
}

impl Default for Gain {
    fn default() -> Self {
        Self {
            params: Arc::new(GainParams::default()),
            editor_state: EguiState::from_size(300, 180),

            peak_meter_decay_weight: 1.0,
            peak_meter: Arc::new(AtomicF32::new(util::MINUS_INFINITY_DB)),
        }
    }
}

impl Default for GainParams {
    fn default() -> Self {
        Self {
            gain: FloatParam::new(
                "Gain",
                0.0,
                FloatRange::Linear {
                    min: -30.0,
                    max: 30.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_step_size(0.01)
            .with_unit(" dB"),
            some_int: IntParam::new("Something", 3, IntRange::Linear { min: 0, max: 3 }),
        }
    }
}

impl Plugin for Gain {
    const NAME: &'static str = "Gain GUI (egui)";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&self) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let peak_meter = self.peak_meter.clone();
        create_egui_editor(
            self.editor_state.clone(),
            (),
            move |egui_ctx, setter, _state| {
                egui::CentralPanel::default().show(egui_ctx, |ui| {
                    // NOTE: See `plugins/diopser/src/editor.rs` for an example using the generic UI widget

                    // This is a fancy widget that can get all the information it needs to properly
                    // display and modify the parameter from the parametr itself
                    // It's not yet fully implemented, as the text is missing.
                    ui.label("Some random integer");
                    ui.add(widgets::ParamSlider::for_param(&params.some_int, setter));

                    ui.label("Gain");
                    ui.add(widgets::ParamSlider::for_param(&params.gain, setter));

                    ui.label(
                        "Also gain, but with a lame widget. Can't even render the value correctly!",
                    );
                    // This is a simple naieve version of a parameter slider that's not aware of how
                    // the parameters work
                    ui.add(
                        egui::widgets::Slider::from_get_set(-30.0..=30.0, |new_value| {
                            match new_value {
                                Some(new_value) => {
                                    setter.begin_set_parameter(&params.gain);
                                    setter.set_parameter(&params.gain, new_value as f32);
                                    setter.end_set_parameter(&params.gain);
                                    new_value
                                }
                                None => params.gain.value as f64,
                            }
                        })
                        .suffix(" dB"),
                    );

                    // TODO: Add a proper custom widget instead of reusing a progress bar
                    let peak_meter =
                        util::gain_to_db(peak_meter.load(std::sync::atomic::Ordering::Relaxed));
                    let peak_meter_text = if peak_meter > util::MINUS_INFINITY_DB {
                        format!("{:.1} dBFS", peak_meter)
                    } else {
                        String::from("-inf dBFS")
                    };

                    let peak_meter_normalized = (peak_meter + 60.0) / 60.0;
                    ui.allocate_space(egui::Vec2::splat(2.0));
                    ui.add(
                        egui::widgets::ProgressBar::new(peak_meter_normalized)
                            .text(peak_meter_text),
                    );
                });
            },
        )
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This works with any symmetrical IO layout
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext,
    ) -> bool {
        // TODO: How do you tie this exponential decay to an actual time span?
        self.peak_meter_decay_weight = 0.9992f32.powf(44_100.0 / buffer_config.sample_rate);

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_samples() {
            let mut amplitude = 0.0;
            let num_samples = channel_samples.len();

            let gain = self.params.gain.smoothed.next();
            for sample in channel_samples {
                *sample *= util::db_to_gain(gain);
                amplitude += *sample;
            }

            // To save resources, a plugin can (and probably should!) only perform expensive
            // calculations that are only displayed on the GUI while the GUI is open
            if self.editor_state.is_open() {
                amplitude = (amplitude / num_samples as f32).abs();
                let current_peak_meter = self.peak_meter.load(std::sync::atomic::Ordering::Relaxed);
                let new_peak_meter = if amplitude > current_peak_meter {
                    amplitude
                } else {
                    current_peak_meter * self.peak_meter_decay_weight
                        + amplitude * (1.0 - self.peak_meter_decay_weight)
                };

                self.peak_meter
                    .store(new_peak_meter, std::sync::atomic::Ordering::Relaxed)
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Gain {
    const CLAP_ID: &'static str = "com.moist-plugins-gmbh-egui.gain-gui";
    const CLAP_DESCRIPTION: &'static str = "A smoothed gain parameter example plugin";
    const CLAP_FEATURES: &'static [&'static str] = &["audio_effect", "mono", "stereo", "tool"];
    const CLAP_MANUAL_URL: &'static str = Self::URL;
    const CLAP_SUPPORT_URL: &'static str = Self::URL;
}

impl Vst3Plugin for Gain {
    const VST3_CLASS_ID: [u8; 16] = *b"GainGuiYeahBoyyy";
    const VST3_CATEGORIES: &'static str = "Fx|Dynamics";
}

nih_export_clap!(Gain);
nih_export_vst3!(Gain);
