//! Static metadata describing an AU plugin. Mirrors the role of `PluginInfo`
//! in the VST3 wrapper — collects everything the AU host needs to identify
//! and instantiate the plugin so it can be type-erased and stored in an
//! array (allowing `nih_export_au!` to export multiple plugins eventually).

/// Metadata for one exported AU plugin.
#[derive(Debug)]
pub struct PluginInfo {
    /// Plugin name, used for the `Info.plist`'s `CFBundleName` and the
    /// `AudioComponents` array entries.
    pub name: &'static str,
    /// Manufacturer / vendor display name.
    pub vendor: &'static str,
    /// Plugin version string ("0.1.3"). Used for `CFBundleShortVersionString`.
    pub version: &'static str,
    /// Optional URL exposed via the manufacturer info.
    pub url: &'static str,
    /// Optional contact email.
    pub email: &'static str,

    /// Audio Unit type (e.g. `*b"aufx"`). Internally stored as `u32` in
    /// big-endian byte order — the order Apple's CoreAudio APIs use.
    pub au_type: u32,
    /// Audio Unit subtype four-char code.
    pub au_subtype: u32,
    /// Audio Unit manufacturer four-char code.
    pub au_manufacturer: u32,
    /// 32-bit version number `(major << 16) | (minor << 8) | patch`.
    pub au_version: u32,
}

/// Build a `u32` four-char code from a 4-byte array.
///
/// Apple's CoreAudio APIs interpret these as big-endian uint32 — for `*b"aufx"`
/// the resulting `u32` is `0x61756678`.
pub const fn fourcc(bytes: [u8; 4]) -> u32 {
    ((bytes[0] as u32) << 24)
        | ((bytes[1] as u32) << 16)
        | ((bytes[2] as u32) << 8)
        | (bytes[3] as u32)
}
