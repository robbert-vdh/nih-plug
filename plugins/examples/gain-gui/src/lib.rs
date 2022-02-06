// nih-plug: plugins, but rewritten in Rust
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

#[macro_use]
extern crate nih_plug;

use atomic_float::AtomicF32;
use nih_plug::{
    formatters, util, Buffer, BufferConfig, BusConfig, Editor, EditorWindowHandle, GuiContext,
    ParamSetter, Plugin, ProcessContext, ProcessStatus, Vst3Plugin,
};
use nih_plug::{FloatParam, Param, Params, Range, Smoother, SmoothingStyle};
use nih_plug_egui::{create_egui_editor, egui, AtomicCell};
use std::pin::Pin;
use std::sync::Arc;

/// This is mostly identical to the gain example, minus some fluff, and with a GUI.
struct Gain {
    params: Pin<Arc<GainParams>>,
    editor_size: Arc<AtomicCell<(u32, u32)>>,

    /// Needed to normalize the VU meter's response based on the sample rate.
    vu_meter_decay_weight: f32,
    /// The current data for the VU meter (which technically isn't a VU meter). This is stored as an
    /// [Arc] so we can share it between the GUI and the audio processing parts. If you have more
    /// state to share, then it's a good idea to put all of that in a struct behind a single `Arc`.
    ///
    /// This is stored as voltage gain.
    vu_meter: Arc<AtomicF32>,
}

#[derive(Params)]
struct GainParams {
    #[id = "gain"]
    pub gain: FloatParam,
}

impl Default for Gain {
    fn default() -> Self {
        Self {
            params: Arc::pin(GainParams::default()),
            editor_size: Arc::new(AtomicCell::new((300, 100))),

            vu_meter_decay_weight: 1.0,
            vu_meter: Arc::new(AtomicF32::new(util::MINUS_INFINITY_DB)),
        }
    }
}

impl Default for GainParams {
    fn default() -> Self {
        Self {
            gain: FloatParam {
                value: 0.0,
                smoothed: Smoother::new(SmoothingStyle::Linear(50.0)),
                value_changed: None,
                range: Range::Linear {
                    min: -30.0,
                    max: 30.0,
                },
                name: "Gain",
                unit: " dB",
                value_to_string: formatters::f32_rounded(2),
                string_to_value: None,
            },
        }
    }
}

impl Plugin for Gain {
    const NAME: &'static str = "Gain GUI";
    const VENDOR: &'static str = "Moist Plugins GmbH";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "info@example.com";

    const VERSION: &'static str = "0.0.1";

    const DEFAULT_NUM_INPUTS: u32 = 2;
    const DEFAULT_NUM_OUTPUTS: u32 = 2;

    const ACCEPTS_MIDI: bool = false;

    fn params(&self) -> Pin<&dyn Params> {
        self.params.as_ref()
    }

    fn create_editor(
        &self,
        parent: EditorWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let vu_meter = self.vu_meter.clone();
        create_egui_editor(
            parent,
            context,
            self.editor_size.clone(),
            (),
            move |egui_ctx, nih_ctx, _state| {
                let setter = ParamSetter::new(nih_ctx);

                egui::CentralPanel::default().show(egui_ctx, |ui| {
                    ui.allocate_space(egui::Vec2::splat(3.0));
                    ui.label("Gain");

                    // TODO: Create a custom widget that can do all of the parameter handling and
                    //       works with nonlinear ranges
                    ui.add(
                        egui::widgets::Slider::from_get_set(-30.0..=30.0, |new_value| {
                            match new_value {
                                Some(new_value) => {
                                    // TODO: Gestures?
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
                    let vu_meter =
                        util::gain_to_db(vu_meter.load(std::sync::atomic::Ordering::Relaxed));
                    let vu_meter_text = if vu_meter > util::MINUS_INFINITY_DB {
                        format!("{:.0} dBFS", vu_meter)
                    } else {
                        String::from("-inf dBFS")
                    };

                    let vu_meter_normalized =
                        (vu_meter - util::MINUS_INFINITY_DB) / -util::MINUS_INFINITY_DB;
                    ui.allocate_space(egui::Vec2::splat(2.0));
                    ui.add(
                        egui::widgets::ProgressBar::new(vu_meter_normalized).text(vu_meter_text),
                    );
                });
            },
        )
    }

    fn editor_size(&self) -> Option<(u32, u32)> {
        Some(self.editor_size.load())
    }

    fn accepts_bus_config(&self, config: &BusConfig) -> bool {
        // This works with any symmetrical IO layout
        config.num_input_channels == config.num_output_channels && config.num_input_channels > 0
    }

    fn initialize(
        &mut self,
        _bus_config: &BusConfig,
        buffer_config: &BufferConfig,
        _context: &mut impl ProcessContext,
    ) -> bool {
        // TODO: How do you tie this exponential decay to an actual time span?
        self.vu_meter_decay_weight = 0.999f32.powf(44_100.0 / buffer_config.sample_rate);

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _context: &mut impl ProcessContext,
    ) -> ProcessStatus {
        for samples in buffer.iter_mut() {
            let gain = self.params.gain.smoothed.next();

            let mut amplitude = 0.0;
            let num_samples = samples.len();
            for sample in samples {
                *sample *= util::db_to_gain(gain);
                amplitude += *sample;
            }

            // This is technically not really a VU meter but just an arbitrary digital peak meter
            amplitude /= num_samples as f32;
            let current_vu_meter = self.vu_meter.load(std::sync::atomic::Ordering::Relaxed);
            let new_vu_meter = if amplitude > current_vu_meter {
                amplitude
            } else {
                current_vu_meter * self.vu_meter_decay_weight
                    + amplitude * (1.0 - self.vu_meter_decay_weight)
            };

            self.vu_meter
                .store(new_vu_meter, std::sync::atomic::Ordering::Relaxed)
        }

        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Gain {
    const VST3_CLASS_ID: [u8; 16] = *b"GainGuiYeahBoyyy";
    const VST3_CATEGORIES: &'static str = "Fx|Dynamics";
}

nih_export_vst3!(Gain);
