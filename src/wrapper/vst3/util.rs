use std::cmp;
use std::ops::Deref;
use vst3_sys::interfaces::IUnknown;
use vst3_sys::vst::TChar;
use vst3_sys::ComInterface;
use widestring::U16CString;

/// When `Plugin::MIDI_INPUT` is set to `MidiConfig::MidiCCs` or higher then we'll register 130*16
/// additional parameters to handle MIDI CCs, channel pressure, and pitch bend, in that order.
/// vst3-sys doesn't expose these constants.
pub const VST3_MIDI_CCS: u32 = 130;
pub const VST3_MIDI_CHANNELS: u32 = 16;
/// The number of parameters we'll need to register if the plugin accepts MIDI CCs.
pub const VST3_MIDI_NUM_PARAMS: u32 = VST3_MIDI_CCS * VST3_MIDI_CHANNELS;
/// The start of the MIDI CC parameter ranges. We'll print an assertion failure if any of the
/// plugin's parameters overlap with this range. The mapping to a parameter index is
/// `VST3_MIDI_PARAMS_START + (cc_idx + (channel * VST3_MIDI_CCS))`.
pub const VST3_MIDI_PARAMS_START: u32 = VST3_MIDI_PARAMS_END - VST3_MIDI_NUM_PARAMS;
/// The (exclusive) end of the MIDI CC parameter range. Anything above this is reserved by the host.
pub const VST3_MIDI_PARAMS_END: u32 = 1 << 31;

/// Early exit out of a VST3 function when one of the passed pointers is null
macro_rules! check_null_ptr {
    ($ptr:expr $(, $ptrs:expr)* $(, )?) => {
        check_null_ptr_msg!("Null pointer passed to function", $ptr $(, $ptrs)*)
    };
}

/// The same as [`check_null_ptr!`], but with a custom message.
macro_rules! check_null_ptr_msg {
    ($msg:expr, $ptr:expr $(, $ptrs:expr)* $(, )?) => {
        if $ptr.is_null() $(|| $ptrs.is_null())* {
            nih_debug_assert_failure!($msg);
            return kInvalidArgument;
        }
    };
}

/// The same as [`strlcpy()`], but for VST3's fun UTF-16 strings instead.
pub fn u16strlcpy(dest: &mut [TChar], src: &str) {
    if dest.is_empty() {
        return;
    }

    let src_utf16 = match U16CString::from_str(src) {
        Ok(s) => s,
        Err(err) => {
            nih_debug_assert_failure!("Invalid UTF-16 string: {}", err);
            return;
        }
    };
    let src_utf16_chars = src_utf16.as_slice();
    let src_utf16_chars_signed: &[TChar] =
        unsafe { &*(src_utf16_chars as *const [u16] as *const [TChar]) };

    // Make sure there's always room for a null terminator
    let copy_len = cmp::min(dest.len() - 1, src_utf16_chars_signed.len());
    dest[..copy_len].copy_from_slice(&src_utf16_chars_signed[..copy_len]);
    dest[copy_len] = 0;
}

/// Send+Sync wrapper for these interface pointers.
#[repr(transparent)]
pub struct VstPtr<T: vst3_sys::ComInterface + ?Sized> {
    ptr: vst3_sys::VstPtr<T>,
}

/// The same as [`VstPtr`] with shared semnatics, but for objects we defined ourself since `VstPtr`
/// only works for interfaces.
#[repr(transparent)]
pub struct ObjectPtr<T: IUnknown> {
    ptr: *const T,
}

impl<T: ComInterface + ?Sized> Deref for VstPtr<T> {
    type Target = vst3_sys::VstPtr<T>;

    fn deref(&self) -> &Self::Target {
        &self.ptr
    }
}

impl<T: IUnknown> Deref for ObjectPtr<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ptr }
    }
}

impl<T: vst3_sys::ComInterface + ?Sized> From<vst3_sys::VstPtr<T>> for VstPtr<T> {
    fn from(ptr: vst3_sys::VstPtr<T>) -> Self {
        Self { ptr }
    }
}

impl<T: IUnknown> From<&T> for ObjectPtr<T> {
    /// Create a smart pointer for an existing reference counted object.
    fn from(obj: &T) -> Self {
        unsafe { obj.add_ref() };
        Self { ptr: obj }
    }
}

impl<T: IUnknown> Drop for ObjectPtr<T> {
    fn drop(&mut self) {
        unsafe { (*self).release() };
    }
}

/// SAFETY: Sharing these pointers across thread is s safe as they have internal atomic reference
/// counting, so as long as a `VstPtr<T>` handle exists the object will stay alive.
unsafe impl<T: ComInterface + ?Sized> Send for VstPtr<T> {}
unsafe impl<T: ComInterface + ?Sized> Sync for VstPtr<T> {}

unsafe impl<T: IUnknown> Send for ObjectPtr<T> {}
unsafe impl<T: IUnknown> Sync for ObjectPtr<T> {}

#[cfg(test)]
mod miri {
    use widestring::U16CStr;

    use super::*;

    #[test]
    fn u16strlcpy_normal() {
        let mut dest = [0; 256];
        u16strlcpy(&mut dest, "Hello, world!");

        assert_eq!(
            unsafe { U16CStr::from_ptr_str(dest.as_ptr() as *const u16) }
                .to_string()
                .unwrap(),
            "Hello, world!"
        );
    }

    #[test]
    fn u16strlcpy_overflow() {
        let mut dest = [0; 6];
        u16strlcpy(&mut dest, "Hello, world!");

        assert_eq!(
            unsafe { U16CStr::from_ptr_str(dest.as_ptr() as *const u16) }
                .to_string()
                .unwrap(),
            "Hello"
        );
    }
}
