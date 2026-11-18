//! The multi-thread runtimes the crate builds, the Attached engine's, the
//! Daemon's, a command's Connection's and a linked engine's, sized by
//! `governor.worker_threads` and `governor.blocking_threads`, which `main`
//! reads once, before any of them is built (#2200). Their two kinds of
//! threads carry names of their own, so `/proc/<pid>/task` tells Tokio's
//! workers from its blocking threads.

use goodfetch_engine::settings::{Resolved, get_or_default};
use goodfetch_protocol::governor::{
    BLOCKING_THREADS, BlockingThreadCap, WORKER_THREADS, WorkerThreads,
};

/// How many threads of each kind a runtime has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Threads {
    workers: WorkerThreads,
    blocking: BlockingThreadCap,
}

impl Threads {
    /// What the machine's `settings` set, or the catalog's defaults where
    /// they could not be read.
    pub(crate) fn of(settings: Option<&Resolved>) -> Self {
        Self {
            workers: get_or_default(settings, &WORKER_THREADS),
            blocking: get_or_default(settings, &BLOCKING_THREADS),
        }
    }
}

/// A multi-thread runtime with `threads`, its workers and its blocking
/// threads named apart (`goodfetch-runtime`).
///
/// # Panics
///
/// If the OS refuses the runtime its threads.
pub(crate) fn multi_thread(threads: Threads) -> tokio::runtime::Runtime {
    let workers = match threads.workers {
        WorkerThreads::Count(count) => Some(count),
        WorkerThreads::PerCore => None,
    };
    goodfetch_runtime::named(goodfetch_runtime::Threads {
        workers,
        blocking: threads.blocking.into(),
    })
    .expect("invariant: the OS gives the runtime its threads")
}
