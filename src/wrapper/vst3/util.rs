use std::ops::Deref;
use vst3_sys::{interfaces::IUnknown, ComInterface};

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

/// Send+Sync wrapper for these interface pointers.
#[repr(transparent)]
pub struct VstPtr<T: vst3_sys::ComInterface + ?Sized> {
    ptr: vst3_sys::VstPtr<T>,
}

/// The same as [`VstPtr`] with shared semnatics, but for objects we defined ourself since VstPtr
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
