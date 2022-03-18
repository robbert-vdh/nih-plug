//! [VIZIA](https://github.com/vizia/vizia) editor support for NIH plug.

use baseview::{WindowHandle, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use nih_plug::prelude::{Editor, GuiContext, ParamSetter, ParentWindowHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use vizia::{Application, Color, Context, Entity, Model, PropSet, WindowDescription};

// Re-export for convenience
pub use vizia;

pub mod assets;
pub mod widgets;

/// Create an [`Editor`] instance using a [`vizia`][::vizia] GUI. The [`ViziaState`] passed to this
/// function contains the GUI's intitial size, and this is kept in sync whenever the GUI gets
/// resized. You can also use this to know if the GUI is open, so you can avoid performing
/// potentially expensive calculations while the GUI is not open. If you want this size to be
/// persisted when restoring a plugin instance, then you can store it in a `#[persist = "key"]`
/// field on your parameters struct.
///
/// See [VIZIA](https://github.com/vizia/vizia)'s repository for examples on how to use this.
pub fn create_vizia_editor<F>(vizia_state: Arc<ViziaState>, app: F) -> Option<Box<dyn Editor>>
where
    F: Fn(&mut Context, &ParamSetter) + 'static + Send + Sync,
{
    Some(Box::new(ViziaEditor {
        vizia_state,
        app: Arc::new(app),

        scaling_factor: AtomicCell::new(None),
    }))
}

// TODO: Once we add resizing, we may want to be able to remember the GUI size. In that case we need
//       to make this serializable (only restoring the size of course) so it can be persisted.
pub struct ViziaState {
    size: AtomicCell<(u32, u32)>,
    open: AtomicBool,
}

impl ViziaState {
    /// Initialize the GUI's state. This value can be passed to [`create_vizia_editor()`]. The window
    /// size is in logical pixels, so before it is multiplied by the DPI scaling factor.
    pub fn from_size(width: u32, height: u32) -> Arc<ViziaState> {
        Arc::new(ViziaState {
            size: AtomicCell::new((width, height)),
            open: AtomicBool::new(false),
        })
    }

    /// Return a `(width, height)` pair for the current size of the GUI in logical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size.load()
    }

    /// Whether the GUI is currently visible.
    // Called `is_open()` instead of `open()` to avoid the ambiguity.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

/// An [`Editor`] implementation that calls an vizia draw loop.
struct ViziaEditor {
    vizia_state: Arc<ViziaState>,
    /// The user's app function.
    app: Arc<dyn Fn(&mut Context, &ParamSetter) + 'static + Send + Sync>,

    /// The scaling factor reported by the host, if any. On macOS this will never be set and we
    /// should use the system scaling factor instead.
    scaling_factor: AtomicCell<Option<f32>>,
}

impl Editor for ViziaEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send + Sync> {
        let app = self.app.clone();

        let (unscaled_width, unscaled_height) = self.vizia_state.size();
        let scaling_factor = self.scaling_factor.load();
        let window_description =
            WindowDescription::new().with_inner_size(unscaled_width, unscaled_height);

        let window = Application::new(window_description, move |cx| {
            let setter = ParamSetter::new(context.as_ref());

            // Set some default styles to match the iced integration
            // TODO: Maybe add a way to override this behavior
            // NOTE: vizia's font rendering looks way too dark and thick. Going one font weight
            //       lower seems to compensate for this.
            assets::register_fonts(cx);
            cx.set_default_font(assets::NOTO_SANS_LIGHT);

            // TOOD: `:root { background-color: #fafafa; }` in a stylesheet doesn't work
            Entity::root().set_background_color(cx, Color::rgb(250, 250, 250));
            // VIZIA uses points instead of pixels, this is 20px
            cx.add_theme("* { font-size: 15; }");

            // There doesn't seem to be any way to bundle styles with a widget, so we'll always
            // include the style sheet for our custom widgets at context creation
            widgets::register_theme(cx);

            // Any widget can change the parameters by emitting `ParamEvent` events. This model will
            // handle them automatically.
            widgets::ParamModel {
                context: context.clone(),
            }
            .build(cx);

            app(cx, &setter)
        })
        .with_scale_policy(
            scaling_factor
                .map(|factor| WindowScalePolicy::ScaleFactor(factor as f64))
                .unwrap_or(WindowScalePolicy::SystemScaleFactor),
        )
        .open_parented(&parent);

        self.vizia_state.open.store(true, Ordering::Release);
        Box::new(ViziaEditorHandle {
            vizia_state: self.vizia_state.clone(),
            window,
        })
    }

    fn size(&self) -> (u32, u32) {
        self.vizia_state.size()
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        self.scaling_factor.store(Some(factor));
        true
    }

    fn param_values_changed(&self) {
        // TODO: Update the GUI when this happens
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
unsafe impl Sync for ViziaEditorHandle {}

impl Drop for ViziaEditorHandle {
    fn drop(&mut self) {
        self.vizia_state.open.store(false, Ordering::Release);
        // XXX: This should automatically happen when the handle gets dropped, but apparently not
        self.window.close();
    }
}
