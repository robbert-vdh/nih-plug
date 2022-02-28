use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status};
use crossbeam::atomic::AtomicCell;
use parking_lot::RwLock;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::ptr;

use super::descriptor::PluginDescriptor;
use crate::plugin::{BufferConfig, BusConfig, ClapPlugin};

#[repr(C)]
pub struct Plugin<P: ClapPlugin> {
    // Keep the vtable as the first field so we can do a simple pointer cast
    pub clap_plugin: clap_plugin,

    /// The wrapped plugin instance.
    plugin: RwLock<P>,

    /// The current IO configuration, modified through the `clap_plugin_audio_ports_config`
    /// extension.
    current_bus_config: AtomicCell<BusConfig>,
    /// The current buffer configuration, containing the sample rate and the maximum block size.
    /// Will be set in `clap_plugin::activate()`.
    pub current_buffer_config: AtomicCell<Option<BufferConfig>>,

    host_callback: *const clap_host,
    /// Needs to be boxed because the plugin object is supposed to contain a static reference to
    /// this.
    plugin_descriptor: Box<PluginDescriptor<P>>,
}

impl<P: ClapPlugin> Plugin<P> {
    pub fn new(host_callback: *const clap_host) -> Self {
        let plugin_descriptor = Box::new(PluginDescriptor::default());

        Self {
            clap_plugin: clap_plugin {
                // This needs to live on the heap because the plugin object contains a direct
                // reference to the manifest as a value. We could share this between instances of
                // the plugin using an `Arc`, but this doesn't consume a lot of memory so it's not a
                // huge deal.
                desc: plugin_descriptor.clap_plugin_descriptor(),
                // We already need to use pointer casts in the factory, so might as well continue
                // doing that here
                plugin_data: ptr::null_mut(),
                init: Self::init,
                destroy: Self::destroy,
                activate: Self::activate,
                deactivate: Self::deactivate,
                start_processing: Self::start_processing,
                stop_processing: Self::stop_processing,
                process: Self::process,
                get_extension: Self::get_extension,
                on_main_thread: Self::on_main_thread,
            },

            plugin: RwLock::new(P::default()),
            current_bus_config: AtomicCell::new(BusConfig {
                num_input_channels: P::DEFAULT_NUM_INPUTS,
                num_output_channels: P::DEFAULT_NUM_OUTPUTS,
            }),
            current_buffer_config: AtomicCell::new(None),

            host_callback,
            plugin_descriptor,
        }
    }
}

impl<P: ClapPlugin> Plugin<P> {
    // pub fn make_process_context(&self) -> WrapperProcessContext<'_, P> {
    //     WrapperProcessContext {
    //         plugin: self,
    //         input_events_guard: self.input_events.write(),
    //     }
    // }

    unsafe extern "C" fn init(_plugin: *const clap_plugin) -> bool {
        // We don't need any special initialization
        true
    }

    unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
        Box::from_raw(plugin as *mut Self);
    }

    unsafe extern "C" fn activate(
        plugin: *const clap_plugin,
        sample_rate: f64,
        min_frames_count: u32,
        max_frames_count: u32,
    ) -> bool {
        let plugin = &*(plugin as *const Self);

        let bus_config = plugin.current_bus_config.load();
        let buffer_config = BufferConfig {
            sample_rate: sample_rate as f32,
            max_buffer_size: max_frames_count,
        };

        // TODO: Reset smoothers

        todo!();
        // if plugin.plugin.write().initialize(
        //     &bus_config,
        //     &buffer_config,
        //     plugin.make_process_context(),
        // ) {
        //     // TODO: Allocate buffer slices

        //     // Also store this for later, so we can reinitialize the plugin after restoring state
        //     plugin.current_buffer_config.store(Some(buffer_config));

        //     true
        // } else {
        //     false
        // }
    }

    unsafe extern "C" fn deactivate(_plugin: *const clap_plugin) {
        // We currently don't do anything here
    }

    unsafe extern "C" fn start_processing(_plugin: *const clap_plugin) -> bool {
        // We currently don't do anything here
        true
    }

    unsafe extern "C" fn stop_processing(_plugin: *const clap_plugin) {
        // We currently don't do anything here
    }

    unsafe extern "C" fn process(
        plugin: *const clap_plugin,
        process: *const clap_process,
    ) -> clap_process_status {
        todo!();
    }

    unsafe extern "C" fn get_extension(
        plugin: *const clap_plugin,
        id: *const c_char,
    ) -> *const c_void {
        todo!();
    }

    unsafe extern "C" fn on_main_thread(plugin: *const clap_plugin) {
        todo!();
    }
}
