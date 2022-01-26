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

use std::marker::PhantomData;
use std::mem;
use std::pin::Pin;

// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;
use vst3_sys::base::tresult;
use vst3_sys::base::{IPluginFactory, IPluginFactory2, IPluginFactory3};
use vst3_sys::VST3;

use crate::plugin::Plugin;

/// Re-export for the wrapper.
pub use vst3_sys::sys::GUID;

pub struct Wrapper<P: Plugin> {
    plugin: Pin<Box<P>>,
}

// TODO: Implement the rest
// #[VST3(implements(IPluginFactory, IPluginFactory2, IPluginFactory3))]
#[VST3(implements(IPluginFactory))]
pub struct Factory<P: Plugin> {
    /// The exposed plugin's GUID. Instead of generating this, we'll just let the programmer decide
    /// on their own.
    cid: GUID,
    /// The type will be used for constructing plugin instances later.
    _phantom: PhantomData<P>,
}

impl<P: Plugin> Factory<P> {
    pub fn new(guid: GUID) -> Box<Self> {
        Self::allocate(guid, PhantomData::default())
    }
}

impl<P: Plugin> IPluginFactory for Factory<P> {
    unsafe fn get_factory_info(&self, info: *mut vst3_sys::base::PFactoryInfo) -> tresult {
        todo!()
    }

    unsafe fn count_classes(&self) -> i32 {
        todo!()
    }

    unsafe fn get_class_info(&self, index: i32, info: *mut vst3_sys::base::PClassInfo) -> tresult {
        todo!()
    }

    unsafe fn create_instance(
        &self,
        cid: *const vst3_com::IID,
        _iid: *const vst3_com::IID,
        obj: *mut *mut vst3_com::c_void,
    ) -> tresult {
        todo!()
    }
}

/// Export a VST3 plugin from this library using the provided plugin type and a 4x4 character class
/// ID. This CID should be a `[u8; 16]`. You can use the `*b"fooofooofooofooo"` syntax for this.
///
/// TODO: Come up with some way to hae Cargo spit out a VST3 module. Is that possible without a
///       custom per-plugin build script?
#[macro_export]
macro_rules! nih_export_vst3 {
    ($plugin_ty:ty, $cid:expr) => {
        #[no_mangle]
        pub extern "system" fn GetPluginFactory() -> *mut ::std::ffi::c_void {
            let factory = ::nih_plug::wrapper::vst3::Factory::<$plugin_ty>::new(
                ::nih_plug::wrapper::vst3::GUID { data: $cid },
            );

            Box::into_raw(factory) as *mut ::std::ffi::c_void
        }

        // We don't need any special initialization logic, so all of these module entry point
        // functions just return true all the time

        // These two entry points are used on Linux, and they would theoretically also be used on
        // the BSDs:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/c3948deb407bdbff89de8fb6ab8500ea4df9d6d9/source/main/linuxmain.cpp#L47-L52
        #[no_mangle]
        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        pub extern "C" fn ModuleEntry(_lib_handle: *mut ::std::ffi::c_void) -> bool {
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
        pub extern "C" fn bundleEntry(_lib_handle: *mut ::std::ffi::c_void) -> bool {
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
