// nih-plug: plugins, but rewritten in Rust
// Copyright (C) 2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use parking_lot::RwLock;
use raw_window_handle::RawWindowHandle;
use std::any::Any;
use std::ffi::{c_void, CStr};
use std::mem;
use std::sync::Arc;
use vst3_sys::base::{kInvalidArgument, kResultFalse, kResultOk, tresult, TBool};
use vst3_sys::gui::IPlugView;
use vst3_sys::VST3;

use super::inner::WrapperInner;
use crate::plugin::{Editor, Plugin};
use crate::ParentWindowHandle;

// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;

// Window handle type constants missing from vst3-sys
#[allow(unused)]
const VST3_PLATFORM_HWND: &str = "HWND";
#[allow(unused)]
const VST3_PLATFORM_HIVIEW: &str = "HIView";
#[allow(unused)]
const VST3_PLATFORM_NSVIEW: &str = "NSView";
#[allow(unused)]
const VST3_PLATFORM_UIVIEW: &str = "UIView";
#[allow(unused)]
const VST3_PLATFORM_X11_WINDOW: &str = "X11EmbedWindowID";

/// The plugin's [IPlugView] instance created in [IEditController::create_view] if `P` has an
/// editor. This is managed separately so the lifetime bounds match up.
#[VST3(implements(IPlugView))]
pub(crate) struct WrapperView<P: Plugin> {
    inner: Arc<WrapperInner<P>>,
    editor: Arc<dyn Editor>,
    editor_handle: RwLock<Option<Box<dyn Any>>>,
}

impl<P: Plugin> WrapperView<P> {
    pub fn new(inner: Arc<WrapperInner<P>>, editor: Arc<dyn Editor>) -> Box<Self> {
        Self::allocate(inner, editor, RwLock::new(None))
    }
}

impl<P: Plugin> IPlugView for WrapperView<P> {
    #[cfg(all(target_family = "unix", not(target_os = "macos")))]
    unsafe fn is_platform_type_supported(&self, type_: vst3_sys::base::FIDString) -> tresult {
        let type_ = CStr::from_ptr(type_);
        match type_.to_str() {
            Ok(type_) if type_ == VST3_PLATFORM_X11_WINDOW => kResultOk,
            _ => {
                nih_debug_assert_failure!("Invalid window handle type: {:?}", type_);
                kResultFalse
            }
        }
    }

    #[cfg(all(target_os = "macos"))]
    unsafe fn is_platform_type_supported(&self, type_: vst3_sys::base::FIDString) -> tresult {
        let type_ = CStr::from_ptr(type_);
        match type_.to_str() {
            Ok(type_) if type_ == VST3_PLATFORM_NSVIEW => kResultOk,
            _ => {
                nih_debug_assert_failure!("Invalid window handle type: {:?}", type_);
                kResultFalse
            }
        }
    }

    #[cfg(all(target_os = "windows"))]
    unsafe fn is_platform_type_supported(&self, type_: vst3_sys::base::FIDString) -> tresult {
        let type_ = CStr::from_ptr(type_);
        match type_.to_str() {
            Ok(type_) if type_ == VST3_PLATFORM_HWND => kResultOk,
            _ => {
                nih_debug_assert_failure!("Invalid window handle type: {:?}", type_);
                kResultFalse
            }
        }
    }

    unsafe fn attached(&self, parent: *mut c_void, type_: vst3_sys::base::FIDString) -> tresult {
        let mut editor_handle = self.editor_handle.write();
        if editor_handle.is_none() {
            let type_ = CStr::from_ptr(type_);
            let handle = match type_.to_str() {
                #[cfg(all(target_family = "unix", not(target_os = "macos")))]
                Ok(type_) if type_ == VST3_PLATFORM_X11_WINDOW => {
                    let mut handle = raw_window_handle::unix::XcbHandle::empty();
                    handle.window = parent as usize as u32;
                    RawWindowHandle::Xcb(handle)
                }
                #[cfg(all(target_os = "macos"))]
                Ok(type_) if type_ == VST3_PLATFORM_NSVIEW => {
                    let mut handle = raw_window_handle::macos::MacOSHandle::empty();
                    handle.ns_view = parent;
                    RawWindowHandle::MacOS(handle)
                }
                #[cfg(all(target_os = "windows"))]
                Ok(type_) if type_ == VST3_PLATFORM_HWND => {
                    let mut handle = raw_window_handle::windows::WindowsHandle::empty();
                    handle.hwnd = parent;
                    RawWindowHandle::Windows(handle)
                }
                _ => {
                    nih_debug_assert_failure!("Unknown window handle type: {:?}", type_);
                    return kInvalidArgument;
                }
            };

            *editor_handle = Some(
                self.editor
                    .spawn(ParentWindowHandle { handle }, self.inner.clone()),
            );
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn removed(&self) -> tresult {
        let mut editor_handle = self.editor_handle.write();
        if editor_handle.is_some() {
            *editor_handle = None;
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn on_wheel(&self, _distance: f32) -> tresult {
        // We'll let the plugin use the OS' input mechamisms because not all DAWs (or very few
        // actually) implement these functions
        kResultOk
    }

    unsafe fn on_key_down(
        &self,
        _key: vst3_sys::base::char16,
        _key_code: i16,
        _modifiers: i16,
    ) -> tresult {
        kResultOk
    }

    unsafe fn on_key_up(
        &self,
        _key: vst3_sys::base::char16,
        _key_code: i16,
        _modifiers: i16,
    ) -> tresult {
        kResultOk
    }

    unsafe fn get_size(&self, size: *mut vst3_sys::gui::ViewRect) -> tresult {
        check_null_ptr!(size);

        *size = mem::zeroed();

        let (width, height) = self.editor.size();
        let size = &mut *size;
        size.left = 0;
        size.right = width as i32;
        size.top = 0;
        size.bottom = height as i32;

        kResultOk
    }

    unsafe fn on_size(&self, _new_size: *mut vst3_sys::gui::ViewRect) -> tresult {
        // TODO: Implement resizing
        kResultOk
    }

    unsafe fn on_focus(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn set_frame(&self, _frame: *mut c_void) -> tresult {
        // TODO: Implement resizing. We don't implement that right now, so we also don't need the
        //       plug frame.
        kResultOk
    }

    unsafe fn can_resize(&self) -> tresult {
        // TODO: Implement resizing
        kResultFalse
    }

    unsafe fn check_size_constraint(&self, rect: *mut vst3_sys::gui::ViewRect) -> tresult {
        check_null_ptr!(rect);

        // TODO: Add this with the resizing
        if (*rect).right - (*rect).left > 0 && (*rect).bottom - (*rect).top > 0 {
            kResultOk
        } else {
            kResultFalse
        }
    }
}
