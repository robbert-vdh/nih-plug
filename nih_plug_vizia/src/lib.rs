//! [VIZIA](https://github.com/vizia/vizia) editor support for NIH plug.

// See the comment in the main `nih_plug` crate
#![allow(clippy::type_complexity)]

use crossbeam::atomic::AtomicCell;
use nih_plug::params::persist::PersistentField;
use nih_plug::prelude::{Editor, GuiContext};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use vizia::prelude::*;

// Re-export for convenience
pub use vizia;

pub mod assets;
mod editor;
pub mod vizia_assets;
pub mod widgets;

/// Create an [`Editor`] instance using a [`vizia`][::vizia] GUI. The [`ViziaState`] passed to this
/// function contains the GUI's intitial size, and this is kept in sync whenever the GUI gets
/// resized. You can also use this to know if the GUI is open, so you can avoid performing
/// potentially expensive calculations while the GUI is not open. If you want this size to be
/// persisted when restoring a plugin instance, then you can store it in a `#[persist = "key"]`
/// field on your parameters struct.
///
/// The [`GuiContext`] is also passed to the app function. This is only meant for saving and
/// restoring state as part of your plugin's preset handling. You should not interact with this
/// directly to set parameters. Use the `ParamEvent`s instead.
///
/// The `theming` argument controls what level of theming to apply. If you use
/// [`ViziaTheming::Custom`], then you **need** to call
/// [`nih_plug_vizia::assets::register_noto_sans_light()`][assets::register_noto_sans_light()] at
/// the start of your app function. Vizia's included fonts are also not registered by default. If
/// you use the Roboto font that normally comes with Vizia or any of its emoji or icon fonts, you
/// also need to register those using the functions in
/// [`nih_plug_vizia::vizia_assets`][crate::vizia_assets].
///
/// See [VIZIA](https://github.com/vizia/vizia)'s repository for examples on how to use this.
pub fn create_vizia_editor<F>(
    vizia_state: Arc<ViziaState>,
    theming: ViziaTheming,
    app: F,
) -> Option<Box<dyn Editor>>
where
    F: Fn(&mut Context, Arc<dyn GuiContext>) + 'static + Send + Sync,
{
    Some(Box::new(editor::ViziaEditor {
        vizia_state,
        app: Arc::new(app),
        theming,

        // TODO: We can't get the size of the window when baseview does its own scaling, so if the
        //       host does not set a scale factor on Windows or Linux we should just use a factor of
        //       1. That may make the GUI tiny but it also prevents it from getting cut off.
        #[cfg(target_os = "macos")]
        scaling_factor: AtomicCell::new(None),
        #[cfg(not(target_os = "macos"))]
        scaling_factor: AtomicCell::new(Some(1.0)),

        emit_parameters_changed_event: Arc::new(AtomicBool::new(false)),
    }))
}

/// Controls what level of theming to apply to the editor.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default)]
pub enum ViziaTheming {
    /// Disable both `nih_plug_vizia`'s and vizia's built-in theming.
    None,
    /// Disable `nih_plug_vizia`'s custom theming. Vizia's included fonts are also not registered by
    /// default. If you use the Roboto font that normally comes with Vizia or any of its emoji or
    /// icon fonts, you need to register those using the functions in
    /// [`nih_plug_vizia::vizia_assets`][crate::vizia_assets].
    Builtin,
    /// Apply `nih_plug_vizia`'s custom theming. This is the default. You **need** to call
    /// [`nih_plug_vizia::assets::register_noto_sans_light()`][assets::register_noto_sans_light()]
    /// at the start of your app function for the font to work correctly.
    #[default]
    Custom,
}

/// State for an `nih_plug_vizia` editor. The scale factor can be manipulated at runtime by changing
/// `cx.user_scale_factor`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ViziaState {
    /// The window's size in logical pixels before applying `scale_factor`.
    #[serde(with = "nih_plug::params::persist::serialize_atomic_cell")]
    size: AtomicCell<(u32, u32)>,
    /// A scale factor that should be applied to `size` separate from from any system HiDPI scaling.
    /// This can be used to allow GUIs to be scaled uniformly.
    #[serde(with = "nih_plug::params::persist::serialize_atomic_cell")]
    scale_factor: AtomicCell<f64>,
    /// Whether the editor's window is currently open.
    #[serde(skip)]
    open: AtomicBool,

    /// Whether the size should be saved. If the window's size is always scaled uniformly, then this
    /// is not needed and can only result in problems.
    should_save_size: bool,
}

impl<'a> PersistentField<'a, ViziaState> for Arc<ViziaState> {
    fn set(&self, new_value: ViziaState) {
        if self.should_save_size {
            self.size.store(new_value.size.load());
        }
        self.scale_factor.store(new_value.scale_factor.load());
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&ViziaState) -> R,
    {
        f(self)
    }
}

impl ViziaState {
    /// Initialize the GUI's state. This value can be passed to [`create_vizia_editor()`]. The
    /// window size is in logical pixels, so before it is multiplied by the DPI scaling factor.
    ///
    /// Setting `should_save_size` to `false` may be useful when the size is supposed to be fixed
    /// and only the scaling factor changes. This allows the object to be persisted in a `Params`
    /// object without accidentally restoring old sizes after the window's logical size has changed
    /// in a plugin update.
    pub fn from_size(width: u32, height: u32, should_save_size: bool) -> Arc<ViziaState> {
        Arc::new(ViziaState {
            size: AtomicCell::new((width, height)),
            scale_factor: AtomicCell::new(1.0),
            open: AtomicBool::new(false),
            should_save_size,
        })
    }

    /// The same as [`from_size()`][Self::from_size()], but with a separate initial scale factor.
    /// This scale factor gets applied on top of any HiDPI scaling, and it can be modified at
    /// runtime by changing `cx.user_scale_factor`.
    pub fn from_size_with_scale(
        width: u32,
        height: u32,
        scale_factor: f64,
        should_save_size: bool,
    ) -> Arc<ViziaState> {
        Arc::new(ViziaState {
            size: AtomicCell::new((width, height)),
            scale_factor: AtomicCell::new(scale_factor),
            open: AtomicBool::new(false),
            should_save_size,
        })
    }

    /// Returns a `(width, height)` pair for the current size of the GUI in logical pixels, after
    /// applying the user scale factor.
    pub fn scaled_logical_size(&self) -> (u32, u32) {
        let (logical_width, logical_height) = self.size.load();
        let scale_factor = self.scale_factor.load();

        (
            (logical_width as f64 * scale_factor).round() as u32,
            (logical_height as f64 * scale_factor).round() as u32,
        )
    }

    /// Returns a `(width, height)` pair for the current size of the GUI in logical pixels before
    /// applying the user scale factor.
    pub fn inner_logical_size(&self) -> (u32, u32) {
        self.size.load()
    }

    /// Get the non-DPI related uniform scaling factor the GUI's size will be multiplied with. This
    /// can be changed by changing `cx.user_scale_factor`.
    pub fn user_scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }

    /// Whether the GUI is currently visible.
    // Called `is_open()` instead of `open()` to avoid the ambiguity.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}
