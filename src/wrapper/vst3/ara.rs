//! ARA 2's VST3 interfaces (ARAVST3.h): the main factory class the plugin factory lists, and the
//! entry points the component answers. Pointers only: the plugin supplies the ARA side
//! ([`crate::prelude::Vst3Plugin::vst3_ara_factory()`] and `vst3_ara_bind()`).

use vst3_com::interfaces::IUnknown;
use vst3_com::{c_void, com_interface};
use vst3_sys as vst3_com;
use vst3_sys::VST3;

/// The plugin factory's class category for it.
pub const ARA_MAIN_FACTORY_CLASS: &str = "ARA Main Factory Class";

#[com_interface("DB2A1669-FAFD-42A5-A82F-864F7B6872EA")]
pub trait IMainFactory: IUnknown {
    unsafe fn get_factory(&self) -> *const c_void;
}

#[com_interface("12814E54-A1CE-4076-82B9-681316950BD6")]
pub trait IPlugInEntryPoint: IUnknown {
    unsafe fn get_factory(&self) -> *const c_void;
    unsafe fn bind_to_document_controller(&self, document_controller: *mut c_void) -> *const c_void;
}

#[com_interface("CD9A5913-C9EB-46D7-96CA-53ADD1DB89F5")]
pub trait IPlugInEntryPoint2: IUnknown {
    unsafe fn bind_to_document_controller_with_roles(&self, document_controller: *mut c_void, known_roles: i32, assigned_roles: i32) -> *const c_void;
}

/// The object the "ARA Main Factory Class" makes: hands out the plugin's `ARAFactory`.
#[VST3(implements(IMainFactory))]
pub struct MainFactory {
    factory: usize,
}

impl MainFactory {
    pub fn new(factory: *const c_void) -> Box<Self> {
        Self::allocate(factory as usize)
    }
}

impl IMainFactory for MainFactory {
    unsafe fn get_factory(&self) -> *const c_void {
        self.factory as *const c_void
    }
}
