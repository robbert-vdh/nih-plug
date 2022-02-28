use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use parking_lot::RwLock;

use crate::plugin::ClapPlugin;

#[repr(C)]
pub struct Plugin<P: ClapPlugin> {
    // Keep the vtable as the first field so we can do a simple pointer cast
    pub clap_plugin: clap_plugin,

    /// The wrapped plugin instance.
    plugin: RwLock<P>,

    host_callback: *const clap_host,
}

impl<P: ClapPlugin> Plugin<P> {
    pub fn new(host_callback: *const clap_host) -> Self {
        Self {
            clap_plugin: clap_plugin {
                desc: todo!(),
                plugin_data: todo!(),
                init: todo!(),
                destroy: todo!(),
                activate: todo!(),
                deactivate: todo!(),
                start_processing: todo!(),
                stop_processing: todo!(),
                process: todo!(),
                get_extension: todo!(),
                on_main_thread: todo!(),
            },
            plugin: RwLock::new(P::default()),
            host_callback,
        }
    }
}
