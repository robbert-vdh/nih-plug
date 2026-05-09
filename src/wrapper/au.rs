//! Apple Audio Unit (AUv2) wrapper for nih-plug.
//!
//! Phase 1 (current): minimum viable AU — registers via
//! `AudioComponentPlugInInterface`, exposes the basic property selectors
//! (`SampleRate`, `StreamFormat`, `Latency`, `MaximumFramesPerSlice`,
//! `SupportedNumChannels`), and renders silence.  `auval` should validate
//! the component end-to-end at this stage.
//!
//! Phase 2: parameters + state.
//! Phase 3: real `Plugin::process` render path.
//! Phase 4: Cocoa view via `objc2-app-kit`.
//! Phase 5: bundle generation in `nih_plug_xtask`.

mod factory;
mod wrapper;

pub use factory::{fourcc, PluginInfo};
pub use wrapper::{AuWrapper, Wrapper};

// Re-export `au-sys` so the macro can refer to its types without a
// separate dependency declaration in the consuming crate.
pub use au_sys;

/// Export an Audio Unit plugin from this library using the provided plugin type.
///
/// Generates an `extern "C"` factory function whose name (`nih_plug_au_factory`)
/// must match the `factoryFunction` value in the bundle's `Info.plist`
/// `AudioComponents` entry. The bundler in `nih_plug_xtask` writes that
/// automatically when `bundle-au` is used.
///
/// Currently only one plugin per library is supported (Phase 1 limitation).
///
/// # Example
///
/// ```ignore
/// use nih_plug::prelude::*;
///
/// struct MyEffect { /* ... */ }
/// impl Plugin for MyEffect { /* ... */ }
/// impl AuPlugin for MyEffect {
///     const AU_TYPE: [u8; 4] = *b"aufx";
///     const AU_SUBTYPE: [u8; 4] = *b"MyFx";
///     const AU_MANUFACTURER: [u8; 4] = *b"MyCo";
/// }
///
/// nih_export_au!(MyEffect);
/// ```
#[macro_export]
macro_rules! nih_export_au {
    ($plugin_ty:ty) => {
        // C-linkage factory function. Apple's component manager calls this
        // with the requested `AudioComponentDescription`; we ignore it (the
        // Info.plist already filtered the lookup) and return a fresh
        // wrapper instance.
        #[doc(hidden)]
        #[no_mangle]
        pub unsafe extern "C" fn nih_plug_au_factory(
            _desc: *const $crate::wrapper::au::au_sys::AudioComponentDescription,
        ) -> *mut ::std::ffi::c_void {
            let iface = $crate::wrapper::au::Wrapper::<$plugin_ty>::new();
            iface as *mut ::std::ffi::c_void
        }
    };
}
