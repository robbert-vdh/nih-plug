use clap_sys::plugin_factory::clap_plugin_factory;
use std::marker::PhantomData;
use std::mem;

use crate::ClapPlugin;

/// The plugin's factory. Initialized using a lazy_static from the entry poiunt's `get_factory()`
/// function. From this point onwards we don't need to generate code with macros anymore.
#[doc(hidden)]
pub struct Factory<P: ClapPlugin> {
    /// The type will be used for constructing plugin instances later.
    _phantom: PhantomData<P>,
}

impl<P: ClapPlugin> Default for Factory<P> {
    fn default() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<P: ClapPlugin> Factory<P> {
    pub fn clap_plugin_factory(&self) -> clap_plugin_factory {
        clap_plugin_factory {
            get_plugin_count: todo!(),
            get_plugin_descriptor: todo!(),
            create_plugin: todo!(),
        }
    }
}
