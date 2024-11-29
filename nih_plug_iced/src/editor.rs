//! And [`Editor`] implementation for iced.

use ::baseview::{WindowOpenOptions, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use crossbeam::channel;
use iced_baseview::settings::IcedBaseviewSettings;
use nih_plug::prelude::{Editor, GuiContext, ParentWindowHandle};
use std::sync::Arc;
use std::{borrow::Cow, sync::atomic::Ordering};

use crate::{wrapper, IcedEditor, IcedState, ParameterUpdate};

pub use iced_baseview::*;

/// An [`Editor`] implementation that renders an iced [`Application`].
pub(crate) struct IcedEditorWrapper<E: IcedEditor> {
    pub(crate) iced_state: Arc<IcedState>,
    pub(crate) initialization_flags: E::InitializationFlags,
    pub(crate) fonts: Vec<Cow<'static, [u8]>>,

    /// The scaling factor reported by the host, if any. On macOS this will never be set and we
    /// should use the system scaling factor instead.
    pub(crate) scaling_factor: AtomicCell<Option<f32>>,

    /// A subscription for sending messages about parameter updates to the `IcedEditor`.
    pub(crate) parameter_updates_sender: channel::Sender<ParameterUpdate>,
    pub(crate) parameter_updates_receiver: Arc<channel::Receiver<ParameterUpdate>>,
}

impl<E: IcedEditor> Editor for IcedEditorWrapper<E> {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let (unscaled_width, unscaled_height) = self.iced_state.size();
        let scaling_factor = self.scaling_factor.load();

        // TODO: iced_baseview does not have gracefuly error handling for context creation failures.
        //       This will panic if the context could not be created.
        let window = iced_baseview::open_parented::<wrapper::IcedEditorWrapperApplication<E>, _>(
            &parent,
            // We use this wrapper to be able to pass the GUI context to the editor
            (
                context,
                self.parameter_updates_receiver.clone(),
                self.initialization_flags.clone(),
            ),
            Settings {
                window: WindowOpenOptions {
                    title: String::from("iced window"),
                    // Baseview should be doing the DPI scaling for us
                    size: baseview::Size::new(unscaled_width as f64, unscaled_height as f64),
                    // NOTE: For some reason passing 1.0 here causes the UI to be scaled on macOS but
                    //       not the mouse events.
                    scale: scaling_factor
                        .map(|factor| WindowScalePolicy::ScaleFactor(factor as f64))
                        .unwrap_or(WindowScalePolicy::SystemScaleFactor),
                },
                iced_baseview: IcedBaseviewSettings {
                    ignore_non_modifier_keys: false,
                    always_redraw: true,
                },
                fonts: self.fonts.clone(),
                ..Default::default()
            },
        );

        self.iced_state.open.store(true, Ordering::Release);
        Box::new(IcedEditorHandle {
            iced_state: self.iced_state.clone(),
            window,
        })
    }

    fn size(&self) -> (u32, u32) {
        self.iced_state.size()
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        // If the editor is currently open then the host must not change the current HiDPI scale as
        // we don't have a way to handle that. Ableton Live does this.
        if self.iced_state.is_open() {
            return false;
        }

        self.scaling_factor.store(Some(factor));
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        // If there's already a paramter change notification in the channel then we don't need
        // to do anything else. This avoids queueing up redundant GUI redraws.
        // NOTE: We could add an event containing the parameter's ID and the normalized value, but
        //       these events aren't really necessary for Vizia.
        let _ = self.parameter_updates_sender.try_send(ParameterUpdate);
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        let _ = self.parameter_updates_sender.try_send(ParameterUpdate);
    }

    fn param_values_changed(&self) {
        let _ = self.parameter_updates_sender.try_send(ParameterUpdate);
    }
}

/// The window handle used for [`IcedEditorWrapper`].
struct IcedEditorHandle<Message: 'static + Send> {
    iced_state: Arc<IcedState>,
    window: iced_baseview::window::WindowHandle<Message>,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl<Message: Send> Send for IcedEditorHandle<Message> {}

impl<Message: Send> Drop for IcedEditorHandle<Message> {
    fn drop(&mut self) {
        self.iced_state.open.store(false, Ordering::Release);
        self.window.close_window();
    }
}
