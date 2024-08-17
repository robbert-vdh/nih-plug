#[macro_use]
mod util;

mod context;
mod descriptor;
pub mod features;
mod wrapper;

/// Re-export for the macro
pub use self::descriptor::PluginDescriptor;
pub use self::wrapper::Wrapper;
pub use clap_sys::entry::clap_plugin_entry;
pub use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
pub use clap_sys::host::clap_host;
pub use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
pub use clap_sys::version::CLAP_VERSION;

/// Export one or more CLAP plugins from this library using the provided plugin types.
#[macro_export]
macro_rules! nih_export_clap {
    ($($plugin_ty:ty),+) => {
        // Earlier versions used a simple generic struct for this, but because we don't have
        // variadic generics (yet) we can't generate the struct for multiple plugin types without
        // macros. So instead we'll generate the implementation ad-hoc inside of this macro.
        #[doc(hidden)]
        mod clap {
            use $crate::prelude::nih_debug_assert_eq;
            use $crate::wrapper::setup_logger;
            use $crate::wrapper::clap::{PluginDescriptor, Wrapper};
            use $crate::wrapper::clap::{CLAP_PLUGIN_FACTORY_ID, clap_host, clap_plugin, clap_plugin_descriptor, clap_plugin_factory};
            use ::std::collections::HashSet;
            use ::std::ffi::{CStr, c_void};
            use ::std::os::raw::c_char;
            use ::std::sync::{Arc, OnceLock};

            // Because the `$plugin_ty`s are likely defined in the enclosing scope. This works even
            // if the types are not public because this is a child module.
            use super::*;

            const CLAP_PLUGIN_FACTORY: clap_plugin_factory = clap_plugin_factory {
                get_plugin_count: Some(get_plugin_count),
                get_plugin_descriptor: Some(get_plugin_descriptor),
                create_plugin: Some(create_plugin),
            };

            // Sneaky way to get the number of expanded elements
            const PLUGIN_COUNT: usize = [$(stringify!($plugin_ty)),+].len();

            // This is a type erased version of the information stored on the plugin types
            static PLUGIN_DESCRIPTORS: OnceLock<[PluginDescriptor; PLUGIN_COUNT]> = OnceLock::new();

            fn plugin_descriptors() -> &'static [PluginDescriptor; PLUGIN_COUNT] {
                PLUGIN_DESCRIPTORS.get_or_init(|| {
                    let descriptors = [$(PluginDescriptor::for_plugin::<$plugin_ty>()),+];

                    if cfg!(debug_assertions) {
                        let unique_plugin_ids: HashSet<_> = descriptors.iter().map(|d| d.clap_id()).collect();
                        nih_debug_assert_eq!(
                            unique_plugin_ids.len(),
                            descriptors.len(),
                            "Duplicate plugin IDs found in `nih_export_clap!()` call"
                        );
                    }

                    descriptors
                })
            }

            unsafe extern "C" fn get_plugin_count(_factory: *const clap_plugin_factory) -> u32 {
                plugin_descriptors().len() as u32
            }

            unsafe extern "C" fn get_plugin_descriptor (
                _factory: *const clap_plugin_factory,
                index: u32,
            ) -> *const clap_plugin_descriptor  {
                match plugin_descriptors().get(index as usize) {
                    Some(descriptor) => descriptor.clap_plugin_descriptor(),
                    None => ::std::ptr::null()
                }
            }

            unsafe extern "C" fn create_plugin (
                _factory: *const clap_plugin_factory,
                host: *const clap_host,
                plugin_id: *const c_char,
            ) -> *const clap_plugin  {
                if plugin_id.is_null() {
                    return ::std::ptr::null();
                }
                let plugin_id_cstr = CStr::from_ptr(plugin_id);

                // This isn't great, but we'll just assume that `$plugin_ids` and the descriptors
                // are in the same order. We also can't directly enumerate over them with an index,
                // which is why we do things the way we do. Otherwise we could have used a tuple
                // instead.
                let descriptors = plugin_descriptors();
                let mut descriptor_idx = 0;
                $({
                    let descriptor = &descriptors[descriptor_idx];
                    if plugin_id_cstr == descriptor.clap_id() {
                        // Arc does not have a convenient leak function like Box, so this gets a bit awkward
                        // This pointer gets turned into an Arc and its reference count decremented in
                        // [Wrapper::destroy()]
                        return (*Arc::into_raw(Wrapper::<$plugin_ty>::new(host)))
                            .clap_plugin
                            .as_ptr();
                    }

                    descriptor_idx += 1;
                })+

                ::std::ptr::null()
            }

            pub extern "C" fn init(_plugin_path: *const c_char) -> bool {
                setup_logger();
                true
            }

            pub extern "C" fn deinit() {}

            pub extern "C" fn get_factory(factory_id: *const c_char) -> *const c_void {
                if !factory_id.is_null() && unsafe { CStr::from_ptr(factory_id) } == CLAP_PLUGIN_FACTORY_ID {
                    &CLAP_PLUGIN_FACTORY as *const _ as *const c_void
                } else {
                    ::std::ptr::null()
                }
            }
        }

        /// The CLAP plugin's entry point.
        #[no_mangle]
        #[used]
        pub static clap_entry: $crate::wrapper::clap::clap_plugin_entry =
            $crate::wrapper::clap::clap_plugin_entry {
                clap_version: $crate::wrapper::clap::CLAP_VERSION,
                init: Some(self::clap::init),
                deinit: Some(self::clap::deinit),
                get_factory: Some(self::clap::get_factory),
            };
    };
}
