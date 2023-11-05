use backtrace::Backtrace;
use std::cmp;
use std::marker::PhantomData;
use std::os::raw::c_char;

use crate::util::permit_alloc;

pub(crate) mod buffer_management;
#[cfg(debug_assertions)]
pub(crate) mod context_checks;

/// The bit that controls flush-to-zero behavior for denormals in 32 and 64-bit floating point
/// numbers on AArch64.
///
/// <https://developer.arm.com/documentation/ddi0595/2021-06/AArch64-Registers/FPCR--Floating-point-Control-Register>
#[cfg(target_arch = "aarch64")]
const AARCH64_FTZ_BIT: u64 = 1 << 24;

#[cfg(all(
    debug_assertions,
    physical_sizefeature = "assert_process_allocs",
    all(windows, target_env = "gnu")
))]
compile_error!("The 'assert_process_allocs' feature does not work correctly in combination with the 'x86_64-pc-windows-gnu' target, see https://github.com/Windfisch/rust-assert-no-alloc/issues/7");

#[cfg(all(debug_assertions, feature = "assert_process_allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// A Rabin fingerprint based string hash for parameter ID strings.
pub fn hash_param_id(id: &str) -> u32 {
    let mut hash: u32 = 0;
    for char in id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(char as u32);
    }

    // In VST3 the last bit is reserved for parameters provided by the host
    // https://developer.steinberg.help/display/VST/Parameters+and+Automation
    hash &= !(1 << 31);

    hash
}

/// The equivalent of the `strlcpy()` C function. Copy `src` to `dest` as a null-terminated
/// C-string. If `dest` does not have enough capacity, add a null terminator at the end to prevent
/// buffer overflows.
pub fn strlcpy(dest: &mut [c_char], src: &str) {
    if dest.is_empty() {
        return;
    }

    let src_bytes: &[u8] = src.as_bytes();
    // NOTE: `c_char` is i8 on x86 based archs, and u8 on AArch64. There this line won't do
    //       anything.
    let src_bytes_signed: &[c_char] = unsafe { &*(src_bytes as *const [u8] as *const [c_char]) };

    // Make sure there's always room for a null terminator
    let copy_len = cmp::min(dest.len() - 1, src.len());
    dest[..copy_len].copy_from_slice(&src_bytes_signed[..copy_len]);
    dest[copy_len] = 0;
}

/// Clamp an input event's timing to the buffer length. Emits a debug assertion failure if it was
/// out of bounds.
#[inline]
pub fn clamp_input_event_timing(timing: u32, total_buffer_len: u32) -> u32 {
    // If `total_buffer_len == 0`, then 0 is a valid timing
    let last_valid_index = total_buffer_len.saturating_sub(1);

    nih_debug_assert!(
        timing <= last_valid_index,
        "Input event is out of bounds, will be clamped to the buffer's size"
    );

    timing.min(last_valid_index)
}

/// Clamp an output event's timing to the buffer length. Emits a debug assertion failure if it was
/// out of bounds.
#[inline]
pub fn clamp_output_event_timing(timing: u32, total_buffer_len: u32) -> u32 {
    let last_valid_index = total_buffer_len.saturating_sub(1);

    nih_debug_assert!(
        timing <= last_valid_index,
        "Output event is out of bounds, will be clamped to the buffer's size"
    );

    timing.min(last_valid_index)
}

/// Set up the logger so that the `nih_*!()` logging and assertion macros log output to a
/// centralized location and panics also get written there. By default this logs to STDERR. If a
/// Windows debugger is attached, then messages will be sent there instead. This uses
/// [NIH-log](https://github.com/robbert-vdh/nih-log). See the readme there for more information.
///
/// In short, NIH-log's behavior can be controlled by setting the `NIH_LOG` environment variable to:
///
/// - `stderr`, in which case the log output always gets written to STDERR.
/// - `windbg` (only on Windows), in which case the output always gets logged using
///   `OutputDebugString()`.
/// - A file path, in which case the output gets appended to the end of that file which will be
///   created if necessary.
pub fn setup_logger() {
    let log_level = if cfg!(debug_assertions) {
        log::LevelFilter::Trace
    } else {
        log::LevelFilter::Info
    };

    let logger_builder = nih_log::LoggerBuilder::new(log_level)
        .filter_module("cosmic_text::buffer")
        .filter_module("cosmic_text::shape")
        .filter_module("selectors::matching");

    // Always show the module in debug builds, makes it clearer where messages are coming from and
    // it helps set up filters
    #[cfg(debug_assertions)]
    let logger_builder = logger_builder.always_show_module_path();

    // In release builds there are some more logging messages from libraries that are not relevant
    // to the end user that can be filtered out
    #[cfg(not(debug_assertions))]
    let logger_builder = logger_builder.filter_module("cosmic_text::font::system::std");

    let logger_set = logger_builder.build_global().is_ok();
    if logger_set {
        log_panics();
    }
}

/// This is copied from same as the `log_panics` crate, but it's wrapped in `permit_alloc()`.
/// Otherwise logging panics will trigger `assert_no_alloc` as this also allocates.
fn log_panics() {
    std::panic::set_hook(Box::new(|info| {
        permit_alloc(|| {
            // All of this is directly copied from `permit_no_alloc`, except that `error!()` became
            // `nih_error!()` and `Shim` has been inlined
            let backtrace = Backtrace::new();

            let thread = std::thread::current();
            let thread = thread.name().unwrap_or("unnamed");

            let msg = match info.payload().downcast_ref::<&'static str>() {
                Some(s) => *s,
                None => match info.payload().downcast_ref::<String>() {
                    Some(s) => &**s,
                    None => "Box<Any>",
                },
            };

            match info.location() {
                Some(location) => {
                    nih_error!(
                        target: "panic", "thread '{}' panicked at '{}': {}:{}\n{:?}",
                        thread,
                        msg,
                        location.file(),
                        location.line(),
                        backtrace
                    );
                }
                None => {
                    nih_error!(
                        target: "panic",
                        "thread '{}' panicked at '{}'\n{:?}",
                        thread,
                        msg,
                        backtrace
                    )
                }
            }
        })
    }));
}

/// A wrapper around the entire process function, including the plugin wrapper parts. This sets up
/// `assert_no_alloc` if needed, while also making sure that things like FTZ are set up correctly if
/// the host has not already done so.
pub fn process_wrapper<T, F: FnOnce() -> T>(f: F) -> T {
    // Make sure FTZ is always enabled, even if the host doesn't do it for us
    let _ftz_guard = ScopedFtz::enable();

    cfg_if::cfg_if! {
        if #[cfg(all(debug_assertions, feature = "assert_process_allocs"))] {
            assert_no_alloc::assert_no_alloc(f)
        } else {
            f()
        }
    }
}

/// Enable the CPU's Flush To Zero flag while this object is in scope. If the flag was not already
/// set, it will be restored to its old value when this gets dropped.
struct ScopedFtz {
    /// Whether FTZ should be disabled again, i.e. if FTZ was not enabled before.
    should_disable_again: bool,
    /// We can't directly implement !Send and !Sync, but this will do the same thing. This object
    /// affects the current thread's floating point registers, so it may only be dropped on the
    /// current thread.
    _send_sync_marker: PhantomData<*const ()>,
}

impl ScopedFtz {
    fn enable() -> Self {
        #[cfg(not(miri))]
        {
            #[cfg(target_feature = "sse")]
            {
                let mode = unsafe { std::arch::x86_64::_MM_GET_FLUSH_ZERO_MODE() };
                let should_disable_again = mode != std::arch::x86_64::_MM_FLUSH_ZERO_ON;
                if should_disable_again {
                    unsafe {
                        std::arch::x86_64::_MM_SET_FLUSH_ZERO_MODE(
                            std::arch::x86_64::_MM_FLUSH_ZERO_ON,
                        )
                    };
                }

                return Self {
                    should_disable_again,
                    _send_sync_marker: PhantomData,
                };
            }

            #[cfg(target_arch = "aarch64")]
            {
                // There are no convient intrinsics to change the FTZ settings on AArch64, so this
                // requires inline assembly:
                // https://developer.arm.com/documentation/ddi0595/2021-06/AArch64-Registers/FPCR--Floating-point-Control-Register
                let mut fpcr: u64;
                unsafe { std::arch::asm!("mrs {}, fpcr", out(reg) fpcr) };

                let should_disable_again = fpcr & AARCH64_FTZ_BIT == 0;
                if should_disable_again {
                    unsafe { std::arch::asm!("msr fpcr, {}", in(reg) fpcr | AARCH64_FTZ_BIT) };
                }

                return Self {
                    should_disable_again,
                    _send_sync_marker: PhantomData,
                };
            }
        }

        #[allow(unreachable_code)] // This is only unreachable if on SSE or aarch64
        Self {
            should_disable_again: false,
            _send_sync_marker: PhantomData,
        }
    }
}

impl Drop for ScopedFtz {
    fn drop(&mut self) {
        #[cfg(not(miri))]
        if self.should_disable_again {
            #[cfg(target_feature = "sse")]
            {
                unsafe {
                    std::arch::x86_64::_MM_SET_FLUSH_ZERO_MODE(
                        std::arch::x86_64::_MM_FLUSH_ZERO_OFF,
                    )
                };
            }

            #[cfg(target_arch = "aarch64")]
            {
                let mut fpcr: u64;
                unsafe { std::arch::asm!("mrs {}, fpcr", out(reg) fpcr) };
                unsafe { std::arch::asm!("msr fpcr, {}", in(reg) fpcr & !AARCH64_FTZ_BIT) };
            }
        }
    }
}

#[cfg(test)]
mod miri {
    use std::ffi::CStr;

    use super::*;

    #[test]
    fn strlcpy_normal() {
        let mut dest = [0; 256];
        strlcpy(&mut dest, "Hello, world!");

        assert_eq!(
            unsafe { CStr::from_ptr(dest.as_ptr()) }.to_str(),
            Ok("Hello, world!")
        );
    }

    #[test]
    fn strlcpy_overflow() {
        let mut dest = [0; 6];
        strlcpy(&mut dest, "Hello, world!");

        assert_eq!(
            unsafe { CStr::from_ptr(dest.as_ptr()) }.to_str(),
            Ok("Hello")
        );
    }
}
