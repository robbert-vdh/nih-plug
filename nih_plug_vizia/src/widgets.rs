//! Widgets and utilities for making widgets to integrate VIZIA with NIH-plug.
//!
//! # Note
//!
//! None of these widgets are finalized, and their sizes or looks can change at any point. Feel free
//! to copy the widgets and modify them to your personal taste.

use nih_plug::prelude::{GuiContext, Param, ParamPtr};
use std::sync::Arc;

use vizia::{Context, Model, WindowEvent};

use super::ViziaState;

mod generic_ui;
mod param_slider;
mod peak_meter;
pub mod util;

pub use generic_ui::GenericUi;
pub use param_slider::{ParamSlider, ParamSliderExt, ParamSliderStyle};
pub use peak_meter::PeakMeter;

/// Register the default theme for the widgets exported by this module. This is automatically called
/// for you when using [`create_vizia_editor()`][super::create_vizia_editor()].
pub fn register_theme(cx: &mut Context) {
    cx.add_theme(include_str!("../assets/widgets.css"));
}

/// An event that updates a parameter's value. Since NIH-plug manages the parameters, interacting
/// with parameter values with VIZIA works a little different from updating any other state. These
/// events are automatically handled by `nih_plug_vizia`.
///
/// Call the [`upcast()`][Self::upcast()] method to be able to emit this event through a
/// [`Context`][vizia::Context].
#[derive(Debug, Clone, Copy)]
pub enum ParamEvent<'a, P: Param> {
    /// Begin an automation gesture for a parameter.
    BeginSetParameter(&'a P),
    /// Set a parameter to a new normalized value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    SetParameter(&'a P, P::Plain),
    /// Set a parameter to a new normalized value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    SetParameterNormalized(&'a P, f32),
    /// End an automation gesture for a parameter.
    EndSetParameter(&'a P),
}

/// The same as [`ParamEvent`], but type erased.
#[derive(Debug, Clone, Copy)]
pub enum RawParamEvent {
    /// Begin an automation gesture for a parameter.
    BeginSetParameter(ParamPtr),
    /// Set a parameter to a new normalized value. This needs to be surrounded by a matching
    /// `BeginSetParameter` and `EndSetParameter`.
    SetParameterNormalized(ParamPtr, f32),
    /// End an automation gesture for a parameter.
    EndSetParameter(ParamPtr),
}

/// Handles parameter updates for VIZIA GUIs. Registered in
/// [`ViziaEditor::spawn()`][super::ViziaEditor::spawn()].
pub(crate) struct ParamModel {
    pub context: Arc<dyn GuiContext>,
}

/// Handles interactions through `WindowEvent` for VIZIA GUIs by updating the `ViziaState`.
/// Registered in [`ViziaEditor::spawn()`][super::ViziaEditor::spawn()].
pub(crate) struct WindowModel {
    pub context: Arc<dyn GuiContext>,
    pub vizia_state: Arc<ViziaState>,
}

impl Model for ParamModel {
    fn event(&mut self, _cx: &mut vizia::Context, event: &mut vizia::Event) {
        if let Some(param_event) = event.message.downcast() {
            // `ParamEvent` gets downcast into `NormalizedParamEvent` by the `Message`
            // implementation below
            match *param_event {
                RawParamEvent::BeginSetParameter(p) => unsafe {
                    self.context.raw_begin_set_parameter(p)
                },
                RawParamEvent::SetParameterNormalized(p, v) => unsafe {
                    self.context.raw_set_parameter_normalized(p, v)
                },
                RawParamEvent::EndSetParameter(p) => unsafe {
                    self.context.raw_end_set_parameter(p)
                },
            }
        }
    }
}

impl Model for WindowModel {
    fn event(&mut self, cx: &mut vizia::Context, event: &mut vizia::Event) {
        if let Some(window_event) = event.message.downcast() {
            match *window_event {
                WindowEvent::ResizeWindow(logical_width, logical_height) => {
                    let logical_size =
                        (logical_width.round() as u32, logical_height.round() as u32);
                    let old_size @ (old_logical_width, old_logical_height) =
                        self.vizia_state.size.load();

                    // Don't do anything if the current size already matches the new size, this
                    // could otherwise also cause a feedback loop on resize failure
                    if logical_size == old_size {
                        return;
                    }

                    // Our embedded baseview window will have already been resized. If the host does
                    // not accept our new size, then we'll try to undo that
                    self.vizia_state.size.store(logical_size);
                    if !self.context.request_resize() {
                        self.vizia_state.size.store(old_size);
                        cx.emit(WindowEvent::ResizeWindow(
                            old_logical_width as f32,
                            old_logical_height as f32,
                        ));
                    }
                }
                WindowEvent::SetScale(user_scale_factor) => {
                    let old_user_scale_factor = self.vizia_state.scale_factor.load();

                    // Don't do anything if the current scale already matches the new scale
                    if user_scale_factor == old_user_scale_factor {
                        return;
                    }

                    // This works the same as the `ResizeWindow` handler. The actual window size
                    // reported to the host gets calculated from a combination of the window's
                    // logical size (before user scaling) and the user scale factor.
                    self.vizia_state.scale_factor.store(user_scale_factor);
                    if !self.context.request_resize() {
                        self.vizia_state.scale_factor.store(old_user_scale_factor);
                        cx.emit(WindowEvent::SetScale(old_user_scale_factor));
                    }
                }

                _ => (),
            }
        }
    }
}
impl<P: Param> From<ParamEvent<'_, P>> for RawParamEvent {
    fn from(event: ParamEvent<'_, P>) -> Self {
        match event {
            ParamEvent::BeginSetParameter(p) => RawParamEvent::BeginSetParameter(p.as_ptr()),
            ParamEvent::SetParameter(p, v) => {
                RawParamEvent::SetParameterNormalized(p.as_ptr(), p.preview_normalized(v))
            }
            ParamEvent::SetParameterNormalized(p, v) => {
                RawParamEvent::SetParameterNormalized(p.as_ptr(), v)
            }
            ParamEvent::EndSetParameter(p) => RawParamEvent::EndSetParameter(p.as_ptr()),
        }
    }
}

impl<P: Param> ParamEvent<'_, P> {
    /// Convert this event into a type erased version of itself that can be emitted through
    /// [`Context::emit()`][vizia::Context::emit()].
    ///
    /// TODO: Think of a better, clearer term for this
    pub fn upcast(self) -> RawParamEvent {
        self.into()
    }
}
