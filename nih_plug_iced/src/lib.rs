//! [iced](https://github.com/iced-rs/iced) editor support for NIH plug.
//!
//! TODO: Proper usage example, for now check out the gain_gui example

use baseview::{WindowOpenOptions, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use crossbeam::channel;
use nih_plug::prelude::{Editor, GuiContext, ParentWindowHandle};
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::widgets::ParamMessage;

/// Re-export for convenience.
pub use iced_baseview::*;

pub mod assets;
pub mod widgets;
mod wrapper;

/// Create an [`Editor`] instance using [iced](https://github.com/iced-rs/iced). The rough idea is
/// that you implement [`IcedEditor`], which is roughly analogous to iced's regular [`Application`]
/// trait except that it receives the [`GuiContext`] alongside its initialization flags so it can
/// update the parameter values. The [`IcedState`] passed to this function contains the GUI's
/// intitial size, and this is kept in sync whenever the GUI gets resized. You can also use this to
/// know if the GUI is open, so you can avoid performing potentially expensive calculations while
/// the GUI is not open. If you want this size to be persisted when restoring a plugin instance,
/// then you can store it in a `#[persist = "key"]` field on your parameters struct.
///
/// See [`IcedState::from_size()`].
pub fn create_iced_editor<E: IcedEditor>(
    iced_state: Arc<IcedState>,
    initialization_flags: E::InitializationFlags,
) -> Option<Box<dyn Editor>> {
    // We need some way to communicate parameter changes to the `IcedEditor` since parameter updates
    // come from outside of the editor's reactive model. This contains only capacity to store only
    // one parameter update, since we're only storing _that_ a parameter update has happened and not
    // which parameter so we'd need to redraw the entire GUI either way.
    let (parameter_updates_sender, parameter_updates_receiver) = channel::bounded(1);

    Some(Box::new(IcedEditorWrapper::<E> {
        iced_state,
        initialization_flags,

        scaling_factor: AtomicCell::new(None),

        parameter_updates_sender,
        parameter_updates_receiver: Arc::new(parameter_updates_receiver),
    }))
}

/// A plugin editor using `iced`. This wraps around [`Application`] with the only change being that
/// the usual `new()` function now additionally takes a `Arc<dyn GuiContext>` that the editor can
/// store to interact with the parameters. The editor should have a `Pin<Arc<impl Params>>` as part
/// of their [`InitializationFlags`][Self::InitializationFlags] so it can read the current parameter
/// values. See [`Application`] for more information.
pub trait IcedEditor: 'static + Send + Sync + Sized {
    /// See [`Application::Executor`]. You'll likely want to use [`crate::executor::Default`].
    type Executor: Executor;
    /// See [`Application::Message`]. You should have one variant containing a [`ParamMessage`].
    type Message: 'static + Clone + Debug + Send;
    /// See [`Application::Flags`].
    type InitializationFlags: 'static + Clone + Send + Sync;

    /// See [`Application::new`]. This also receivs the GUI context in addition to the flags.
    fn new(
        initialization_fags: Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>);

    /// Return a reference to the GUI context.
    /// [`handle_param_message()`][Self::handle_param_message()] uses this to interact with the
    /// parameters.
    fn context(&self) -> &dyn GuiContext;

    /// See [`Application::update`]. When receiving the variant that contains a
    /// [`widgets::ParamMessage`] you can call
    /// [`handle_param_message()`][Self::handle_param_message()] to handle the parameter update.
    fn update(
        &mut self,
        window: &mut WindowQueue,
        message: Self::Message,
    ) -> Command<Self::Message>;

    /// See [`Application::subscription`].
    fn subscription(
        &self,
        _window_subs: &mut WindowSubs<Self::Message>,
    ) -> Subscription<Self::Message> {
        Subscription::none()
    }

    /// See [`Application::view`].
    fn view(&mut self) -> Element<'_, Self::Message>;

    /// See [`Application::background_color`].
    fn background_color(&self) -> Color {
        Color::WHITE
    }

    /// See [`Application::scale_policy`].
    ///
    /// TODO: Is this needed? Editors shouldn't change the scale policy.
    fn scale_policy(&self) -> WindowScalePolicy {
        WindowScalePolicy::SystemScaleFactor
    }

    /// See [`Application::renderer_settings`].
    fn renderer_settings() -> iced_baseview::backend::settings::Settings {
        iced_baseview::backend::settings::Settings {
            // Enable some anti-aliasing by default. Since GUIs are likely very simple and most of
            // the work will be on the CPU anyways this should not affect performance much.
            antialiasing: Some(iced_baseview::backend::settings::Antialiasing::MSAAx4),
            // Use Noto Sans as the default font as that renders a bit more cleanly than the default
            // Lato font. This crate also contains other weights and versions of this font you can
            // use for individual widgets.
            default_font: Some(crate::assets::fonts::NOTO_SANS_REGULAR),
            ..iced_baseview::backend::settings::Settings::default()
        }
    }

    /// Handle a parameter update using the GUI context.
    fn handle_param_message(&self, message: ParamMessage) {
        // We can't use the fancy ParamSetter here because this needs to be type erased
        let context = self.context();
        match message {
            ParamMessage::BeginSetParameter(p) => unsafe { context.raw_begin_set_parameter(p) },
            ParamMessage::SetParameterNormalized(p, v) => unsafe {
                context.raw_set_parameter_normalized(p, v)
            },
            ParamMessage::ResetParameter(p) => unsafe {
                let default_value = context.raw_default_normalized_param_value(p);
                context.raw_set_parameter_normalized(p, default_value);
            },
            ParamMessage::EndSetParameter(p) => unsafe { context.raw_end_set_parameter(p) },
        }
    }
}

// TODO: Once we add resizing, we may want to be able to remember the GUI size. In that case we need
//       to make this serializable (only restoring the size of course) so it can be persisted.
pub struct IcedState {
    size: AtomicCell<(u32, u32)>,
    open: AtomicBool,
}

impl IcedState {
    /// Initialize the GUI's state. This value can be passed to [`create_iced_editor()`]. The window
    /// size is in logical pixels, so before it is multiplied by the DPI scaling factor.
    pub fn from_size(width: u32, height: u32) -> Arc<IcedState> {
        Arc::new(IcedState {
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

/// A marker struct to indicate that a parameter update has happened.
pub(crate) struct ParameterUpdate;

/// An [`Editor`] implementation that renders an iced [`Application`].
struct IcedEditorWrapper<E: IcedEditor> {
    iced_state: Arc<IcedState>,
    initialization_flags: E::InitializationFlags,

    /// The scaling factor reported by the host, if any. On macOS this will never be set and we
    /// should use the system scaling factor instead.
    scaling_factor: AtomicCell<Option<f32>>,

    /// A subscription for sending messages about parameter updates to the `IcedEditor`.
    parameter_updates_sender: channel::Sender<ParameterUpdate>,
    parameter_updates_receiver: Arc<channel::Receiver<ParameterUpdate>>,
}

impl<E: IcedEditor> Editor for IcedEditorWrapper<E> {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send + Sync> {
        let (unscaled_width, unscaled_height) = self.iced_state.size();
        let scaling_factor = self.scaling_factor.load();

        // TODO: iced_baseview does not have gracefuly error handling for context creation failures.
        //       This will panic if the context could not be created.
        let window = IcedWindow::<wrapper::IcedEditorWrapperApplication<E>>::open_parented(
            &parent,
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

                    #[cfg(feature = "opengl")]
                    gl_config: Some(baseview::gl::GlConfig {
                        // FIXME: glow_glyph forgot to add an `#extension`, so this won't work under
                        //        OpenGL 3.2 at the moment. With that change applied this should work on
                        //        OpenGL 3.2/macOS.
                        version: (3, 3),
                        red_bits: 8,
                        blue_bits: 8,
                        green_bits: 8,
                        alpha_bits: 8,
                        depth_bits: 24,
                        stencil_bits: 8,
                        samples: None,
                        srgb: true,
                        double_buffer: true,
                        vsync: true,
                        ..Default::default()
                    }),
                    // FIXME: Rust analyzer always thinks baseview/opengl is enabled even if we
                    //        don't explicitly enable it, so you'd get a compile error if this line
                    //        is missing
                    #[cfg(not(feature = "opengl"))]
                    gl_config: None,
                },
                iced_baseview: IcedBaseviewSettings {
                    ignore_non_modifier_keys: false,
                    always_redraw: true,
                },
                // We use this wrapper to be able to pass the GUI context to the editor
                flags: (
                    context,
                    self.parameter_updates_receiver.clone(),
                    self.initialization_flags.clone(),
                ),
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
        self.scaling_factor.store(Some(factor));
        true
    }

    fn param_values_changed(&self) {
        if self.iced_state.is_open() {
            // If there's already a paramter change notification in the channel then we don't need
            // to do anything else. This avoids queueing up redundant GUI redraws.
            let _ = self.parameter_updates_sender.try_send(ParameterUpdate);
        }
    }
}

/// The window handle used for [`IcedEditorWrapper`].
struct IcedEditorHandle<Message: 'static + Send> {
    iced_state: Arc<IcedState>,
    window: iced_baseview::WindowHandle<Message>,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl<Message: Send> Send for IcedEditorHandle<Message> {}
unsafe impl<Message: Send> Sync for IcedEditorHandle<Message> {}

impl<Message: Send> Drop for IcedEditorHandle<Message> {
    fn drop(&mut self) {
        self.iced_state.open.store(false, Ordering::Release);
        self.window.close_window();
    }
}
