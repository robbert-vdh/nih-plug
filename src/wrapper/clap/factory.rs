use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::plugin_factory::clap_plugin_factory;
use std::marker::PhantomData;
use std::os::raw::c_char;

use crate::ClapPlugin;

/// The plugin's factory. Initialized using a lazy_static from the entry poiunt's `get_factory()`
/// function. From this point onwards we don't need to generate code with macros anymore.
#[doc(hidden)]
#[repr(C)]
pub struct Factory<P: ClapPlugin> {
    // Keep the vtable as the first field so we can do a simple pointer cast
    pub clap_plugin_factory: clap_plugin_factory,

    /// The type will be used for constructing plugin instances later.
    _phantom: PhantomData<P>,
}

impl<P: ClapPlugin> Default for Factory<P> {
    fn default() -> Self {
        Self {
            clap_plugin_factory: clap_plugin_factory {
                get_plugin_count: Self::get_plugin_count,
                get_plugin_descriptor: Self::get_plugin_descriptor,
                create_plugin: Self::create_plugin,
            },
            _phantom: PhantomData,
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
        todo!()
    }

    unsafe extern "C" fn create_plugin(
        factory: *const clap_plugin_factory,
        host: *const clap_host,
        plugin_id: *const c_char,
    ) -> *const clap_plugin {
        todo!()
    }
}
