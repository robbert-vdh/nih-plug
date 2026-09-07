//! The [`Editor`] trait implementation for Vizia editors.

use baseview::{WindowHandle, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use nih_plug::debug::*;
use nih_plug::prelude::{Editor, GuiContext, ParentWindowHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use vizia::context::backend::TextConfig;
use vizia::prelude::*;

use crate::widgets::RawParamEvent;
use crate::{assets, widgets, ViziaState, ViziaTheming};

/// An [`Editor`] implementation that calls an vizia draw loop.
pub(crate) struct ViziaEditor {
    pub(crate) vizia_state: Arc<ViziaState>,
    /// The user's app function.
    pub(crate) app: Arc<dyn Fn(&mut Context, Arc<dyn GuiContext>) + 'static + Send + Sync>,
    /// What level of theming to apply. See [`ViziaEditorTheming`].
    pub(crate) theming: ViziaTheming,

    /// The scaling factor reported by the host, if any. On macOS this will never be set and we
    /// should use the system scaling factor instead.
    pub(crate) scaling_factor: AtomicCell<Option<f32>>,

    /// Whether to emit a parameters changed event during the next idle callback. This is set in the
    /// `parameter_values_changed()` implementation and it can be used by widgets to explicitly
    /// check for new parameter values. This is useful when the parameter value is (indirectly) used
    /// to compute a property in an event handler. Like when positioning an element based on the
    /// display value's width.
    pub(crate) emit_parameters_changed_event: Arc<AtomicBool>,

    /// A proxy into the running GUI's event loop, captured
    /// when the editor spawns. `Editor::set_size` (called from the host's
    /// thread) uses it to wake the GUI and apply a host-driven resize — the
    /// proxy queue is drained every frame, unlike the idle callback, which
    /// vizia_baseview only runs on input events.
    pub(crate) host_resize_proxy: Arc<std::sync::Mutex<Option<vizia::context::ContextProxy>>>,
}

impl Editor for ViziaEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let app = self.app.clone();
        let vizia_state = self.vizia_state.clone();
        let theming = self.theming;
        let host_resize_proxy = self.host_resize_proxy.clone();

        let (unscaled_width, unscaled_height) = vizia_state.inner_logical_size();
        let system_scaling_factor = self.scaling_factor.load();
        let user_scale_factor = vizia_state.user_scale_factor();

        let mut application = Application::new(move |cx| {
            // Set some default styles to match the iced integration
            if theming >= ViziaTheming::Custom {
                cx.set_default_font(&[assets::NOTO_SANS]);
                if let Err(err) = cx.add_stylesheet(include_style!("assets/theme.css")) {
                    nih_error!("Failed to load stylesheet: {err:?}")
                }

                // There doesn't seem to be any way to bundle styles with a widget, so we'll always
                // include the style sheet for our custom widgets at context creation
                widgets::register_theme(cx);
            }

            // Any widget can change the parameters by emitting `ParamEvent` events. This model will
            // handle them automatically.
            widgets::ParamModel {
                context: context.clone(),
            }
            .build(cx);

            // And we'll link `WindowEvent::ResizeWindow` and `WindowEvent::SetScale` events to our
            // `ViziaState`. We'll notify the host when any of these change.
            let current_inner_window_size = cx.window_size();
            widgets::WindowModel {
                context: context.clone(),
                vizia_state: vizia_state.clone(),
                last_inner_window_size: AtomicCell::new((
                    current_inner_window_size.width,
                    current_inner_window_size.height,
                )),
            }
            .build(cx);

            // Capture a proxy into this GUI's event loop so
            // host-driven resizes (arriving on the host's thread) can wake it.
            if let Ok(mut proxy) = host_resize_proxy.lock() {
                *proxy = Some(cx.get_proxy());
            }

            app(cx, context.clone())
        })
        .with_scale_policy(
            system_scaling_factor
                .map(|factor| WindowScalePolicy::ScaleFactor(factor as f64))
                .unwrap_or(WindowScalePolicy::SystemScaleFactor),
        )
        .inner_size((unscaled_width, unscaled_height))
        .user_scale_factor(user_scale_factor)
        .with_text_config(TextConfig {
            hint: false,
            subpixel: true,
        })
        .on_idle({
            let emit_parameters_changed_event = self.emit_parameters_changed_event.clone();
            let vizia_state = self.vizia_state.clone();
            move |cx| {
                if emit_parameters_changed_event
                    .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    cx.emit_custom(
                        Event::new(RawParamEvent::ParametersChanged)
                            .propagate(Propagation::Subtree),
                    );
                }

                // A host-driven resize updated the size
                // state from the host's thread; apply it to the embedded
                // window here on the GUI thread (one-shot, so a host that
                // rejects the follow-up request_resize can't ping-pong).
                if vizia_state
                    .deferred_resize
                    .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    let (width, height) = vizia_state.inner_logical_size();
                    // The host initiated this resize — skip the
                    // `request_resize` renegotiation that the following
                    // `GeometryChanged` would trigger (hosts that refuse
                    // plugin-initiated requests would revert it).
                    vizia_state
                        .suppress_resize_request
                        .store(true, Ordering::Release);
                    let mut event_cx = EventContext::new(cx);
                    event_cx.set_window_size(WindowSize { width, height });
                }
            }
        });

        // This way the plugin can decide to use none of the built in theming
        if theming == ViziaTheming::None {
            application = application.ignore_default_theme();
        }

        let window = application.open_parented(&parent);

        self.vizia_state.open.store(true, Ordering::Release);
        Box::new(ViziaEditorHandle {
            vizia_state: self.vizia_state.clone(),
            window,
        })
    }

    fn size(&self) -> (u32, u32) {
        // This includes the user scale factor if set, but not any HiDPI scaling
        self.vizia_state.scaled_logical_size()
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        // If the editor is currently open then the host must not change the current HiDPI scale as
        // we don't have a way to handle that. Ableton Live does this.
        if self.vizia_state.is_open() {
            return false;
        }

        // We're making things a bit more complicated by having both a system scale factor, which is
        // used for HiDPI and also known to the host, and a user scale factor that the user can use
        // to arbitrarily resize the GUI
        self.scaling_factor.store(Some(factor));
        true
    }

    fn set_size(&self, width: u32, height: u32) -> bool {
        // Host-driven resize. `width`/`height` arrive in the
        // same units `size()` reports (logical pixels including the user
        // scale factor). Convert to `size_fn` units, let the app's callback
        // update (and clamp) its size state, then wake the GUI's event loop
        // to apply the result (`ApplyHostResize` → `WindowModel`). The idle
        // flag stays as a fallback for runtimes without an event proxy.
        let Some(on_host_resize) = &self.vizia_state.on_host_resize else {
            return false;
        };
        let user_scale = self.vizia_state.user_scale_factor();
        let logical_width = (f64::from(width) / user_scale).round() as u32;
        let logical_height = (f64::from(height) / user_scale).round() as u32;
        on_host_resize(logical_width, logical_height);
        self.vizia_state
            .deferred_resize
            .store(true, Ordering::Release);
        if let Ok(mut proxy) = self.host_resize_proxy.lock() {
            if let Some(proxy) = proxy.as_mut() {
                let _ = proxy.emit(crate::widgets::ApplyHostResize);
            }
        }
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        // This will cause a future idle callback to send a parameters changed event.
        // NOTE: We could add an event containing the parameter's ID and the normalized value, but
        //       these events aren't really necessary for Vizia.
        self.emit_parameters_changed_event
            .store(true, Ordering::Relaxed);
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        self.emit_parameters_changed_event
            .store(true, Ordering::Relaxed);
    }

    fn param_values_changed(&self) {
        self.emit_parameters_changed_event
            .store(true, Ordering::Relaxed);
    }
}

/// The window handle used for [`ViziaEditor`].
struct ViziaEditorHandle {
    vizia_state: Arc<ViziaState>,
    window: WindowHandle,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl Send for ViziaEditorHandle {}

impl Drop for ViziaEditorHandle {
    fn drop(&mut self) {
        self.vizia_state.open.store(false, Ordering::Release);
        // XXX: This should automatically happen when the handle gets dropped, but apparently not
        self.window.close();
    }
}
