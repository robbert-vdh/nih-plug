//! Traits for working with MIDI SysEx data.

use std::borrow::{Borrow, BorrowMut};
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
    /// The byte array buffer the messages are read from and serialized to. Should be a `[u8; N]`,
    /// where `N` is the maximum supported message length in bytes. This covers the full message,
    /// see the trait's docstring for more information.
    ///
    /// Ideally this could just be a const generic but Rust doesn't let you use those as array
    /// lengths just yet.
    ///
    /// <https://github.com/rust-lang/rust/issues/60551>
    type Buffer: Borrow<[u8]> + BorrowMut<[u8]>;

    /// Read a SysEx message from `buffer` and convert it to this message type if supported. This
    /// covers the full message, see the trait's docstring for more information. `buffer`'s length
    /// matches the received message. It is not padded to match [`Buffer`][Self::Buffer].
    fn from_buffer(buffer: &[u8]) -> Option<Self>;

    /// Serialize this message object as a SysEx message in a byte buffer. This returns a buffer
    /// alongside the message's length in bytes. The buffer may contain padding at the end. This
    /// should contain the full message including headers and the EOX byte, see the trait's
    /// docstring for more information.
    fn to_buffer(self) -> (Self::Buffer, usize);
}

/// A default implementation plugins that don't need SysEx support can use.
impl SysExMessage for () {
    type Buffer = [u8; 0];

    fn from_buffer(_buffer: &[u8]) -> Option<Self> {
        None
    }

    fn to_buffer(self) -> (Self::Buffer, usize) {
        ([], 0)
    }
}
