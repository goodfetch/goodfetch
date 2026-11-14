//! The multi-thread runtimes the executable builds, the Attached engine's,
//! the Daemon's and a command's Connection's, sized by
//! `governor.worker_threads` and `governor.blocking_threads`, which `main`
//! reads once, before any of them is built (#2200). Their two kinds of
//! threads carry names of their own, so `/proc/<pid>/task` tells Tokio's
//! workers from its blocking threads.

use std::sync::{Arc, OnceLock};

use goodfetch_engine::settings::{Resolved, get_or_default};
use goodfetch_protocol::governor::{
    BLOCKING_THREADS, BlockingThreadCap, WORKER_THREADS, WorkerThreads,
};

/// The name of a Tokio worker thread.
pub(super) const WORKER_THREAD: &str = "gf-worker";

/// The name of a thread of Tokio's blocking pool.
pub(super) const BLOCKING_THREAD: &str = "gf-blocking";

/// How many threads of each kind a runtime has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Threads {
    workers: WorkerThreads,
    blocking: BlockingThreadCap,
}

impl Threads {
    /// What the machine's `settings` set, or the catalog's defaults where
    /// they could not be read.
    pub(super) fn of(settings: Option<&Resolved>) -> Self {
        Self {
            workers: get_or_default(settings, &WORKER_THREADS),
            blocking: get_or_default(settings, &BLOCKING_THREADS),
        }
    }
}

/// A multi-thread runtime with `threads`, its workers named
/// [`WORKER_THREAD`] and its blocking threads [`BLOCKING_THREAD`].
///
/// # Panics
///
/// If the OS refuses the runtime its threads.
pub(super) fn multi_thread(threads: Threads) -> tokio::runtime::Runtime {
    // Tokio starts each worker on a thread of its blocking pool while it
    // builds the runtime, and names a thread when it starts it: every
    // thread started before the runtime is built is a worker.
    let built = Arc::new(OnceLock::new());
    let named = Arc::clone(&built);
    #[expect(
        clippy::disallowed_methods,
        reason = "main builds the runtime (ADR 0060)"
    )]
    let mut builder = tokio::runtime::Builder::new_multi_thread();
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
    if let WorkerThreads::Count(count) = threads.workers {
        builder.worker_threads(count.get());
    }
    let runtime = builder
        .build()
        .expect("invariant: the OS gives the runtime its threads");
    let _ = built.set(());
    runtime
}
