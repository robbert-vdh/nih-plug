//! Utility helpers for the AU wrapper.

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
