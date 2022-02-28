use clap_sys::plugin_factory::clap_plugin_factory;
use std::marker::PhantomData;

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
                get_plugin_descriptor: todo!(),
                create_plugin: todo!(),
            },
            _phantom: PhantomData,
        }
    }
}

impl<P: ClapPlugin> Factory<P> {
    unsafe extern "C" fn get_plugin_count(_factory: *const clap_plugin_factory) -> u32 {
        1
    }
}
