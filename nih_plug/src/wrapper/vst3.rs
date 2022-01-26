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

//! Wrappers for different plugin types. Each wrapper has an entry point macro that you can pass the
//! name of a type that implements `Plugin` to. The macro will handle the rest.

use std::pin::Pin;

use crate::plugin::Plugin;

pub struct Vst3Wrapper<P: Plugin> {
    plugin: Pin<Box<P>>,
}

// TODO: Come up with some way to hae Cargo spit out a VST3 module. Is that possible without a
//       custom per-plugin build script?
#[macro_export]
macro_rules! nih_export_vst3 {
    ($plugin_ty:ty) => {
        // These two entry points are used on Linux, and they would theoretically also be used on
        // the BSDs:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/c3948deb407bdbff89de8fb6ab8500ea4df9d6d9/source/main/linuxmain.cpp#L47-L52
        #[no_mangle]
        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        pub extern "C" fn ModuleEntry(_lib_handle: *mut std::ffi::c_void) -> bool {
            true
        }

        #[no_mangle]
        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        pub extern "C" fn ModuleExit() -> bool {
            true
        }

        // These two entry points are used on macOS:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/bc459feee68803346737901471441fd4829ec3f9/source/main/macmain.cpp#L60-L61
        #[no_mangle]
        #[cfg(target_os = "macos")]
        pub extern "C" fn bundleEntry(_lib_handle: *mut std::ffi::c_void) -> bool {
            true
        }

        #[no_mangle]
        #[cfg(target_os = "macos")]
        pub extern "C" fn bundleExit() -> bool {
            true
        }

        // And these two entry points are used on Windows:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/bc459feee68803346737901471441fd4829ec3f9/source/main/dllmain.cpp#L59-L60
        #[no_mangle]
        #[cfg(target_os = "windows")]
        pub extern "system" fn InitModule() -> bool {
            true
        }

        #[no_mangle]
        #[cfg(target_os = "windows")]
        pub extern "system" fn DeinitModule() -> bool {
            true
        }
    };
}
