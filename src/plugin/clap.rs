use super::Plugin;
use crate::prelude::{ClapFeature, RemoteControlsContext};

/// Provides auxiliary metadata needed for a CLAP plugin.
#[allow(unused_variables)]
pub trait ClapPlugin: Plugin {
    /// A unique ID that identifies this particular plugin. This is usually in reverse domain name
    /// notation, e.g. `com.manufacturer.plugin-name`.
    const CLAP_ID: &'static str;
    /// An optional short description for the plugin.
    const CLAP_DESCRIPTION: Option<&'static str>;
    /// The URL to the plugin's manual, if available.
    const CLAP_MANUAL_URL: Option<&'static str>;
    /// The URL to the plugin's support page, if available.
    const CLAP_SUPPORT_URL: Option<&'static str>;
    /// Keywords describing the plugin. The host may use this to classify the plugin in its plugin
    /// browser.
    const CLAP_FEATURES: &'static [ClapFeature];

    /// If set, this informs the host about the plugin's capabilities for polyphonic modulation.
    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = None;

    /// This function can be implemented to define plugin-specific [remote control
    /// pages](https://github.com/free-audio/clap/blob/main/include/clap/ext/draft/remote-controls.h)
    /// that the host can use to provide better hardware mapping for a plugin. See the linked
    /// extension for more information.
    fn remote_controls(&self, context: &mut impl RemoteControlsContext) {}

    /// Extra factories on the CLAP entry (for instance ARA's `org.ara-audio.ara.factory/2`):
    /// a pointer to a `#[repr(C)]` factory the plugin keeps alive for the program's life, or null.
    fn clap_factory(id: &std::ffi::CStr) -> *const std::ffi::c_void {
        let _ = id;
        std::ptr::null()
    }

    /// Extra plugin extensions the wrapper does not know (for instance ARA's plugin extension):
    /// a pointer to a `#[repr(C)]` extension struct the plugin keeps alive for the program's
    /// life, or null. Its functions get the `clap_plugin` pointer; reach this instance from it
    /// with [`crate::wrapper::clap::extension_context()`].
    fn clap_extension(id: &std::ffi::CStr) -> *const std::ffi::c_void {
        let _ = id;
        std::ptr::null()
    }

    /// Called once, when the wrapper is made: what [`crate::wrapper::clap::extension_context()`]
    /// returns for this instance (typically `Arc::as_ptr` of state the plugin keeps alive for its
    /// own life).
    fn clap_extension_context(&self) -> *const std::ffi::c_void {
        std::ptr::null()
    }
}

/// Configuration for the plugin's polyphonic modulation options, if it supports .
pub struct PolyModulationConfig {
    /// The maximum number of voices this plugin will ever use. Call the context's
    /// `set_current_voice_capacity()` method during initialization or audio processing to set the
    /// polyphony limit.
    pub max_voice_capacity: u32,
    /// If set to `true`, then the host may send note events for the same channel and key, but using
    /// different voice IDs. Bitwig Studio, for instance, can use this to do voice stacking. After
    /// enabling this, you should always prioritize using voice IDs to map note events to voices.
    pub supports_overlapping_voices: bool,
}
