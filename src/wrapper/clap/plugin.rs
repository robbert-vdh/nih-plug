use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status};
use crossbeam::atomic::AtomicCell;
use crossbeam::queue::ArrayQueue;
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::AtomicU32;
use std::thread::{self, ThreadId};

use super::context::WrapperProcessContext;
use super::descriptor::PluginDescriptor;
use crate::event_loop::{EventLoop, MainThreadExecutor, TASK_QUEUE_CAPACITY};
use crate::plugin::{BufferConfig, BusConfig, ClapPlugin};
use crate::NoteEvent;

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
    current_buffer_config: AtomicCell<Option<BufferConfig>>,
    /// The incoming events for the plugin, if `P::ACCEPTS_MIDI` is set.
    ///
    /// TODO: Maybe load these lazily at some point instead of needing to spool them all to this
    ///       queue first
    /// TODO: Read these in the process call.
    input_events: RwLock<VecDeque<NoteEvent>>,
    /// The current latency in samples, as set by the plugin through the [ProcessContext]. uses the
    /// latency extnesion
    ///
    /// TODO: Implement the latency extension.
    pub current_latency: AtomicU32,

    host_callback: HostCallback,
    /// Needs to be boxed because the plugin object is supposed to contain a static reference to
    /// this.
    plugin_descriptor: Box<PluginDescriptor<P>>,

    /// A queue of tasks that still need to be performed. Because CLAP lets the plugin request a
    /// host callback directly, we don't need to use the OsEventLoop we use in our other plugin
    /// implementations. Instead, we'll post tasks to this queue, ask the host to call
    /// [Self::on_main_thread] on the main thread, and then continue to pop tasks off this queue
    /// there until it is empty.
    tasks: ArrayQueue<Task>,
    /// The ID of the main thread. In practice this is the ID of the thread that created this
    /// object.
    ///
    /// TODO: If the host supports the ThreadCheck extension, we should use that instead.
    main_thread_id: ThreadId,
}

/// Send+Sync wrapper around clap_host.
struct HostCallback(*const clap_host);

/// Tasks that can be sent from the plugin to be executed on the main thread in a non-blocking
/// realtime safe way. Instead of using a random thread or the OS' event loop like in the Linux
/// implementation, this uses [clap_host::request_callback()] instead.
#[derive(Debug, Clone)]
pub enum Task {
    /// Inform the host that the latency has changed.
    LatencyChanged,
}

/// Because CLAP has this [clap_host::request_host_callback()] function, we don't need to use
/// `OsEventLoop` and can instead just request a main thread callback directly.
impl<P: ClapPlugin> EventLoop<Task, Plugin<P>> for Plugin<P> {
    fn new_and_spawn(_executor: std::sync::Weak<Self>) -> Self {
        panic!("What are you doing");
    }

    fn do_maybe_async(&self, task: Task) -> bool {
        if self.is_main_thread() {
            unsafe { self.execute(task) };
            true
        } else {
            let success = self.tasks.push(task).is_ok();
            if success {
                // CLAP lets us use the host's event loop instead of having to implement our own
                let host = self.host_callback.0;
                unsafe { ((*host).request_callback)(host) };
            }

            success
        }
    }

    fn is_main_thread(&self) -> bool {
        // TODO: Use the `thread_check::is_main_thread` extension method if that's available
        thread::current().id() == self.main_thread_id
    }
}

impl<P: ClapPlugin> MainThreadExecutor<Task> for Plugin<P> {
    unsafe fn execute(&self, task: Task) {
        todo!("Implement latency changes for CLAP")
    }
}

unsafe impl Send for HostCallback {}
unsafe impl Sync for HostCallback {}

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
            input_events: RwLock::new(VecDeque::with_capacity(512)),
            current_latency: AtomicU32::new(0),

            host_callback: HostCallback(host_callback),
            plugin_descriptor,

            tasks: ArrayQueue::new(TASK_QUEUE_CAPACITY),
            main_thread_id: thread::current().id(),
        }
    }

    fn make_process_context(&self) -> WrapperProcessContext<'_, P> {
        WrapperProcessContext {
            plugin: self,
            input_events_guard: self.input_events.write(),
        }
    }

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
        _min_frames_count: u32,
        max_frames_count: u32,
    ) -> bool {
        let plugin = &*(plugin as *const Self);

        let bus_config = plugin.current_bus_config.load();
        let buffer_config = BufferConfig {
            sample_rate: sample_rate as f32,
            max_buffer_size: max_frames_count,
        };

        // TODO: Reset smoothers

        if plugin.plugin.write().initialize(
            &bus_config,
            &buffer_config,
            &mut plugin.make_process_context(),
        ) {
            // TODO: Allocate buffer slices

            // Also store this for later, so we can reinitialize the plugin after restoring state
            plugin.current_buffer_config.store(Some(buffer_config));

            true
        } else {
            false
        }
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
        let plugin = &*(plugin as *const Self);

        // [Self::do_maybe_async] posts a task to the queue and asks the host to call this function
        // on the main thread, so once that's done we can just handle all requests here
        while let Some(task) = plugin.tasks.pop() {
            plugin.execute(task);
        }
    }
}
