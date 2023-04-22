use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem;
use vst3_sys::base::{kInvalidArgument, kResultOk, tresult};
use vst3_sys::base::{IPluginFactory, IPluginFactory2, IPluginFactory3, IUnknown};
use vst3_sys::VST3;

// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;

use super::subcategories::Vst3SubCategory;
use super::util::u16strlcpy;
use super::wrapper::Wrapper;
use crate::prelude::Vst3Plugin;
use crate::wrapper::util::strlcpy;

/// The VST3 SDK version this is roughly based on. The bindings include some VST 3.7 things but not
/// everything, so we'll play it safe.
const VST3_SDK_VERSION: &str = "VST 3.6.14";

#[doc(hidden)]
#[VST3(implements(IPluginFactory, IPluginFactory2, IPluginFactory3))]
pub struct Factory<P: Vst3Plugin> {
    /// The type will be used for constructing plugin instances later.
    _phantom: PhantomData<P>,
}

impl<P: Vst3Plugin> Factory<P> {
    pub fn new() -> Box<Self> {
        Self::allocate(PhantomData::default())
    }
}

impl<P: Vst3Plugin> IPluginFactory for Factory<P> {
    unsafe fn get_factory_info(&self, info: *mut vst3_sys::base::PFactoryInfo) -> tresult {
        *info = mem::zeroed();

        let info = &mut *info;
        strlcpy(&mut info.vendor, P::VENDOR);
        strlcpy(&mut info.url, P::URL);
        strlcpy(&mut info.email, P::EMAIL);
        info.flags = vst3_sys::base::FactoryFlags::kUnicode as i32;

        kResultOk
    }

    unsafe fn count_classes(&self) -> i32 {
        // We don't do shell plugins, and good of an idea having separated components and edit
        // controllers in theory is, few software can use it, and doing that would make our simple
        // microframework a lot less simple
        1
    }

    unsafe fn get_class_info(&self, index: i32, info: *mut vst3_sys::base::PClassInfo) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid.data = P::PLATFORM_VST3_CLASS_ID;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        strlcpy(&mut info.name, P::NAME);

        kResultOk
    }

    unsafe fn create_instance(
        &self,
        cid: *const vst3_sys::IID,
        iid: *const vst3_sys::IID,
        obj: *mut *mut vst3_sys::c_void,
    ) -> tresult {
        check_null_ptr!(cid, obj);

        if (*cid).data != P::PLATFORM_VST3_CLASS_ID {
            return kInvalidArgument;
        }

        let wrapper = Wrapper::<P>::new();

        // 99.999% of the times `iid` will be that of `IComponent`, but the caller is technically
        // allowed to create an object for any support interface. We don't have a way to check
        // whether our plugin supports the interface without creating it, but since the odds that a
        // caller will create an object with an interface we don't support are basically zero this
        // is not a problem.
        let result = wrapper.query_interface(iid, obj);
        if result == kResultOk {
            // This is a bit awkward now but if the cast succeeds we need to get rid of the
            // reference from the `wrapper` binding. The VST3 query interface always increments the
            // reference count and returns an owned reference, so we need to explicitly release the
            // reference from `wrapper` and leak the `Box` so the wrapper doesn't automatically get
            // deallocated when this function returns (`Box` is an incorrect choice on vst3-sys'
            // part, it should have used a `VstPtr` instead).
            wrapper.release();
            Box::leak(wrapper);
        }

        result
    }
}

impl<P: Vst3Plugin> IPluginFactory2 for Factory<P> {
    unsafe fn get_class_info2(
        &self,
        index: i32,
        info: *mut vst3_sys::base::PClassInfo2,
    ) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid.data = P::PLATFORM_VST3_CLASS_ID;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        strlcpy(&mut info.name, P::NAME);
        info.class_flags = 1 << 1; // kSimpleModeSupported
        strlcpy(&mut info.subcategories, &make_subcategories_string::<P>());
        strlcpy(&mut info.vendor, P::VENDOR);
        strlcpy(&mut info.version, P::VERSION);
        strlcpy(&mut info.sdk_version, VST3_SDK_VERSION);

        kResultOk
    }
}

impl<P: Vst3Plugin> IPluginFactory3 for Factory<P> {
    unsafe fn get_class_info_unicode(
        &self,
        index: i32,
        info: *mut vst3_sys::base::PClassInfoW,
    ) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }

        *info = mem::zeroed();

        let info = &mut *info;
        info.cid.data = P::PLATFORM_VST3_CLASS_ID;
        info.cardinality = vst3_sys::base::ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        u16strlcpy(&mut info.name, P::NAME);
        info.class_flags = 1 << 1; // kSimpleModeSupported
        strlcpy(&mut info.subcategories, &make_subcategories_string::<P>());
        u16strlcpy(&mut info.vendor, P::VENDOR);
        u16strlcpy(&mut info.version, P::VERSION);
        u16strlcpy(&mut info.sdk_version, VST3_SDK_VERSION);

        kResultOk
    }

    unsafe fn set_host_context(&self, _context: *mut c_void) -> tresult {
        // We don't need to do anything with this
        kResultOk
    }
}

fn make_subcategories_string<P: Vst3Plugin>() -> String {
    // No idea if any hosts do something with OnlyRT, but it's part of VST3's example categories
    // list. Plugins should not be adding this feature manually
    nih_debug_assert!(!P::VST3_SUBCATEGORIES.contains(&Vst3SubCategory::Custom("OnlyRT")));
    let subcategory_string = P::VST3_SUBCATEGORIES
        .iter()
        .map(Vst3SubCategory::as_str)
        .collect::<Vec<&str>>()
        .join("|");

    let subcategory_string = if P::HARD_REALTIME_ONLY {
        format!("{subcategory_string}|OnlyRT")
    } else {
        subcategory_string
    };
    nih_debug_assert!(subcategory_string.len() <= 127);

    subcategory_string
}
