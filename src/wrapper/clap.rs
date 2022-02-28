use crate::ClapPlugin;

/// Re-export for the wrapper.
pub use ::clap_sys::entry::clap_plugin_entry;
pub use ::clap_sys::version::CLAP_VERSION;

/// Export a CLAP plugin from this library using the provided plugin type.
#[macro_export]
macro_rules! nih_export_clap {
    ($plugin_ty:ty) => {
        // We need a function pointer to a [wrapper::get_factory()] that creates a factory for
        // `$plugin_ty`, so we need to generate the function inside of this macro
        #[doc(hidden)]
        mod clap {
            // We don't need any special initialization or deinitialization handling
            pub fn init(_plugin_path: *const ::std::os::raw::c_char) -> bool {
                eprintln!("Init!");

                true
            }

            pub fn deinit() {
                eprintln!("Deinit!");
            }

            pub fn get_factory(
                factory_id: *const ::std::os::raw::c_char,
            ) -> *const ::std::ffi::c_void {
                eprintln!("get factory!! {factory_id:#?}");

                std::ptr::null()
            }
        }

        #[no_mangle]
        #[used]
        pub static clap_entry: ::nih_plug::wrapper::clap::clap_plugin_entry =
            ::nih_plug::wrapper::clap::clap_plugin_entry {
                clap_version: ::nih_plug::wrapper::clap::CLAP_VERSION,
                // These function pointers are marked as `extern "C"`but there's no reason why the symbols
                // would need to be exported, so we need these transmutes
                init: unsafe {
                    ::std::mem::transmute(clap::init as fn(*const ::std::os::raw::c_char) -> bool)
                },
                deinit: unsafe { ::std::mem::transmute(clap::deinit as fn()) },
                get_factory: unsafe {
                    ::std::mem::transmute(
                        clap::get_factory
                            as fn(*const ::std::os::raw::c_char) -> *const ::std::ffi::c_void,
                    )
                },
            };
    };
}
