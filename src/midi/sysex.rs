//! Traits for working with MIDI SysEx data.

use std::fmt::Debug;

/// A type that can be converted to and from byte buffers containing MIDI SysEx messages.
pub trait SysExMessage: Debug + Clone + PartialEq + Send + Sync {
    /// The maximum SysEx message size, in bytes.
    const MAX_BUFFER_SIZE: usize;

    /// Read a SysEx message from `buffer` and convert it to this message type if supported.
    /// `buffer`'s length matches the received message. It is not padded to `MAX_BUFFER_SIZE` bytes.
    fn from_buffer(buffer: &[u8]) -> Option<Self>;

    /// Serialize this message object as a SysEx message in `buffer`, returning the message's length
    /// in bytes.
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
