//! Traits for running background tasks from a [`Plugin`][crate::prelude::Plugin].
//!
//! This should not be confused with the `async` language features and ecosystem in Rust.

/// Something that can run tasks of type [`Task`][Self::Task]. This can be used to defer expensive
/// computations to a background thread. Tasks can be spawned through the methods on the various
/// [`*Context`][crate::context] types.
pub trait AsyncExecutor: Send + Sync {
    /// The type of task this executor can execute. This is usually an enum type. The task type
    /// should not contain any heap allocated data like [`Vec`]s and [`Box`]es.
    type Task;

    /// Run `task` on the current thread. This is usually called from the operating system's main
    /// thread or a similar thread.
    fn execute(&self, task: Self::Task);
}

/// A default implementation for plugins that don't need asynchronous background tasks.
impl AsyncExecutor for () {
    type Task = ();

    fn execute(&self, _task: Self::Task) {}
}
