//! Utilities for building a VST3 factory.
//!
//! In order to support exporting multiple VST3 plugins from a single library a lot of functionality
//! had to be moved to the `nih_export_vst3!()` macro. Because working in macro-land can be both
//! frustrating and error prone, most code that does not specifically depend on all of the exposed
//! plugin types was moved back to this module so it can be compiled and type checked as normal.

use vst3_sys::base::{
    ClassCardinality, FactoryFlags, PClassInfo, PClassInfo2, PClassInfoW, PFactoryInfo,
};

use super::subcategories::Vst3SubCategory;
use crate::prelude::Vst3Plugin;
use crate::wrapper::util::strlcpy;
use crate::wrapper::vst3::util::u16strlcpy;

/// The VST3 SDK version this is roughly based on. The bindings include some VST 3.7 things but not
/// everything, so we'll play it safe.
pub const VST3_SDK_VERSION: &str = "VST 3.6.14";

/// The information queried about a plugin through the `IPluginFactory*` methods. Stored in a
/// separate struct so it can be type erased and stored in an array.
#[derive(Debug)]
pub struct PluginInfo {
    pub cid: &'static [u8; 16],
    pub name: &'static str,
    pub subcategories: String,
    pub vendor: &'static str,
    pub version: &'static str,

    // These are used for the factory's own info struct
    pub url: &'static str,
    pub email: &'static str,
}

impl PluginInfo {
    pub fn for_plugin<P: Vst3Plugin>() -> PluginInfo {
        PluginInfo {
            cid: &P::PLATFORM_VST3_CLASS_ID,
            name: P::NAME,
            subcategories: make_subcategories_string::<P>(),
            vendor: P::VENDOR,
            version: P::VERSION,
            url: P::URL,
            email: P::EMAIL,
        }
    }

    /// Fill a [`PFactoryInfo`] struct with the information from this library. Used in
    /// `IPluginFactory`.
    pub fn create_factory_info(&self) -> PFactoryInfo {
        let mut info: PFactoryInfo = unsafe { std::mem::zeroed() };
        strlcpy(&mut info.vendor, self.vendor);
        strlcpy(&mut info.url, self.url);
        strlcpy(&mut info.email, self.email);
        info.flags = FactoryFlags::kUnicode as i32;

        info
    }

    /// Fill a [`PClassInfo`] struct with the information from this library. Used in
    /// `IPluginFactory`.
    pub fn create_class_info(&self) -> PClassInfo {
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        info.cid.data = *self.cid;
        info.cardinality = ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        strlcpy(&mut info.name, self.name);

        info
    }

    /// Fill a [`PClassInfo2`] struct with the information from this library. Used in
    /// `IPluginFactory2`.
    pub fn create_class_info_2(&self) -> PClassInfo2 {
        let mut info: PClassInfo2 = unsafe { std::mem::zeroed() };
        info.cid.data = *self.cid;
        info.cardinality = ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        strlcpy(&mut info.name, self.name);
        info.class_flags = 1 << 1; // kSimpleModeSupported
        strlcpy(&mut info.subcategories, &self.subcategories);
        strlcpy(&mut info.vendor, self.vendor);
        strlcpy(&mut info.version, self.version);
        strlcpy(&mut info.sdk_version, VST3_SDK_VERSION);

        info
    }

    /// Fill a [`PClassInfoW`] struct with the information from this library. Used in
    /// `IPluginFactory3`.
    pub fn create_class_info_unicode(&self) -> PClassInfoW {
        let mut info: PClassInfoW = unsafe { std::mem::zeroed() };
        info.cid.data = *self.cid;
        info.cardinality = ClassCardinality::kManyInstances as i32;
        strlcpy(&mut info.category, "Audio Module Class");
        u16strlcpy(&mut info.name, self.name);
        info.class_flags = 1 << 1; // kSimpleModeSupported
        strlcpy(&mut info.subcategories, &self.subcategories);
        u16strlcpy(&mut info.vendor, self.vendor);
        u16strlcpy(&mut info.version, self.version);
        u16strlcpy(&mut info.sdk_version, VST3_SDK_VERSION);

        info
    }
}

/// Build a pipe separated subcategories string for a VST3 plugin.
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
