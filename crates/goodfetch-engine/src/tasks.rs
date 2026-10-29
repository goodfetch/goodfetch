//! The ownership tree (ADR 0038): every task the engine starts belongs to an
//! owner (the engine process, a Connection, a call, a run, later a Session
//! and a page), which holds a cancellation token, a child of its parent's,
//! and a tracker of its tasks. Stopping an owner cancels first, then waits
//! for its tasks. This is the only module that starts tasks and threads
//! (ADR 0060); the engine builds no runtime, `main` does.
//!
//! Blocking work runs under the owner's [`BlockingPolicy`], a value: in
//! production on tracked blocking threads, and an actor that owns blocking
//! state (the run's SQLite writer) on a dedicated thread its owner joins; in
//! the Simulation and in-process tests, blocking calls run inline and an
//! actor's loop runs as a task on the host's runtime.

use std::future::Future;
use std::io;
use std::ops::ControlFlow;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// Where an owner runs blocking work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockingPolicy {
    /// Production: tracked blocking threads, and a dedicated thread per
    /// actor, joined when its owner stops.
    Threads,
    /// The Simulation and in-process tests: blocking work runs inline, and
    /// an actor's loop runs as a task, so a run replays from its seed.
    Inline,
}

/// An owner of tasks: what it starts stops with it.
#[derive(Debug)]
pub struct TaskOwner {
    token: CancellationToken,
    tracker: TaskTracker,
    policy: BlockingPolicy,
}

impl TaskOwner {
    /// The root of a tree, such as the engine process's, whose blocking
    /// work runs under `policy`.
    pub fn root(policy: BlockingPolicy) -> Self {
        Self {
            token: CancellationToken::new(),
            tracker: TaskTracker::new(),
            policy,
        }
    }

    /// An owner under this one, with its blocking policy: cancelling this
    /// one cancels it, never the other way round. Its tasks are its own,
    /// which its holder waits for.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            token: self.token.child_token(),
            tracker: TaskTracker::new(),
            policy: self.policy,
        }
    }

    /// Its cancellation token, which its tasks watch at each cancellation
    /// point.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Its blocking policy.
    pub fn policy(&self) -> BlockingPolicy {
        self.policy
    }

    /// Starts `future` as one of its tasks.
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tracker.spawn(future);
    }

    /// Runs `work`, which blocks, under its blocking policy, and answers
    /// what it returns; a panic in `work` resumes in the caller.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops `work` once it started: on a
    /// blocking thread it runs to its end, and the owner waits for it.
    pub async fn blocking<T, F>(&self, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        match self.policy {
            BlockingPolicy::Inline => work(),
            BlockingPolicy::Threads => match self.tracker.spawn_blocking(work).await {
                Ok(answer) => answer,
                Err(error) => match error.try_into_panic() {
                    Ok(panic) => std::panic::resume_unwind(panic),
                    Err(error) => panic!("invariant: a blocking task is never cancelled: {error}"),
                },
            },
        }
    }

    /// Starts an actor named `name` that owns blocking state: `handle` gets
    /// every message `mailbox` holds at once, in order, so it can group
    /// them (the run's SQLite writer commits a group in one transaction).
    /// The actor ends once every sender of `mailbox` is gone, and the owner
    /// waits for it then.
    ///
    /// # Errors
    ///
    /// When the OS refuses the dedicated thread.
    pub fn actor<M, H>(
        &self,
        name: &str,
        mailbox: mpsc::Receiver<M>,
        mut handle: H,
    ) -> io::Result<()>
    where
        M: Send + 'static,
        H: FnMut(Vec<M>) + Send + 'static,
    {
        self.actor_until(name, mailbox, move |group| {
            handle(group);
            ControlFlow::Continue(())
        })
    }

    /// Starts an actor as [`TaskOwner::actor`] does, which also ends once
    /// `handle` breaks: its mailbox is dropped then, with every message it
    /// still holds, so a sender's later message is refused, whoever holds a
    /// sender still.
    ///
    /// # Errors
    ///
    /// When the OS refuses the dedicated thread.
    pub fn actor_until<M, H>(
        &self,
        name: &str,
        mut mailbox: mpsc::Receiver<M>,
        mut handle: H,
    ) -> io::Result<()>
    where
        M: Send + 'static,
        H: FnMut(Vec<M>) -> ControlFlow<()> + Send + 'static,
    {
        match self.policy {
            BlockingPolicy::Inline => {
                self.tracker.spawn(async move {
                    while let Some(first) = mailbox.recv().await {
                        if handle(drain(first, &mut mailbox)).is_break() {
                            break;
                        }
                    }
                });
            }
            BlockingPolicy::Threads => {
                let (finished, ended) = oneshot::channel::<()>();
                #[expect(
                    clippy::disallowed_methods,
                    reason = "the tasks module is the one place that starts threads (ADR 0060)"
                )]
                let thread =
                    std::thread::Builder::new()
                        .name(name.to_owned())
                        .spawn(move || {
                            while let Some(first) = mailbox.blocking_recv() {
                                if handle(drain(first, &mut mailbox)).is_break() {
                                    break;
                                }
                            }
                            drop(mailbox);
                            let _ = finished.send(());
                        })?;
                self.tracker.spawn(async move {
                    let _ = ended.await;
                    // The thread has returned, or panicked: joining takes no
                    // time, and a panic resumes here.
                    if let Err(panic) = thread.join() {
                        std::panic::resume_unwind(panic);
                    }
                });
            }
        }
        Ok(())
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

/// `first`, then every message `mailbox` holds now.
fn drain<M>(first: M, mailbox: &mut mpsc::Receiver<M>) -> Vec<M> {
    let mut batch = vec![first];
    while let Ok(next) = mailbox.try_recv() {
        batch.push(next);
    }
    batch
}

#[cfg(test)]
mod tests;
