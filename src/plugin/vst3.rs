use super::Plugin;
use crate::prelude::Vst3SubCategory;

/// Provides auxiliary metadata needed for a VST3 plugin.
pub trait Vst3Plugin: Plugin {
    /// The unique class ID that identifies this particular plugin. You can use the
    /// `*b"fooofooofooofooo"` syntax for this.
    ///
    /// This will be shuffled into a different byte order on Windows for project-compatibility.
    const VST3_CLASS_ID: [u8; 16];
    /// One or more subcategories. The host may use these to categorize the plugin. Internally this
    /// slice will be converted to a string where each character is separated by a pipe character
    /// (`|`). This string has a limit of 127 characters, and anything longer than that will be
    /// truncated.
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory];

    /// [`VST3_CLASS_ID`][Self::VST3_CLASS_ID`] in the correct order for the current platform so
    /// projects and presets can be shared between platforms. This should not be overridden.
    const PLATFORM_VST3_CLASS_ID: [u8; 16] = swap_vst3_uid_byte_order(Self::VST3_CLASS_ID);

    /// ARA 2: the plugin's `ARAFactory` (a pointer it keeps alive for the program's life), or null
    /// for a plugin without ARA. When set, the factory also lists an "ARA Main Factory Class" and
    /// the component answers `ARA::IPlugInEntryPoint` and `IPlugInEntryPoint2`.
    fn vst3_ara_factory() -> *const std::ffi::c_void {
        std::ptr::null()
    }

    /// ARA 2: binds the instance whose [`Self::vst3_extension_context()`] is `context` to the
    /// host's document controller; returns its `ARAPlugInExtensionInstance` (kept alive by the
    /// plugin), or null.
    fn vst3_ara_bind(context: *const std::ffi::c_void, document_controller: *mut std::ffi::c_void, known_roles: i32, assigned_roles: i32) -> *const std::ffi::c_void {
        let _ = (context, document_controller, known_roles, assigned_roles);
        std::ptr::null()
    }

    /// Asked once when the wrapper is made: what [`Self::vst3_ara_bind()`] gets as `context` for
    /// this instance (typically `Arc::as_ptr` of state the plugin keeps alive).
    fn vst3_extension_context(&self) -> *const std::ffi::c_void {
        std::ptr::null()
    }
}

/// The ARA main factory class's ID for a plugin: its class ID with every byte inverted.
pub const fn vst3_ara_class_id(cid: [u8; 16]) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = !cid[i];
        i += 1;
    }
    out
}

#[cfg(not(target_os = "windows"))]
const fn swap_vst3_uid_byte_order(uid: [u8; 16]) -> [u8; 16] {
    uid
}

#[cfg(target_os = "windows")]
const fn swap_vst3_uid_byte_order(mut uid: [u8; 16]) -> [u8; 16] {
    // No mutable references in const functions, so we can't use `uid.swap()`
    let original_uid = uid;

    uid[0] = original_uid[3];
    uid[1] = original_uid[2];
    uid[2] = original_uid[1];
    uid[3] = original_uid[0];

    uid[4] = original_uid[5];
    uid[5] = original_uid[4];
    uid[6] = original_uid[7];
    uid[7] = original_uid[6];

    uid
}
