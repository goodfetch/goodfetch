//! The Kennel's one task owner (STYLE.md, ADR 0060): every task and every
//! blocking call starts through its `TaskTracker`, on the runtime `main`
//! holds, and stops when the Kennel stops.

use std::future::Future;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// The tracker, its runtime and the Kennel's stop.
#[derive(Clone, Debug)]
pub struct Tasks {
    tracker: TaskTracker,
    runtime: Handle,
    stop: CancellationToken,
}

impl Tasks {
    /// An owner of tasks on `runtime`.
    pub fn new(runtime: Handle) -> Self {
        Self {
            tracker: TaskTracker::new(),
            runtime,
            stop: CancellationToken::new(),
        }
    }

    /// Starts `task`, which ends when the Kennel stops at the latest.
    pub fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        let stop = self.stop.clone();
        self.tracker.spawn_on(
            async move {
                tokio::select! {
                    () = stop.cancelled() => {}
                    () = task => {}
                }
            },
            &self.runtime,
        );
    }

    /// Runs the blocking `call` on the tracker's blocking threads. A panic
    /// in `call` resumes here; a call the stopping runtime dropped before
    /// it ran never answers, and the task awaiting it is stopped with it.
    ///
    /// # Cancel safety
    /// Dropping the future does not stop `call`, whose result is lost.
    pub async fn blocking<T: Send + 'static>(
        &self,
        call: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        match self.tracker.spawn_blocking_on(call, &self.runtime).await {
            Ok(value) => value,
            Err(failure) => match failure.try_into_panic() {
                Ok(panic) => std::panic::resume_unwind(panic),
                Err(_) => std::future::pending().await,
            },
        }
    }

    /// Cancelled once the Kennel stops.
    pub fn stopping(&self) -> CancellationToken {
        self.stop.clone()
    }

    /// Stops every task and waits up to `grace` for them to end.
    ///
    /// # Cancel safety
    /// Dropping the future stops the wait; the tasks still stop.
    pub async fn stop(&self, grace: Duration) {
        self.stop.cancel();
        self.tracker.close();
        let _ = tokio::time::timeout(grace, self.tracker.wait()).await;
    }
}
