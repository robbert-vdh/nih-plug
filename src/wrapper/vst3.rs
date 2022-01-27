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
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem;
use vst3_sys::base::{kInvalidArgument, kNoInterface, kResultFalse, kResultOk, tresult, TBool};
use vst3_sys::base::{IPluginBase, IPluginFactory, IPluginFactory2, IPluginFactory3};
use vst3_sys::vst::TChar;
use vst3_sys::vst::{IComponent, IEditController};
use vst3_sys::VST3;
use widestring::U16CStr;

use crate::params::ParamPtr;
use crate::plugin::{BusConfig, Plugin, Vst3Plugin};
use crate::wrapper::util::{hash_param_id, strlcpy, u16strlcpy};

// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;

/// Re-export for the wrapper.
pub use vst3_sys::sys::GUID;

/// The VST3 SDK version this is roughtly based on.
const VST3_SDK_VERSION: &str = "VST 3.6.14";
/// Right now the wrapper adds its own bypass parameter.
///
/// TODO: Actually use this parameter.
const BYPASS_PARAM_ID: &str = "bypass";
lazy_static! {
    static ref BYPASS_PARAM_HASH: u32 = hash_param_id(BYPASS_PARAM_ID);
}

#[VST3(implements(IComponent, IEditController))]
pub struct Wrapper<P: Plugin> {
    /// The wrapped plugin instance.
    plugin: P,
    /// Whether the plugin is currently bypassed. This is not yet integrated with the `Plugin`
    /// trait.
    bypass_state: Cell<bool>,

    /// A mapping from parameter ID hashes (obtained from the string parameter IDs) to pointers to
    /// parameters belonging to the plugin. As long as `plugin` does not get recreated, these
    /// addresses will remain stable, as they are obtained from a pinned object.
    param_by_hash: HashMap<u32, ParamPtr>,
    /// The keys from `param_map` in a stable order.
    param_hashes: Vec<u32>,
    /// The default normalized parameter value for every parameter in `param_ids`. We need to store
    /// this in case the host requeries the parmaeter later.
    param_defaults_normalized: Vec<f32>,
    /// Mappings from parameter hashes back to string parameter indentifiers. Useful for debug
    /// logging and when handling plugin state.
    param_id_hashes: HashMap<u32, &'static str>,

    /// The current bus configuration, modified through `IAudioProcessor::setBusArrangements()`.
    current_bus_config: RefCell<BusConfig>,
}

impl<P: Plugin> Wrapper<P> {
    pub fn new() -> Box<Self> {
        let mut wrapper = Self::allocate(
            P::default(),     // plugin
            Cell::new(false), // bypass_state
            HashMap::new(),   // param_by_hash
            Vec::new(),       // param_hashes
            Vec::new(),       // param_defaults_normalized
            HashMap::new(),   // param_id_hashes
            // Some hosts, like the current version of Bitwig and Ardour at the time of writing,
            // will try using the plugin's default not yet initialized bus arrangement. Because of
            // that, we'll always initialize this configuration even before the host requests a
            // channel layout.
            RefCell::new(BusConfig {
                num_input_channels: P::DEFAULT_NUM_INPUTS,
                num_output_channels: P::DEFAULT_NUM_OUTPUTS,
            }),
        );

        // This is a mapping from the parameter IDs specified by the plugin to pointers to thsoe
        // parameters. Since the object returned by `params()` is pinned, these pointers are safe to
        // dereference as long as `wrapper.plugin` is alive
        let param_map = wrapper.plugin.params().param_map();
        nih_debug_assert!(
            !param_map.contains_key(BYPASS_PARAM_ID),
            "The wrapper alread yadds its own bypass parameter"
        );

        wrapper.param_by_hash = param_map
            .iter()
            .map(|(id, p)| (hash_param_id(id), *p))
            .collect();
        wrapper.param_hashes = wrapper.param_by_hash.keys().copied().collect();
        wrapper.param_defaults_normalized = wrapper
            .param_hashes
            .iter()
            .map(|hash| unsafe { wrapper.param_by_hash[hash].normalized_value() })
            .collect();
        wrapper.param_id_hashes = param_map
            .into_keys()
            .map(|id| (hash_param_id(id), id))
            .collect();

        wrapper
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
            x if x == vst3_sys::vst::MediaTypes::kAudio as i32 => 1,
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
        if info.is_null() {
            nih_debug_assert_failure!("Null pointer passed to function");
            return kInvalidArgument;
        }

        match type_ {
            t if t == vst3_sys::vst::MediaTypes::kAudio as i32 => {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_sys::vst::MediaTypes::kAudio as i32;
                info.bus_type = vst3_sys::vst::BusTypes::kMain as i32;
                info.flags = vst3_sys::vst::BusFlags::kDefaultActive as u32;
                match (dir, index) {
                    (d, 0) if d == vst3_sys::vst::BusDirections::kInput as i32 => {
                        info.direction = vst3_sys::vst::BusDirections::kInput as i32;
                        info.channel_count =
                            self.current_bus_config.borrow().num_input_channels as i32;
                        u16strlcpy(&mut info.name, "Input");

                        kResultOk
                    }
                    (d, 0) if d == vst3_sys::vst::BusDirections::kOutput as i32 => {
                        info.direction = vst3_sys::vst::BusDirections::kOutput as i32;
                        info.channel_count =
                            self.current_bus_config.borrow().num_output_channels as i32;
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
        if in_info.is_null() || out_info.is_null() {
            nih_debug_assert_failure!("Null pointer passed to function");
            return kInvalidArgument;
        }

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

impl<P: Plugin> IEditController for Wrapper<P> {
    unsafe fn set_component_state(&self, _state: *mut c_void) -> tresult {
        // We have a single file component, so we don't need to do anything here
        kResultOk
    }

    unsafe fn set_state(&self, state: *mut c_void) -> tresult {
        // We have a single file component, so there's only one `set_state()` function. Unlike C++,
        // Rust allows you to have multiple methods with the same name when they're provided by
        // different treats, but because of the Rust implementation the host may call either of
        // these functions depending on how they're implemented
        IComponent::set_state(self, state)
    }

    unsafe fn get_state(&self, state: *mut c_void) -> tresult {
        // Same for this function
        IComponent::get_state(self, state)
    }

    unsafe fn get_parameter_count(&self) -> i32 {
        // NOTE: We add a bypass parameter ourselves on index `self.param_ids.len()`, so these
        //       indices are all off by one
        self.param_hashes.len() as i32 + 1
    }

    unsafe fn get_parameter_info(
        &self,
        param_index: i32,
        info: *mut vst3_sys::vst::ParameterInfo,
    ) -> tresult {
        if info.is_null() {
            nih_debug_assert_failure!("Null pointer passed to function");
            return kInvalidArgument;
        }

        // Parameter index `self.param_ids.len()` is our own bypass parameter
        if param_index < 0 || param_index > self.param_hashes.len() as i32 {
            return kInvalidArgument;
        }

        *info = std::mem::zeroed();

        let info = &mut *info;
        if param_index == self.param_hashes.len() as i32 {
            info.id = hash_param_id(BYPASS_PARAM_ID);
            u16strlcpy(&mut info.title, "Bypass");
            u16strlcpy(&mut info.short_title, "Bypass");
            u16strlcpy(&mut info.units, "");
            info.step_count = 0;
            info.default_normalized_value = 0.0;
            info.unit_id = vst3_sys::vst::kRootUnitId;
            info.flags = vst3_sys::vst::ParameterFlags::kCanAutomate as i32
                | vst3_sys::vst::ParameterFlags::kIsBypass as i32;
        } else {
            let param_hash = &self.param_hashes[param_index as usize];
            let default_value = &self.param_defaults_normalized[param_index as usize];
            let param_ptr = &self.param_by_hash[param_hash];

            info.id = *param_hash;
            u16strlcpy(&mut info.title, param_ptr.name());
            u16strlcpy(&mut info.short_title, param_ptr.name());
            u16strlcpy(&mut info.units, param_ptr.unit());
            // TODO: Don't forget this when we add enum parameters
            info.step_count = 0;
            info.default_normalized_value = *default_value as f64;
            info.unit_id = vst3_sys::vst::kRootUnitId;
            info.flags = vst3_sys::vst::ParameterFlags::kCanAutomate as i32;
        }

        kResultOk
    }

    unsafe fn get_param_string_by_value(
        &self,
        id: u32,
        value_normalized: f64,
        string: *mut TChar,
    ) -> tresult {
        if string.is_null() {
            nih_debug_assert_failure!("Null pointer passed to function");
            return kInvalidArgument;
        }

        // Somehow there's no length there, so we'll assume our own maximum
        let dest = &mut *(string as *mut [TChar; 128]);

        if id == *BYPASS_PARAM_HASH {
            if value_normalized > 0.5 {
                u16strlcpy(dest, "Bypassed")
            } else {
                u16strlcpy(dest, "Enabled")
            }

            kResultOk
        } else if let Some(param_ptr) = self.param_by_hash.get(&id) {
            u16strlcpy(
                dest,
                &param_ptr.normalized_value_to_string(value_normalized as f32),
            );

            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn get_param_value_by_string(
        &self,
        id: u32,
        string: *const TChar,
        value_normalized: *mut f64,
    ) -> tresult {
        if string.is_null() || value_normalized.is_null() {
            nih_debug_assert_failure!("Null pointer passed to function");
            return kInvalidArgument;
        }

        let string = match U16CStr::from_ptr_str(string as *const u16).to_string() {
            Ok(s) => s,
            Err(_) => return kInvalidArgument,
        };

        if id == *BYPASS_PARAM_HASH {
            let value = match string.as_str() {
                "Bypassed" => 1.0,
                "Enabled" => 0.0,
                _ => return kResultFalse,
            };
            *value_normalized = value;

            kResultOk
        } else if let Some(param_ptr) = self.param_by_hash.get(&id) {
            let value = match param_ptr.string_to_normalized_value(&string) {
                Some(v) => v as f64,
                None => return kResultFalse,
            };
            *value_normalized = value;

            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn normalized_param_to_plain(&self, id: u32, value_normalized: f64) -> f64 {
        if id == *BYPASS_PARAM_HASH {
            value_normalized
        } else if let Some(param_ptr) = self.param_by_hash.get(&id) {
            param_ptr.preview_unnormalized(value_normalized as f32) as f64
        } else {
            0.5
        }
    }

    unsafe fn plain_param_to_normalized(&self, id: u32, plain_value: f64) -> f64 {
        if id == *BYPASS_PARAM_HASH {
            plain_value.clamp(0.0, 1.0)
        } else if let Some(param_ptr) = self.param_by_hash.get(&id) {
            param_ptr.preview_normalized(plain_value as f32) as f64
        } else {
            0.5
        }
    }

    unsafe fn get_param_normalized(&self, id: u32) -> f64 {
        if id == *BYPASS_PARAM_HASH {
            if self.bypass_state.get() {
                1.0
            } else {
                0.0
            }
        } else if let Some(param_ptr) = self.param_by_hash.get(&id) {
            param_ptr.normalized_value() as f64
        } else {
            0.5
        }
    }

    unsafe fn set_param_normalized(&self, id: u32, value: f64) -> tresult {
        if id == *BYPASS_PARAM_HASH {
            self.bypass_state.set(value >= 0.5);

            kResultOk
        } else if let Some(param_ptr) = self.param_by_hash.get(&id) {
            param_ptr.set_normalized_value(value as f32);

            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn set_component_handler(&self, _handler: *mut c_void) -> tresult {
        // TODO: Use this when we add GUI support
        kResultOk
    }

    unsafe fn create_view(&self, _name: vst3_sys::base::FIDString) -> *mut c_void {
        // We currently don't support GUIs
        std::ptr::null_mut()
    }
}

#[VST3(implements(IPluginFactory, IPluginFactory2, IPluginFactory3))]
pub struct Factory<P: Vst3Plugin> {
    /// The exposed plugin's GUID. Instead of generating this, we'll just let the programmer decide
    /// on their own.
    cid: GUID,
    /// The type will be used for constructing plugin instances later.
    _phantom: PhantomData<P>,
}

impl<P: Vst3Plugin> Factory<P> {
    pub fn new() -> Box<Self> {
        Self::allocate(
            GUID {
                data: P::VST3_CLASS_ID,
            },
            PhantomData::default(),
        )
    }
}

impl<P: Vst3Plugin> IPluginFactory for Factory<P> {
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
        if cid.is_null() || obj.is_null() {
            nih_debug_assert_failure!("Null pointer passed to function");
            return kInvalidArgument;
        }

        if *cid != self.cid {
            return kNoInterface;
        }

        *obj = Box::into_raw(Wrapper::<P>::new()) as *mut vst3_sys::c_void;

        kResultOk
    }
}

impl<P: Vst3Plugin> IPluginFactory2 for Factory<P> {
    unsafe fn get_class_info2(
        &self,
        index: i32,
        info: *mut vst3_sys::base::PClassInfo2,
    ) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid = self.cid;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        strlcpy(&mut info.name, P::NAME);
        info.class_flags = 1 << 1; // kSimpleModeSupported
        strlcpy(&mut info.subcategories, P::VST3_CATEGORIES);
        strlcpy(&mut info.vendor, P::VENDOR);
        strlcpy(&mut info.version, P::VERSION);
        strlcpy(&mut info.sdk_version, VST3_SDK_VERSION);

        kResultOk
    }
}

impl<P: Vst3Plugin> IPluginFactory3 for Factory<P> {
    unsafe fn get_class_info_unicode(
        &self,
        index: i32,
        info: *mut vst3_sys::base::PClassInfoW,
    ) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid = self.cid;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        u16strlcpy(&mut info.name, P::NAME);
        info.class_flags = 1 << 1; // kSimpleModeSupported
        strlcpy(&mut info.subcategories, P::VST3_CATEGORIES);
        u16strlcpy(&mut info.vendor, P::VENDOR);
        u16strlcpy(&mut info.version, P::VERSION);
        u16strlcpy(&mut info.sdk_version, VST3_SDK_VERSION);

        kResultOk
    }

    unsafe fn set_host_context(&self, _context: *mut c_void) -> tresult {
        // We don't need to do anything with this
        kResultOk
    }
}

/// Export a VST3 plugin from this library using the provided plugin type.
///
/// TODO: Come up with some way to hae Cargo spit out a VST3 module. Is that possible without a
///       custom per-plugin build script?
#[macro_export]
macro_rules! nih_export_vst3 {
    ($plugin_ty:ty) => {
        #[no_mangle]
        pub extern "system" fn GetPluginFactory() -> *mut ::std::ffi::c_void {
            let factory = ::nih_plug::wrapper::vst3::Factory::<$plugin_ty>::new();

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
