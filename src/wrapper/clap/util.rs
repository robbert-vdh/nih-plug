use clap_sys::stream::{clap_istream, clap_ostream};
use std::mem::MaybeUninit;
use std::ops::Deref;
use std::os::raw::c_void;

/// Early exit out of a function with the specified return value when one of the passed pointers is
/// null.
macro_rules! check_null_ptr {
    ($ret:expr, $ptr:expr $(, $ptrs:expr)* $(, )?) => {
        check_null_ptr_msg!("Null pointer passed to function", $ret, $ptr $(, $ptrs)*)
    };
}

/// The same as [`check_null_ptr!`], but with a custom message.
macro_rules! check_null_ptr_msg {
    ($msg:expr, $ret:expr, $ptr:expr $(, $ptrs:expr)* $(, )?) => {
        // Clippy doesn't understand it when we use a unit in our `check_null_ptr!()` maccro, even
        // if we explicitly pattern match on that unit
        #[allow(clippy::unused_unit)]
        if $ptr.is_null() $(|| $ptrs.is_null())* {
            nih_debug_assert_failure!($msg);
            return $ret;
        }
    };
}

/// Send+Sync wrapper around CLAP host extension pointers.
pub struct ClapPtr<T> {
    inner: *const T,
}

impl<T> Deref for ClapPtr<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner }
    }
}

unsafe impl<T> Send for ClapPtr<T> {}
unsafe impl<T> Sync for ClapPtr<T> {}

impl<T> ClapPtr<T> {
    /// Create a wrapper around a CLAP object pointer.
    ///
    /// # Safety
    ///
    /// The pointer must point to a valid object with a lifetime that exceeds this object.
    pub unsafe fn new(ptr: *const T) -> Self {
        Self { inner: ptr }
    }
}

/// A buffer a stream can be read into. This is needed to allow reading into uninitialized vectors
/// using slices without invoking UB.
///
/// # Safety
///
/// This may only be implemented by slices of `u8` and types with the same representation as `u8`.
pub unsafe trait ByteReadBuffer {
    /// The length of the slice, in bytes.
    fn len(&self) -> usize;

    /// Get a pointer to the start of the stream.
    fn as_mut_ptr(&mut self) -> *mut u8;
}

unsafe impl ByteReadBuffer for &mut [u8] {
    fn len(&self) -> usize {
        // Bit of a fun one since we reuse the names of the original functions
        <[u8]>::len(self)
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        <[u8]>::as_mut_ptr(self)
    }
}

unsafe impl ByteReadBuffer for &mut [MaybeUninit<u8>] {
    fn len(&self) -> usize {
        <[MaybeUninit<u8>]>::len(self)
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        <[MaybeUninit<u8>]>::as_mut_ptr(self) as *mut u8
    }
}

/// Read from a stream until either the byte slice as been filled, or the stream doesn't contain any
/// data anymore. This correctly handles streams that only allow smaller, buffered reads. If the
/// stream ended before the entire slice has been filled, then this will return `false`.
pub fn read_stream(stream: &clap_istream, mut slice: impl ByteReadBuffer) -> bool {
    let mut read_pos = 0;
    while read_pos < slice.len() {
        let bytes_read = unsafe {
            (stream.read)(
                stream,
                slice.as_mut_ptr().add(read_pos) as *mut c_void,
                (slice.len() - read_pos) as u64,
            )
        };
        if bytes_read <= 0 {
            return false;
        }

        read_pos += bytes_read as usize;
    }

    true
}

/// Write the data from a slice to a stream until either all data has been written, or the stream
/// returns an error. This correctly handles streams that only allow smaller, buffered writes. This
/// returns `false` if the stream returns an error or doesn't allow any writes anymore.
pub fn write_stream(stream: &clap_ostream, slice: &[u8]) -> bool {
    let mut write_pos = 0;
    while write_pos < slice.len() {
        let bytes_written = unsafe {
            (stream.write)(
                stream,
                slice.as_ptr().add(write_pos) as *const c_void,
                (slice.len() - write_pos) as u64,
            )
        };
        if bytes_written <= 0 {
            return false;
        }

        write_pos += bytes_written as usize;
    }

    true
}
