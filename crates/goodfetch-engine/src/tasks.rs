//! The ownership tree (ADR 0038): every task the engine starts belongs to an
//! owner (the engine process, a Connection, a call, later a run, a Session,
//! a page), which holds a cancellation token, a child of its parent's, and a
//! tracker of its tasks. Stopping an owner cancels first, then waits for its
//! tasks. This is the only module that starts tasks (ADR 0060); the engine
//! builds no runtime, `main` does. Blocking work and dedicated threads join
//! here with the first module that needs them (#215).

use std::future::Future;

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// An owner of tasks: what it starts stops with it.
#[derive(Debug)]
pub struct TaskOwner {
    token: CancellationToken,
    tracker: TaskTracker,
}

impl TaskOwner {
    /// The root of a tree, such as the engine process's.
    pub fn root() -> Self {
        Self {
            token: CancellationToken::new(),
            tracker: TaskTracker::new(),
        }
    }

    /// An owner under this one: cancelling this one cancels it, never the
    /// other way round. Its tasks are its own, which its holder waits for.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            token: self.token.child_token(),
            tracker: TaskTracker::new(),
        }
    }

    /// Its cancellation token, which its tasks watch at each cancellation
    /// point.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Starts `future` as one of its tasks.
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tracker.spawn(future);
    }

    /// Asks its tasks to stop at their next cancellation point.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Waits for every task it started, without cancelling them.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops waiting; the tasks keep running and no new
    /// one can be started.
    pub async fn wait(&self) {
        self.tracker.close();
        self.tracker.wait().await;
    }

    /// Cancels its tasks, then waits for them.
    ///
    /// # Cancel safety
    ///
    /// As [`TaskOwner::wait`]: the tasks are already cancelled.
    pub async fn stop(&self) {
        self.cancel();
        self.wait().await;
    }
}

#[cfg(test)]
mod tests;
