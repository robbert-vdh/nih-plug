use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::plugin_factory::clap_plugin_factory;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::ptr;
use std::sync::Arc;

use super::descriptor::PluginDescriptor;
use super::wrapper::Wrapper;
use crate::plugin::ClapPlugin;

/// The plugin's factory. Initialized using a lazy_static from the entry point's `get_factory()`
/// function. From this point onwards we don't need to generate code with macros anymore.
#[doc(hidden)]
#[repr(C)]
pub struct Factory<P: ClapPlugin> {
    // Keep the vtable as the first field so we can do a simple pointer cast
    pub clap_plugin_factory: clap_plugin_factory,

    plugin_descriptor: PluginDescriptor<P>,
}

impl<P: ClapPlugin> Default for Factory<P> {
    fn default() -> Self {
        Self {
            clap_plugin_factory: clap_plugin_factory {
                get_plugin_count: Some(Self::get_plugin_count),
                get_plugin_descriptor: Some(Self::get_plugin_descriptor),
                create_plugin: Some(Self::create_plugin),
            },
            plugin_descriptor: PluginDescriptor::default(),
        }
    }
}

impl<P: ClapPlugin> Factory<P> {
    unsafe extern "C" fn get_plugin_count(_factory: *const clap_plugin_factory) -> u32 {
        1
    }

    unsafe extern "C" fn get_plugin_descriptor(
        factory: *const clap_plugin_factory,
        index: u32,
    ) -> *const clap_plugin_descriptor {
        let factory = &*(factory as *const Self);

        if index == 0 {
            factory.plugin_descriptor.clap_plugin_descriptor()
        } else {
            ptr::null()
        }
    }

    unsafe extern "C" fn create_plugin(
        factory: *const clap_plugin_factory,
        host: *const clap_host,
        plugin_id: *const c_char,
    ) -> *const clap_plugin {
        let factory = &*(factory as *const Self);

        if !plugin_id.is_null() && CStr::from_ptr(plugin_id) == factory.plugin_descriptor.clap_id()
        {
            // Arc does not have a convenient leak function like Box, so this gets a bit awkward
            // This pointer gets turned into an Arc and its reference count decremented in
            // [Wrapper::destroy()]
            &(*Arc::into_raw(Wrapper::<P>::new(host))).clap_plugin
        } else {
            ptr::null()
        }
    }
}
