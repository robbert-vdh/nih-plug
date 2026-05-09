use super::Plugin;

/// Provides auxiliary metadata needed for an Apple Audio Unit (AUv2) plugin.
///
/// AU plugins are identified by three four-character codes: a type (e.g. `aufx`
/// for an effect, `aumu` for a music device), a subtype (a unique identifier
/// for the specific plugin from this manufacturer), and a manufacturer code
/// (a four-character identifier for the vendor — must be registered with Apple
/// for App Store distribution but for direct distribution any unique code works).
pub trait AuPlugin: Plugin {
    /// The Audio Unit type four-character code.
    ///
    /// Common values:
    /// - `*b"aufx"` for an effect (most plugins)
    /// - `*b"aumu"` for a music device / instrument
    /// - `*b"aumf"` for a music effect (effect that handles MIDI)
    /// - `*b"augn"` for a generator
    ///
    /// See `au-sys` constants `kAudioUnitType_*` for the full list.
    const AU_TYPE: [u8; 4];

    /// A four-character subtype that uniquely identifies this plugin within
    /// the manufacturer's product line. Choose any code that hasn't been used
    /// for one of your other plugins.
    const AU_SUBTYPE: [u8; 4];

    /// A four-character manufacturer code. Apple recommends registering this
    /// at <https://developer.apple.com/datatype/> for App Store distribution.
    /// For direct distribution any unique code works.
    const AU_MANUFACTURER: [u8; 4];

    /// Optional version number encoded as a single 32-bit integer in
    /// "major.minor.patch" form: `(major << 16) | (minor << 8) | patch`.
    /// Defaults to 1.0.0 (`0x10000`).
    const AU_VERSION: u32 = 0x0001_0000;
}
