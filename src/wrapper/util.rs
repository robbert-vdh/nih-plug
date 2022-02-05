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

use std::cmp;
use std::marker::PhantomData;
use std::os::raw::c_char;
use vst3_sys::vst::TChar;
use widestring::U16CString;

#[cfg(all(debug_assertions, feature = "assert_process_allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// A Rabin fingerprint based string hash for parameter ID strings.
pub fn hash_param_id(id: &str) -> u32 {
    let mut overflow;
    let mut overflow2;
    let mut has_overflown = false;
    let mut hash: u32 = 0;
    for char in id.bytes() {
        (hash, overflow) = hash.overflowing_mul(31);
        (hash, overflow2) = hash.overflowing_add(char as u32);
        has_overflown |= overflow || overflow2;
    }

    if has_overflown {
        nih_log!(
            "Overflow while hashing param ID \"{}\", consider using 6 character IDs to avoid collissions",
            id
        );
    }

    // Studio One apparently doesn't like negative parameters, so JUCE just zeroes out the sign bit
    hash &= !(1 << 31);

    hash
}

/// The equivalent of the `strlcpy()` C function. Copy `src` to `dest` as a null-terminated
/// C-string. If `dest` does not have enough capacity, add a null terminator at the end to prevent
/// buffer overflows.
pub fn strlcpy(dest: &mut [c_char], src: &str) {
    if dest.is_empty() {
        return;
    }

    let src_bytes: &[u8] = src.as_bytes();
    let src_bytes_signed: &[i8] = unsafe { &*(src_bytes as *const [u8] as *const [i8]) };

    // Make sure there's always room for a null terminator
    let copy_len = cmp::min(dest.len() - 1, src.len());
    dest[..copy_len].copy_from_slice(&src_bytes_signed[..copy_len]);
    dest[copy_len] = 0;
}

/// The same as [strlcpy()], but for VST3's fun UTF-16 strings instead.
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

/// A wrapper around the entire process function, including the plugin wrapper parts. This sets up
/// `assert_no_alloc` if needed, while also making sure that things like FTZ are set up correctly if
/// the host has not already done so.
pub fn process_wrapper<T, F: FnOnce() -> T>(f: F) -> T {
    // Make sure FTZ is always enabled, even if the host doesn't do it for us
    let _ftz_guard = ScopedFtz::enable();

    cfg_if::cfg_if! {
        if #[cfg(all(debug_assertions, feature = "assert_process_allocs"))] {
            assert_no_alloc::assert_no_alloc(f)
        } else {
            f()
        }
    }
}

/// Enable the CPU's Flush To Zero flag while this object is in scope. If the flag was not already
/// set, it will be restored to its old value when this gets dropped.
struct ScopedFtz {
    /// The old FTZ mode to restore to, if FTZ was not already set.
    old_ftz_mode: Option<u32>,
    /// We can't directly implement !Send and !Sync, but this will do the same thing. This object
    /// affects the current thread's floating point registers, so it may only be dropped on the
    /// current thread.
    send_sync_marker: PhantomData<*const ()>,
}

impl ScopedFtz {
    fn enable() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(target_feature = "sse")] {
                let mode = unsafe { std::arch::x86_64::_MM_GET_FLUSH_ZERO_MODE() };
                if mode != std::arch::x86_64::_MM_FLUSH_ZERO_ON {
                    unsafe { std::arch::x86_64::_MM_SET_FLUSH_ZERO_MODE(std::arch::x86_64::_MM_FLUSH_ZERO_ON) };

                    Self {
                        old_ftz_mode: Some(mode),
                        send_sync_marker: PhantomData,
                    }
                } else {
                    Self {
                        old_ftz_mode: None,
                        send_sync_marker: PhantomData,
                    }
                }
            } else {
                Self {
                    old_ftz_mode: None,
                    send_sync_marker: PhantomData,
                }
            }
        }
    }
}

impl Drop for ScopedFtz {
    fn drop(&mut self) {
        if let Some(mode) = self.old_ftz_mode {
            cfg_if::cfg_if! {
                if #[cfg(target_feature = "sse")] {
                    unsafe { std::arch::x86_64::_MM_SET_FLUSH_ZERO_MODE(mode) };
                }
            };
        }
    }
}
