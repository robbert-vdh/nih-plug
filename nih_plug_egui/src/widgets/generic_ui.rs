//! A simple generic UI widget that renders all parameters in a [`Params`] object as a scrollable
//! list of sliders and labels.

use std::sync::Arc;

use egui_baseview::egui::{self, TextStyle, Ui, Vec2};
use nih_plug::prelude::{Param, ParamFlags, ParamPtr, ParamSetter, Params};

use super::ParamSlider;

/// A widget that can be used to create a generic UI with. This is used in conjuction with empty
/// structs to emulate existential types.
pub trait ParamWidget {
    fn add_widget<P: Param>(&self, ui: &mut Ui, param: &P, setter: &ParamSetter);

    /// The same as [`add_widget()`][Self::add_widget()], but for a `ParamPtr`.
    ///
    /// # Safety
    ///
    /// Undefined behavior of the `ParamPtr` does not point to a valid parameter.
    unsafe fn add_widget_raw(&self, ui: &mut Ui, param: &ParamPtr, setter: &ParamSetter) {
        match param {
            ParamPtr::FloatParam(p) => self.add_widget(ui, &**p, setter),
            ParamPtr::IntParam(p) => self.add_widget(ui, &**p, setter),
            ParamPtr::BoolParam(p) => self.add_widget(ui, &**p, setter),
            ParamPtr::EnumParam(p) => self.add_widget(ui, &**p, setter),
        }
    }
}

/// Create a generic UI using [`ParamSlider`]s.
pub struct GenericSlider;

/// Create a scrollable generic UI using the specified widget. Takes up all the remaining vertical
/// space.
pub fn create(
    ui: &mut Ui,
    params: Arc<impl Params>,
    setter: &ParamSetter,
    widget: impl ParamWidget,
) {
    let padding = Vec2::splat(ui.text_style_height(&TextStyle::Body) * 0.2);
    egui::containers::ScrollArea::vertical()
        // Take up all remaining space, use a wrapper container to adjust how much space that is
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut first_widget = true;
            for (_, param_ptr, _) in params.param_map().into_iter() {
                let flags = unsafe { param_ptr.flags() };
                if flags.contains(ParamFlags::HIDE_IN_GENERIC_UI) {
                    continue;
                }

                // This list looks weird without a little padding
                if !first_widget {
                    ui.allocate_space(padding);
                }

                ui.label(unsafe { param_ptr.name() });
                unsafe { widget.add_widget_raw(ui, &param_ptr, setter) };

                first_widget = false;
            }
        });
}

impl ParamWidget for GenericSlider {
    fn add_widget<P: Param>(&self, ui: &mut Ui, param: &P, setter: &ParamSetter) {
        // Make these sliders a bit wider, else they look a bit odd
        ui.add(ParamSlider::for_param(param, setter).with_width(100.0));
    }
}
