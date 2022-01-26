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

use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem;
// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;
use vst3_sys::base::{kInvalidArgument, kNoInterface, kResultFalse, kResultOk, tresult, TBool};
use vst3_sys::base::{IPluginBase, IPluginFactory, IPluginFactory2, IPluginFactory3};
use vst3_sys::vst::IComponent;
use vst3_sys::VST3;

use crate::plugin::{BusConfig, Plugin};
use crate::wrapper::util::{strlcpy, u16strlcpy};

/// Re-export for the wrapper.
pub use vst3_sys::sys::GUID;

#[VST3(implements(IComponent))]
pub struct Wrapper<P: Plugin> {
    plugin: P,
    current_bus_config: BusConfig,
}

impl<P: Plugin> Wrapper<P> {
    pub fn new() -> Box<Self> {
        Self::allocate(
            P::default(),
            // Some hosts, like the current version of Bitwig and Ardour at the time of writing,
            // will try using the plugin's default not yet initialized bus arrangement. Because of
            // that, we'll always initialize this configuration even before the host requests a
            // channel layout.
            BusConfig {
                num_input_channels: P::DEFAULT_NUM_INPUTS,
                num_output_channels: P::DEFAULT_NUM_OUTPUTS,
            },
        )
    }
}

impl<P: Plugin> IPluginBase for Wrapper<P> {
    unsafe fn initialize(&self, _context: *mut c_void) -> tresult {
        // We currently don't need or allow any initialization logic
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl<P: Plugin> IComponent for Wrapper<P> {
    unsafe fn get_controller_class_id(&self, _tuid: *mut vst3_sys::IID) -> tresult {
        // We won't separate the edit controller to keep the implemetnation a bit smaller
        kNoInterface
    }

    unsafe fn set_io_mode(&self, _mode: vst3_sys::vst::IoMode) -> tresult {
        // This would need to integrate with the GUI, which we currently don't have
        kResultOk
    }

    unsafe fn get_bus_count(
        &self,
        type_: vst3_sys::vst::MediaType,
        _dir: vst3_sys::vst::BusDirection,
    ) -> i32 {
        // All plugins currently only have a single input and a single output bus
        match type_ {
            x if x == vst3_com::vst::MediaTypes::kAudio as i32 => 1,
            _ => 0,
        }
    }

    unsafe fn get_bus_info(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
        index: i32,
        info: *mut vst3_sys::vst::BusInfo,
    ) -> tresult {
        match type_ {
            t if t == vst3_com::vst::MediaTypes::kAudio as i32 => {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_com::vst::MediaTypes::kAudio as i32;
                info.bus_type = vst3_com::vst::BusTypes::kMain as i32;
                info.flags = vst3_com::vst::BusFlags::kDefaultActive as u32;
                match (dir, index) {
                    (d, 0) if d == vst3_com::vst::BusDirections::kInput as i32 => {
                        info.direction = vst3_com::vst::BusDirections::kInput as i32;
                        info.channel_count = self.current_bus_config.num_input_channels as i32;
                        u16strlcpy(&mut info.name, "Input");

                        kResultOk
                    }
                    (d, 0) if d == vst3_com::vst::BusDirections::kOutput as i32 => {
                        info.direction = vst3_com::vst::BusDirections::kOutput as i32;
                        info.channel_count = self.current_bus_config.num_output_channels as i32;
                        u16strlcpy(&mut info.name, "Output");

                        kResultOk
                    }
                    _ => kInvalidArgument,
                }
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn get_routing_info(
        &self,
        in_info: *mut vst3_sys::vst::RoutingInfo,
        out_info: *mut vst3_sys::vst::RoutingInfo,
    ) -> tresult {
        *out_info = mem::zeroed();

        let in_info = &*in_info;
        let out_info = &mut *out_info;
        match (in_info.media_type, in_info.bus_index) {
            (t, 0) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => {
                out_info.media_type = vst3_sys::vst::MediaTypes::kAudio as i32;
                out_info.bus_index = in_info.bus_index;
                out_info.channel = in_info.channel;

                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn activate_bus(
        &self,
        type_: vst3_sys::vst::MediaType,
        _dir: vst3_sys::vst::BusDirection,
        index: i32,
        _state: vst3_sys::base::TBool,
    ) -> tresult {
        // We don't need any special handling here
        match (type_, index) {
            (t, 0) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => kResultOk,
            _ => kInvalidArgument,
        }
    }

    unsafe fn set_active(&self, _state: TBool) -> tresult {
        // We don't need any special handling here
        kResultOk
    }

    unsafe fn set_state(&self, _state: *mut c_void) -> tresult {
        // TODO: Implemnt state saving and restoring
        kResultFalse
    }

    unsafe fn get_state(&self, _state: *mut c_void) -> tresult {
        // TODO: Implemnt state saving and restoring
        kResultFalse
    }
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
        *info = mem::zeroed();

        let info = &mut *info;
        strlcpy(&mut info.vendor, P::NAME);
        strlcpy(&mut info.url, P::URL);
        strlcpy(&mut info.email, P::EMAIL);
        info.flags = vst3_sys::base::FactoryFlags::kUnicode as i32;

        kResultOk
    }

    unsafe fn count_classes(&self) -> i32 {
        // We don't do shell plugins, and good of an idea having separated components and edit
        // controllers in theory is, few software can use it, and doing that would make our simple
        // microframework a lot less simple
        1
    }

    unsafe fn get_class_info(&self, index: i32, info: *mut vst3_sys::base::PClassInfo) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid = self.cid;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        strlcpy(&mut info.name, P::NAME);

        kResultOk
    }

    unsafe fn create_instance(
        &self,
        cid: *const vst3_sys::IID,
        _iid: *const vst3_sys::IID,
        obj: *mut *mut vst3_sys::c_void,
    ) -> tresult {
        if *cid != self.cid {
            return kNoInterface;
        }

        *obj = Box::into_raw(Wrapper::<P>::new()) as *mut vst3_sys::c_void;

        kResultOk
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
