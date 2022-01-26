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
use std::os::raw::c_char;
use vst3_sys::vst::TChar;
use widestring::U16CString;

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

/// The same as [strlcpy], but for VST3's fun UTF-16 strings instead.
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
