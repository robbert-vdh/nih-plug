use std::cmp;
use std::fs::File;
use std::marker::PhantomData;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};

/// The environment variable for controlling the logging behavior.
const NIH_LOG_ENV: &str = "NIH_LOG";

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

/// Set up the logger so that the `nih_*!()` logging and assertion macros log output to a
/// centralized location and panics also get written there. By default this logs to STDERR unless
/// the user is running Windows and a debugger has been attached, in which case
/// `OutputDebugString()` will be used instead.
///
/// The logger's behavior can be controlled by setting the `NIH_LOG` environment variable to:
///
/// - `stderr`, in which case the log output always gets written to STDERR.
/// - `windbg` (only on Windows), in which case the output always gets logged using
///   `OutputDebugString()`.
/// - A file path, in which case the output gets appended to the end of that file which will be
///   created if necessary.
///
/// # TODO
///
/// Document `NIH_LOG` in an easier to find place.
pub fn setup_logger() {
    // `win_dbg_logger` has no way to let us know that the logger has already been set up, so we'll
    // need to do it this way
    static LOGGER_SET_UP: AtomicBool = AtomicBool::new(false);
    if LOGGER_SET_UP
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let nih_log_env = std::env::var(NIH_LOG_ENV);
    let nih_log_env_str = nih_log_env.as_deref().unwrap_or("");

    #[cfg(target_os = "windows")]
    if nih_log_env_str.eq_ignore_ascii_case("windbg") || win_dbg_logger::is_debugger_present() {
        win_dbg_logger::init();
        log_panics::init();
        return;
    }

    // If opening the file fails, then we'll log to STDERR anyways, hence this closure
    let logger_config = simplelog::ConfigBuilder::new()
        .set_thread_mode(simplelog::ThreadLogMode::Both)
        .build();
    let init_stderr_logger = || {
        simplelog::TermLogger::init(
            simplelog::LevelFilter::Trace,
            logger_config.clone(),
            simplelog::TerminalMode::Stderr,
            simplelog::ColorChoice::Auto,
        )
    };

    // If the logger has already been set up outside of this function then that won't cause any
    // problems, so we can ignore the results here
    if nih_log_env_str.eq_ignore_ascii_case("stderr") || nih_log_env_str.is_empty() {
        let _ = init_stderr_logger();
    } else {
        let file = File::options()
            .append(true)
            .create(true)
            .open(nih_log_env_str);
        match file {
            Ok(file) => {
                let _ = simplelog::WriteLogger::init(
                    simplelog::LevelFilter::Trace,
                    logger_config,
                    file,
                );
            }
            Err(err) => {
                let _ = init_stderr_logger();
                nih_debug_assert_failure!("Could not open '{}': {:?}", nih_log_env_str, err);
            }
        }
    }

    log_panics::init();
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
        // TODO: Implement this for AArch64/neon
        cfg_if::cfg_if! {
            if #[cfg(target_feature = "sse")] {
                let mode = unsafe { std::arch::x86_64::_MM_GET_FLUSH_ZERO_MODE() };
                if mode != std::arch::x86_64::_MM_FLUSH_ZERO_ON {
                    unsafe { std::arch::x86_64::_MM_SET_FLUSH_ZERO_MODE(std::arch::x86_64::_MM_FLUSH_ZERO_ON) };

                    Self {
                        should_disable_again: true,
                        _send_sync_marker: PhantomData,
                    }
                } else {
                    Self {
                        should_disable_again: false,
                        _send_sync_marker: PhantomData,
                    }
                }
            } else {
                Self {
                    should_disable_again: false,
                    _send_sync_marker: PhantomData,
                }
            }
        }
    }
}

impl Drop for ScopedFtz {
    fn drop(&mut self) {
        if self.should_disable_again {
            cfg_if::cfg_if! {
                if #[cfg(target_feature = "sse")] {
                    unsafe { std::arch::x86_64::_MM_SET_FLUSH_ZERO_MODE(std::arch::x86_64::_MM_FLUSH_ZERO_OFF) };
                }
            };
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
