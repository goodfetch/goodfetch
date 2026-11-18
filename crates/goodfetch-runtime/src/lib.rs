//! The one place a Goodfetch Tokio runtime is built (ADRs 0060 and 0081):
//! each executable's `main`, each SDK crate's Connection, the test helpers
//! and the bench targets take their builder here, so this crate alone
//! carries the `#[expect]` of `disallowed-methods` for building one, and a
//! runtime built anywhere else fails Clippy. Engine code never builds one.
//!
//! [`current_thread`] and [`multi_thread`] hand out Tokio's builders, which
//! the caller sets up further (a test helper pauses the clock, a test bounds
//! the blocking pool); [`named`] builds the production multi-thread runtime
//! whose two kinds of threads carry names of their own, so
//! `/proc/<pid>/task` tells Tokio's workers from its blocking threads.

use std::io;
use std::num::NonZeroUsize;
use std::sync::{Arc, OnceLock};

pub use tokio::runtime::{Builder, Runtime};

/// The name of a [`named`] runtime's worker thread.
pub const WORKER_THREAD: &str = "gf-worker";

/// The name of a thread of a [`named`] runtime's blocking pool.
pub const BLOCKING_THREAD: &str = "gf-blocking";

/// A current-thread runtime's builder, nothing enabled yet.
#[must_use]
pub fn current_thread() -> Builder {
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0081: goodfetch-runtime is the one crate that builds a runtime"
    )]
    Builder::new_current_thread()
}

/// A multi-thread runtime's builder, nothing enabled yet.
#[must_use]
pub fn multi_thread() -> Builder {
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0081: goodfetch-runtime is the one crate that builds a runtime"
    )]
    Builder::new_multi_thread()
}

/// How many threads of each kind a [`named`] runtime has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Threads {
    /// Its workers, or Tokio's default, one per core, when `None`.
    pub workers: Option<NonZeroUsize>,
    /// The cap on its blocking pool.
    pub blocking: NonZeroUsize,
}

/// A multi-thread runtime with `threads`, every driver enabled, its workers
/// named [`WORKER_THREAD`] and its blocking threads [`BLOCKING_THREAD`].
///
/// # Errors
///
/// When the OS refuses the runtime its threads.
pub fn named(threads: Threads) -> io::Result<Runtime> {
    // Tokio starts each worker on a thread of its blocking pool while it
    // builds the runtime, and names a thread when it starts it: every
    // thread started before the runtime is built is a worker.
    let built = Arc::new(OnceLock::new());
    let named = Arc::clone(&built);
    let mut builder = multi_thread();
    builder
        .enable_all()
        .thread_name_fn(move || {
            if named.get().is_some() {
                BLOCKING_THREAD
            } else {
                WORKER_THREAD
            }
            .to_owned()
        })
        .max_blocking_threads(threads.blocking.get());
    if let Some(workers) = threads.workers {
        builder.worker_threads(workers.get());
    }
    let runtime = builder.build()?;
    let _ = built.set(());
    Ok(runtime)
}

#[cfg(test)]
mod tests;
