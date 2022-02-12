use lazy_static::lazy_static;

use crate::wrapper::util::hash_param_id;

/// Right now the wrapper adds its own bypass parameter.
///
/// TODO: Actually use this parameter.
pub const BYPASS_PARAM_ID: &str = "bypass";
lazy_static! {
    pub static ref BYPASS_PARAM_HASH: u32 = hash_param_id(BYPASS_PARAM_ID);
}

/// Early exit out of a VST3 function when one of the passed pointers is null
macro_rules! check_null_ptr {
    ($ptr:expr $(, $ptrs:expr)* $(, )?) => {
        check_null_ptr_msg!("Null pointer passed to function", $ptr $(, $ptrs)*)
    };
}

/// The same as [check_null_ptr!], but with a custom message.
macro_rules! check_null_ptr_msg {
    ($msg:expr, $ptr:expr $(, $ptrs:expr)* $(, )?) => {
        if $ptr.is_null() $(|| $ptrs.is_null())* {
            nih_debug_assert_failure!($msg);
            return kInvalidArgument;
        }
    };
}

/// Send+Sync wrapper for these interface pointers.
#[repr(transparent)]
pub struct VstPtr<T: vst3_sys::ComInterface + ?Sized> {
    ptr: vst3_sys::VstPtr<T>,
}

impl<T: vst3_sys::ComInterface + ?Sized> std::ops::Deref for VstPtr<T> {
    type Target = vst3_sys::VstPtr<T>;

    fn deref(&self) -> &Self::Target {
        &self.ptr
    }
}

impl<T: vst3_sys::ComInterface + ?Sized> From<vst3_sys::VstPtr<T>> for VstPtr<T> {
    fn from(ptr: vst3_sys::VstPtr<T>) -> Self {
        Self { ptr }
    }
}

/// SAFETY: Sharing these pointers across thread is s safe as they have internal atomic reference
/// counting, so as long as a `VstPtr<T>` handle exists the object will stay alive.
unsafe impl<T: vst3_sys::ComInterface + ?Sized> Send for VstPtr<T> {}
unsafe impl<T: vst3_sys::ComInterface + ?Sized> Sync for VstPtr<T> {}
