//! Traits for working with MIDI SysEx data.

use std::fmt::Debug;

/// A type that can be converted to and from byte buffers containing MIDI SysEx messages.
///
/// # SysEx buffers
///
/// For maximum flexibility this trait works with RAW MIDI messages. This means that status bytes
/// and end of SysEx (EOX) bytes are included in the input, and should also be included in the
/// output. A consequence of this is that it is also possible to support system common and system
/// real time messages as needed, as long as the plugin API supports those.
///
/// For example, the message to turn general MIDI mode on is `[0xf0, 0x7e, 0x7f, 0x09, 0x01, 0xf7]`,
/// and has a length of 6 bytes. Note that this includes the `0xf0` start byte and `0xf7` end byte.
pub trait SysExMessage: Debug + Clone + PartialEq + Send + Sync {
    /// The maximum SysEx message size, in bytes. This covers the full message, see the trait's
    /// docstring for more information.
    const MAX_BUFFER_SIZE: usize;

    /// Read a SysEx message from `buffer` and convert it to this message type if supported. This
    /// covers the full message, see the trait's docstring for more information. `buffer`'s length
    /// matches the received message. It is not padded to `MAX_BUFFER_SIZE` bytes.
    fn from_buffer(buffer: &[u8]) -> Option<Self>;

    /// Serialize this message object as a SysEx message in `buffer`, returning the message's length
    /// in bytes. This should contain the full message including headers and the EOX byte, see the
    /// trait's docstring for more information.
    ///
    /// `buffer` is a `[u8; Self::MAX_BUFFER_SIZE]`, but Rust currently doesn't allow using
    /// associated constants in method types:
    ///
    /// <https://github.com/rust-lang/rust/issues/60551>
    fn to_buffer(self, buffer: &mut [u8]) -> usize;
}

/// A default implementation plugins that don't need SysEx support can use.
impl SysExMessage for () {
    const MAX_BUFFER_SIZE: usize = 0;

    fn from_buffer(_buffer: &[u8]) -> Option<Self> {
        None
    }

    fn to_buffer(self, _buffer: &mut [u8]) -> usize {
        0
    }
}
