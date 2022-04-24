/// Write something to the logger. This defaults to STDERR unless the user is running Windows and a
/// debugger has been attached, in which case `OutputDebugString()` will be used instead.
///
/// The logger's behavior can be controlled by setting the `NIH_LOG` environment variable to:
///
/// - `stderr`, in which case the log output always gets written to STDERR.
/// - `windbg` (only on Windows), in which case the output always gets logged using
///   `OutputDebugString()`.
/// - A file path, in which case the output gets appended to the end of that file which will be
///   created if necessary.
#[macro_export]
macro_rules! nih_log {
    ($($args:tt)*) => (
        $crate::log::info!($($args)*)
    );
}

/// A `debug_assert!()` analogue that prints the error with line number information instead of
/// panicking.
///
/// TODO: Detect if we're running under a debugger, and trigger a break if we are
#[macro_export]
macro_rules! nih_debug_assert {
    ($cond:expr $(,)?) => (
        if cfg!(debug_assertions) && !$cond {
            $crate::log::debug!(concat!("Debug assertion failed: ", stringify!($cond)));
        }
    );
    ($cond:expr, $format:expr $(, $($args:tt)*)?) => (
        if cfg!(debug_assertions) && !$cond {
            $crate::log::debug!(concat!("Debug assertion failed: ", stringify!($cond), ", ", $format), $($($args)*)?);
        }
    );
}

/// An unconditional debug assertion failure, for if the condition has already been checked
/// elsewhere.
#[macro_export]
macro_rules! nih_debug_assert_failure {
    () => (
        if cfg!(debug_assertions) {
            $crate::log::debug!("Debug assertion failed");
        }
    );
    ($format:expr $(, $($args:tt)*)?) => (
        if cfg!(debug_assertions) {
            $crate::log::debug!(concat!("Debug assertion failed: ", $format), $($($args)*)?);
        }
    );
}

/// A `debug_assert_eq!()` analogue that prints the error with line number information instead of
/// panicking.
#[macro_export]
macro_rules! nih_debug_assert_eq {
    ($left:expr, $right:expr $(,)?) => (
        if cfg!(debug_assertions) && $left != $right {
            $crate::log::debug!(concat!("Debug assertion failed: ", stringify!($left), " != ", stringify!($right)));
        }
    );
    ($left:expr, $right:expr, $format:expr $(, $($args:tt)*)?) => (
        if cfg!(debug_assertions) && $left != $right  {
            $crate::log::debug!(concat!("Debug assertion failed: ", stringify!($left), " != ", stringify!($right), ", ", $format), $($($args)*)?);
        }
    );
}

/// A `debug_assert_ne!()` analogue that prints the error with line number information instead of
/// panicking.
#[macro_export]
macro_rules! nih_debug_assert_ne {
    ($left:expr, $right:expr $(,)?) => (
        if cfg!(debug_assertions) && $left == $right {
            $crate::log::debug!(concat!("Debug assertion failed: ", stringify!($left), " == ", stringify!($right)));
        }
    );
    ($left:expr, $right:expr, $format:expr $(, $($args:tt)*)?) => (
        if cfg!(debug_assertions) && $left == $right  {
            $crate::log::debug!(concat!("Debug assertion failed: ", stringify!($left), " == ", stringify!($right), ", ", $format), $($($args)*)?);
        }
    );
}
