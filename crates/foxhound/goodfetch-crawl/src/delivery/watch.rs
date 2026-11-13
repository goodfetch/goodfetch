//! An attempt's watch (issue 2285, ADR 0148): a panic inside an attempt,
//! or inside a task it starts on its Run while it runs (its Lease's
//! renewal while a page waits for a Politeness Permit, the browser side of
//! its tier race), ends the attempt as a failed one with `engine_panicked`,
//! `ended: attempt`, through the same end as every other attempt, so each
//! of its holdings is given back by the code that gives it back. A panic
//! in the reading of a late answer, whose attempt is over, ends nothing:
//! the late answer is discarded. Each panic is counted in
//! `goodfetch.engine.panics`, and its error made where it was caught, on
//! the thread that panicked, so it names that panic's crash report.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures_util::FutureExt as _;
use goodfetch_engine::CallError;
use goodfetch_engine::crash_report::engine_panicked;
use goodfetch_engine::metrics::Metrics;
use goodfetch_protocol::connection::PanicEnded;
use tokio::sync::SetOnce;

/// What an attempt that panicked tells.
const ENDED: &str = "The engine hit a bug in Goodfetch while it ran this attempt, which ended the attempt; its work item is tried again as `crawl.retry.engine_panicked` says";

/// What the reading of a late answer that panicked tells: its attempt was
/// already over.
const LATE: &str = "The engine hit a bug in Goodfetch while it read the late answer of an attempt already over, which discarded the answer";

/// One attempt's watch, shared by the tasks it starts.
#[derive(Clone, Debug)]
pub(super) struct Watch(Arc<Watched>);

#[derive(Debug)]
struct Watched {
    /// The error of the first panic in a task the attempt started, once
    /// one panicked.
    panicked: SetOnce<CallError>,
    /// The Run's instruments, which count each panic.
    metrics: Metrics,
}

impl Watch {
    /// The watch of an attempt of the Run whose instruments are `metrics`.
    pub(super) fn new(metrics: &Metrics) -> Self {
        Self(Arc::new(Watched {
            panicked: SetOnce::new(),
            metrics: metrics.clone(),
        }))
    }

    /// Runs `attempt`: its output, or the error of a panic in it or in a
    /// task [`over`](Self::over) it, which drops it.
    pub(super) async fn ended<T>(&self, attempt: impl Future<Output = T>) -> Result<T, CallError> {
        let caught = AssertUnwindSafe(attempt).catch_unwind();
        tokio::select! {
            biased;
            panicked = self.0.panicked.wait() => Err(panicked.clone()),
            caught = caught => caught.map_err(|_| self.panicked()),
        }
    }

    /// `task`, to start on the Run as the attempt's: a panic in it ends the
    /// attempt, while it still runs.
    pub(super) fn over(
        &self,
        task: impl Future<Output = ()> + Send + 'static,
    ) -> impl Future<Output = ()> + Send + 'static {
        let watch = self.clone();
        async move {
            if AssertUnwindSafe(task).catch_unwind().await.is_err() {
                // A later panic ends nothing more: the first ended it.
                let _ = watch.0.panicked.set(watch.panicked());
            }
        }
    }

    /// The error of the panic the caller just caught, counted.
    fn panicked(&self) -> CallError {
        engine_panicked(&self.0.metrics, PanicEnded::Attempt, ENDED)
    }
}

/// `task`, to start on the Run whose instruments are `metrics`, which reads
/// the late answer of an attempt already over: a panic in it ends nothing,
/// and the late answer is discarded.
pub(super) fn after(
    metrics: &Metrics,
    task: impl Future<Output = ()> + Send + 'static,
) -> impl Future<Output = ()> + Send + 'static {
    let metrics = metrics.clone();
    async move {
        if AssertUnwindSafe(task).catch_unwind().await.is_err() {
            let error = engine_panicked(&metrics, PanicEnded::Attempt, LATE);
            tracing::debug!(code = error.code(), "{}", error.message());
        }
    }
}

#[cfg(test)]
mod tests;
