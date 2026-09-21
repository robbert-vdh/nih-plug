#[macro_use]
mod util;

mod context;
mod factory;
mod inner;
mod note_expressions;
mod param_units;
pub mod subcategories;
mod view;
mod wrapper;

/// Re-export for the wrapper.
pub use factory::PluginInfo;
pub use vst3;
pub use wrapper::Wrapper;

/// Export one or more VST3 plugins from this library using the provided plugin types. The first
/// plugin's vendor information is used for the factory's information.
#[macro_export]
macro_rules! nih_export_vst3 {
    ($($plugin_ty:ty),+) => {
        // Earlier versions used a simple generic struct for this, but because we don't have
        // variadic generics (yet) we can't generate the struct for multiple plugin types without
        // macros. So instead we'll generate the implementation ad-hoc inside of this macro.
        #[doc(hidden)]
        mod vst3 {
            use ::std::collections::HashSet;
            use ::std::ffi::c_void;

            // `vst3_sys` is imported from the VST3 wrapper module
            use $crate::wrapper::vst3::{PluginInfo, Wrapper};
            use $crate::wrapper::vst3::vst3::Steinberg::{kInvalidArgument, kResultOk, tresult, int32, FIDString, TUID};
            use $crate::wrapper::vst3::vst3::Steinberg::{
                PFactoryInfo_::FactoryFlags_, IPluginFactory, IPluginFactory2, IPluginFactory3, FUnknown,
                PClassInfo, PClassInfo2, PClassInfoW, PFactoryInfo, IPluginFactoryTrait, IPluginFactory2Trait, IPluginFactory3Trait,
            };
            use $crate::wrapper::vst3::vst3::{Class, ComWrapper};

            // Because the `$plugin_ty`s are likely defined in the enclosing scope. This works even
            // if the types are not public because this is a child module.
            use super::*;

            // Sneaky way to get the number of expanded elements
            const PLUGIN_COUNT: usize = [$(stringify!($plugin_ty)),+].len();

            #[doc(hidden)]
            pub struct Factory {
                // This is a type erased version of the information stored on the plugin types
                plugin_infos: [PluginInfo; PLUGIN_COUNT],
            }

            impl Class for Factory {
                type Interfaces = (IPluginFactory, IPluginFactory2, IPluginFactory3);
            }

            impl Factory {
                pub fn new() -> Self {
                    let plugin_infos = [$(PluginInfo::for_plugin::<$plugin_ty>()),+];

                    if cfg!(debug_assertions) {
                        let unique_cids: HashSet<[u8; 16]> = plugin_infos.iter().map(|d| *d.cid).collect();
                        $crate::nih_debug_assert_eq!(
                            unique_cids.len(),
                            plugin_infos.len(),
                            "Duplicate VST3 class IDs found in `nih_export_vst3!()` call"
                        );
                    }

                    Factory { plugin_infos }
                }
            }

            impl IPluginFactoryTrait for Factory {
                unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
                    if info.is_null() {
                        return kInvalidArgument;
                    }

                    // We'll use the first plugin's info for this
                    *info = self.plugin_infos[0].create_factory_info();

                    kResultOk
                }

                unsafe fn countClasses(&self) -> int32 {
                    self.plugin_infos.len() as i32
                }

                unsafe fn getClassInfo(&self, index: int32, info: *mut PClassInfo) -> tresult {
                    if index < 0 || index >= self.plugin_infos.len() as i32 {
                        return kInvalidArgument;
                    }

                    *info = self.plugin_infos[index as usize].create_class_info();

                    kResultOk
                }

                unsafe fn createInstance(
                    &self,
                    cid: FIDString,
                    iid: FIDString,
                    obj: *mut *mut c_void,
                ) -> tresult {
                    // Can't use `check_null_ptr!()` here without polluting NIH-plug's general
                    // exports
                    if cid.is_null() || obj.is_null() {
                        return kInvalidArgument;
                    }

                    let cid = &*(cid as *const [u8; 16]);

                    // This is a poor man's way of treating `$plugin_ty` like an indexable array.
                    // Assuming `self.plugin_infos` is in the same order, we can simply check all of
                    // the registered plugin CIDs for matches using an unrolled loop.
                    let mut plugin_idx = 0;
                    $({
                        let plugin_info = &self.plugin_infos[plugin_idx];
                        if cid == plugin_info.cid {
                            let wrapper = ComWrapper::new(Wrapper::<$plugin_ty>::new());
                            let unknown = wrapper.as_com_ref::<FUnknown>().unwrap();
                            let ptr = unknown.as_ptr();
                            return ((*(*ptr).vtbl).queryInterface)(ptr, iid as *const TUID, obj);
                        }

                        plugin_idx += 1;
                    })+

                    kInvalidArgument
                }
            }

            impl IPluginFactory2Trait for Factory {
                unsafe fn getClassInfo2(&self, index: int32, info: *mut PClassInfo2) -> tresult {
                    if index < 0 || index >= self.plugin_infos.len() as i32 {
                        return kInvalidArgument;
                    }

                    *info = self.plugin_infos[index as usize].create_class_info_2();

                    kResultOk
                }
            }

            impl IPluginFactory3Trait for Factory {
                unsafe fn getClassInfoUnicode(
                    &self,
                    index: int32,
                    info: *mut PClassInfoW,
                ) -> tresult {
                    if index < 0 || index >= self.plugin_infos.len() as i32 {
                        return kInvalidArgument;
                    }

                    *info = self.plugin_infos[index as usize].create_class_info_unicode();

                    kResultOk
                }

                unsafe fn setHostContext(&self, _context: *mut FUnknown) -> tresult {
                    // We don't need to do anything with this
                    kResultOk
                }
            }
        }

        /// The VST3 plugin factory entry point.
        #[no_mangle]
        pub extern "system" fn GetPluginFactory() -> *mut ::std::ffi::c_void {
            use $crate::wrapper::vst3::vst3::{ComWrapper, Steinberg::IPluginFactory};

            ComWrapper::new(self::vst3::Factory::new())
                .to_com_ptr::<IPluginFactory>()
                .unwrap()
                .into_raw() as *mut ::std::ffi::c_void
        }

        // These two entry points are used on Linux, and they would theoretically also be used on
        // the BSDs:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/c3948deb407bdbff89de8fb6ab8500ea4df9d6d9/source/main/linuxmain.cpp#L47-L52
        #[allow(missing_docs)]
        #[no_mangle]
        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        pub extern "C" fn ModuleEntry(_lib_handle: *mut ::std::ffi::c_void) -> bool {
            $crate::wrapper::setup_logger();
            true
        }

        #[allow(missing_docs)]
        #[no_mangle]
        #[cfg(all(target_family = "unix", not(target_os = "macos")))]
        pub extern "C" fn ModuleExit() -> bool {
            true
        }

        // These two entry points are used on macOS:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/bc459feee68803346737901471441fd4829ec3f9/source/main/macmain.cpp#L60-L61
        #[allow(missing_docs)]
        #[no_mangle]
        #[cfg(target_os = "macos")]
        pub extern "C" fn bundleEntry(_lib_handle: *mut ::std::ffi::c_void) -> bool {
            $crate::wrapper::setup_logger();
            true
        }

        #[allow(missing_docs)]
        #[no_mangle]
        #[cfg(target_os = "macos")]
        pub extern "C" fn bundleExit() -> bool {
            true
        }

        // And these two entry points are used on Windows:
        // https://github.com/steinbergmedia/vst3_public_sdk/blob/bc459feee68803346737901471441fd4829ec3f9/source/main/dllmain.cpp#L59-L60
        #[allow(missing_docs)]
        #[no_mangle]
        #[cfg(target_os = "windows")]
        pub extern "system" fn InitDll() -> bool {
            $crate::wrapper::setup_logger();
            true
        }

        #[allow(missing_docs)]
        #[no_mangle]
        #[cfg(target_os = "windows")]
        pub extern "system" fn ExitDll() -> bool {
            true
        }
    };
}
