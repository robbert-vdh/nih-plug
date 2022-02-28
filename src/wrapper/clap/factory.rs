use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::plugin_factory::clap_plugin_factory;
use clap_sys::version::CLAP_VERSION;
use std::ffi::CString;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::os::raw::c_char;
use std::ptr;

use crate::ClapPlugin;

/// The plugin's factory. Initialized using a lazy_static from the entry poiunt's `get_factory()`
/// function. From this point onwards we don't need to generate code with macros anymore.
#[doc(hidden)]
#[repr(C)]
pub struct Factory<P: ClapPlugin> {
    // Keep the vtable as the first field so we can do a simple pointer cast
    pub clap_plugin_factory: clap_plugin_factory,

    // We need [CString]s for all of `ClapPlugin`'s `&str` fields
    clap_id: CString,
    name: CString,
    vendor: CString,
    url: CString,
    clap_manual_url: CString,
    clap_support_url: CString,
    version: CString,
    clap_description: CString,
    clap_features: Vec<CString>,
    clap_features_ptrs: MaybeUninit<CStrPtrs>,

    /// We only support a single plugin per factory right now, so we'll fill in the plugin
    /// descriptor upfront. We also need to initialize the `CString` fields above first before we
    /// can initialize this plugin descriptor.
    plugin_descriptor: MaybeUninit<clap_plugin_descriptor>,

    /// The type will be used for constructing plugin instances later.
    _phantom: PhantomData<P>,
}

/// Needed for the Send+Sync implementation for lazy_static.
struct CStrPtrs(Vec<*const c_char>);

impl<P: ClapPlugin> Default for Factory<P> {
    fn default() -> Self {
        let mut factory = Self {
            clap_plugin_factory: clap_plugin_factory {
                get_plugin_count: Self::get_plugin_count,
                get_plugin_descriptor: Self::get_plugin_descriptor,
                create_plugin: Self::create_plugin,
            },
            clap_id: CString::new(P::CLAP_ID).expect("`CLAP_ID` contained null bytes"),
            name: CString::new(P::NAME).expect("`NAME` contained null bytes"),
            vendor: CString::new(P::VENDOR).expect("`VENDOR` contained null bytes"),
            url: CString::new(P::URL).expect("`URL` contained null bytes"),
            clap_manual_url: CString::new(P::CLAP_MANUAL_URL)
                .expect("`CLAP_MANUAL_URL` contained null bytes"),
            clap_support_url: CString::new(P::CLAP_SUPPORT_URL)
                .expect("`CLAP_SUPPORT_URL` contained null bytes"),
            version: CString::new(P::VERSION).expect("`VERSION` contained null bytes"),
            clap_description: CString::new(P::CLAP_DESCRIPTION)
                .expect("`CLAP_DESCRIPTION` contained null bytes"),
            clap_features: P::CLAP_FEATURES
                .iter()
                .map(|s| CString::new(*s).expect("`CLAP_FEATURES` contained null bytes"))
                .collect(),
            clap_features_ptrs: MaybeUninit::uninit(),
            plugin_descriptor: MaybeUninit::uninit(),
            _phantom: PhantomData,
        };

        // The keyword list is an environ-like list of char pointers terminated by a null pointer
        let mut clap_features_ptrs: Vec<*const c_char> = factory
            .clap_features
            .iter()
            .map(|feature| feature.as_ptr())
            .collect();
        clap_features_ptrs.push(ptr::null());
        factory
            .clap_features_ptrs
            .write(CStrPtrs(clap_features_ptrs));

        // We couldn't initialize this directly because of all the CStrings
        factory.plugin_descriptor.write(clap_plugin_descriptor {
            clap_version: CLAP_VERSION,
            id: factory.clap_id.as_ptr(),
            name: factory.name.as_ptr(),
            vendor: factory.vendor.as_ptr(),
            url: factory.url.as_ptr(),
            manual_url: factory.clap_manual_url.as_ptr(),
            support_url: factory.clap_support_url.as_ptr(),
            version: factory.version.as_ptr(),
            description: factory.clap_description.as_ptr(),
            features: unsafe { factory.clap_features_ptrs.assume_init_ref() }
                .0
                .as_ptr(),
        });

        factory
    }
}

unsafe impl Send for CStrPtrs {}
unsafe impl Sync for CStrPtrs {}

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
            factory.plugin_descriptor.assume_init_ref()
        } else {
            ptr::null()
        }
    }

    unsafe extern "C" fn create_plugin(
        factory: *const clap_plugin_factory,
        host: *const clap_host,
        plugin_id: *const c_char,
    ) -> *const clap_plugin {
        todo!()
    }
}
