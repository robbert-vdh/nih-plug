// nih-plug: plugins, but rewritten in Rust
// Copyright (C) 2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

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
