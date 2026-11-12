//! The ownership tree (ADR 0038): every task the engine starts belongs to an
//! owner (the engine process, a Connection, a call, a run, later a Session
//! and a page), which holds a cancellation token, a child of its parent's,
//! and a tracker of its tasks. Stopping an owner cancels first, then waits
//! for its tasks, then ends its actors once they handled what their
//! mailboxes hold, whoever still holds a sender. This is the only module
//! that starts tasks and threads (ADR 0060); the engine builds no runtime,
//! `main` does.
//!
//! Blocking work runs under the owner's [`BlockingPolicy`], a value: in
//! production on tracked blocking threads, and an actor that owns blocking
//! state (the run's SQLite writer) on a dedicated thread its owner joins; in
//! the Simulation and in-process tests, blocking calls run inline and an
//! actor's loop runs as a task on the host's runtime.

use std::future::Future;
use std::io;
use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::crash_report;

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
    /// Its actors, which outlive its other tasks: a stop ends them only
    /// once those have ended.
    actors: TaskTracker,
    /// Cancelled once a stop's other tasks have ended: each actor then
    /// closes its mailbox, handles what it still holds and ends.
    closing: CancellationToken,
    policy: BlockingPolicy,
    /// The CPU pool's permits, one per core, which every owner of the
    /// tree shares: each Page analysis on a blocking thread holds one.
    analyses: Arc<Semaphore>,
}

impl TaskOwner {
    /// The root of a tree, such as the engine process's, whose blocking
    /// work runs under `policy`, with a CPU pool sized to the machine's
    /// cores (one when the OS cannot tell).
    pub fn root(policy: BlockingPolicy) -> Self {
        let cores = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        Self::root_on_cores(policy, cores)
    }

    /// The root of a tree whose blocking work runs under `policy`, with a
    /// CPU pool of `cores` permits.
    pub fn root_on_cores(policy: BlockingPolicy, cores: NonZeroUsize) -> Self {
        Self {
            token: CancellationToken::new(),
            tracker: TaskTracker::new(),
            actors: TaskTracker::new(),
            closing: CancellationToken::new(),
            policy,
            analyses: Arc::new(Semaphore::new(cores.get())),
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
            actors: TaskTracker::new(),
            closing: CancellationToken::new(),
            policy: self.policy,
            analyses: Arc::clone(&self.analyses),
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
        run_blocking(&self.tracker, self.policy, work).await
    }

    /// Its blocking threads, as a handle that runs blocking work as
    /// [`TaskOwner::blocking`] does for whoever holds it, such as a run
    /// file's readers, which its writer's handles hold; the owner waits for
    /// that work too.
    pub fn blocking_threads(&self) -> BlockingThreads {
        BlockingThreads {
            tracker: self.tracker.clone(),
            policy: self.policy,
        }
    }

    /// Runs `work`, a Page analysis's parsing or reading of a page, on the
    /// CPU pool, and answers what it returns: on a blocking thread once
    /// one of the pool's permits is free, held until `work` returns, so no
    /// more analyses run at once than the machine has cores, whatever the
    /// requests in flight (ADR 0138); inline under
    /// [`BlockingPolicy::Inline`]. A panic in `work` resumes in the caller.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future while it waits for a permit runs nothing;
    /// once `work` started, as [`TaskOwner::blocking`].
    pub async fn analysis<T, F>(&self, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        match self.policy {
            BlockingPolicy::Inline => work(),
            BlockingPolicy::Threads => {
                let permit = Arc::clone(&self.analyses)
                    .acquire_owned()
                    .await
                    .expect("invariant: the CPU pool's permits are never closed");
                self.blocking(move || {
                    let answer = work();
                    drop(permit);
                    answer
                })
                .await
            }
        }
    }

    /// Starts an actor named `name` that owns blocking state: `handle` gets
    /// every message `mailbox` holds at once, in order, so it can group
    /// them (the run's SQLite writer commits a group in one transaction).
    /// The actor ends once every sender of `mailbox` is gone, or once a
    /// stop of the owner saw its other tasks end: it closes its mailbox
    /// then, refusing any later message, whoever holds a sender, and
    /// handles what the mailbox still holds first. The owner waits for it.
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
        let closing = self.closing.clone();
        match self.policy {
            BlockingPolicy::Inline => {
                self.actors.spawn(async move {
                    while let Some(first) = next(&mut mailbox, &closing).await {
                        if handle(drain(first, &mut mailbox)).is_break() {
                            break;
                        }
                    }
                });
            }
            BlockingPolicy::Threads => {
                // The thread waits for its next message on the runtime's
                // handle, outside any of its tasks, beside the stop's
                // closing.
                let runtime = tokio::runtime::Handle::current();
                let (finished, ended) = oneshot::channel::<()>();
                #[expect(
                    clippy::disallowed_methods,
                    reason = "the tasks module is the one place that starts threads (ADR 0060)"
                )]
                let thread =
                    std::thread::Builder::new()
                        .name(name.to_owned())
                        .spawn(move || {
                            while let Some(first) = runtime.block_on(next(&mut mailbox, &closing)) {
                                if handle(drain(first, &mut mailbox)).is_break() {
                                    break;
                                }
                            }
                            drop(mailbox);
                            let _ = finished.send(());
                        })?;
                self.actors.spawn(async move {
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

    /// Waits for every task it started, without cancelling them, its
    /// actors last, each ending once every sender of its mailbox is gone.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops waiting; the tasks keep running and no new
    /// one can be started.
    pub async fn wait(&self) {
        self.tracker.close();
        self.tracker.wait().await;
        self.actors.close();
        self.actors.wait().await;
    }

    /// Cancels its tasks, waits for them, then ends its actors once each
    /// handled what its mailbox holds, never waiting for a sender held
    /// anywhere else to go (#1763).
    ///
    /// # Cancel safety
    ///
    /// As [`TaskOwner::wait`]: the tasks are already cancelled.
    pub async fn stop(&self) {
        self.cancel();
        self.tracker.close();
        self.tracker.wait().await;
        self.closing.cancel();
        self.actors.close();
        self.actors.wait().await;
    }
}

/// An owner's blocking threads, under its policy, for whoever holds the
/// handle ([`TaskOwner::blocking_threads`]).
#[derive(Clone, Debug)]
pub struct BlockingThreads {
    tracker: TaskTracker,
    policy: BlockingPolicy,
}

impl BlockingThreads {
    /// Runs `work`, which blocks, as [`TaskOwner::blocking`] does.
    ///
    /// # Cancel safety
    ///
    /// As [`TaskOwner::blocking`]'s.
    pub async fn run<T, F>(&self, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        run_blocking(&self.tracker, self.policy, work).await
    }
}

/// Runs `work` under `policy`, on a blocking thread `tracker` tracks, and
/// answers what it returns; a panic in `work` resumes in the caller, with
/// the name of its crash report (ADR 0148).
async fn run_blocking<T, F>(tracker: &TaskTracker, policy: BlockingPolicy, work: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match policy {
        BlockingPolicy::Inline => work(),
        BlockingPolicy::Threads => {
            let caught = tracker.spawn_blocking(move || {
                std::panic::catch_unwind(AssertUnwindSafe(work))
                    .map_err(|panic| (panic, crash_report::take_written()))
            });
            match caught.await {
                Ok(Ok(answer)) => answer,
                Ok(Err((panic, written))) => crash_report::resume_unwind(panic, written),
                Err(error) => panic!("invariant: a blocking task is never cancelled: {error}"),
            }
        }
    }
}

/// The next message of `mailbox`; once `closing` is cancelled, the mailbox
/// is closed first, so it answers what it holds, then none.
async fn next<M>(mailbox: &mut mpsc::Receiver<M>, closing: &CancellationToken) -> Option<M> {
    tokio::select! {
        biased;
        message = mailbox.recv() => message,
        () = closing.cancelled() => {
            mailbox.close();
            mailbox.recv().await
        }
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
