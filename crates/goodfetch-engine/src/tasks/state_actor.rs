//! An actor of an owner that keeps one state, such as a store's SQLite
//! connection, and runs each job sent to it on that state, answering its
//! caller: the state's one owner (STYLE.md), started by the first job.
//!
//! A job after the actor ended while its owner runs starts it again, from
//! a fresh state, as one does after the runtime it ran on under the Inline
//! policy ended (a harness that builds a runtime per step); once its owner
//! is cancelled, or once one of its jobs panicked, none does: a panic is
//! told to the owner's parent, which ends the owner, and nothing is
//! restarted in place (ADR 0148).

#![expect(
    clippy::disallowed_types,
    reason = "the mailbox is taken in one step and the panic flag set or read in one, never across an `.await` or another lock, so there is no lock order for loom to test (ADR 0060)"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::TaskOwner;

mod mailbox;

use mailbox::{Job, Mailbox};

/// How many jobs wait for the actor before a caller waits to send its own.
const MAILBOX: usize = 64;

/// An actor named `name` of `owner` that keeps a state `S`, and the
/// mailbox its callers share.
#[derive(Debug)]
pub(crate) struct StateActor<S> {
    name: &'static str,
    owner: Arc<TaskOwner>,
    mailbox: Mutex<Option<Mailbox<S>>>,
    /// Set once one of its jobs panicked.
    panicked: Arc<AtomicBool>,
}

/// Why an actor ran no job, as the end of "… is not running: ": its owner
/// stopped, one of its jobs panicked, or the OS refused its thread.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ActorStopped(String);

impl<S: Default + Send + 'static> StateActor<S> {
    /// The actor named `name` of `owner`, which no job started yet.
    pub(crate) fn of(name: &'static str, owner: Arc<TaskOwner>) -> Self {
        Self {
            name,
            owner,
            mailbox: Mutex::new(None),
            panicked: Arc::default(),
        }
    }

    /// Its owner.
    pub(crate) fn owner(&self) -> &Arc<TaskOwner> {
        &self.owner
    }

    /// Runs `work` on the actor's state, after the jobs sent before it, and
    /// answers what it returned.
    ///
    /// # Errors
    ///
    /// When the actor's owner stopped, one of its jobs panicked, this one
    /// included, or the OS refused its thread.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the job was sent leaves it to run.
    pub(crate) async fn run<T, F>(&self, work: F) -> Result<T, ActorStopped>
    where
        T: Send + 'static,
        F: FnOnce(&mut S) -> T + Send + 'static,
    {
        let mailbox = self.mailbox()?;
        let place = mailbox.reserve().await.ok_or_else(|| self.stopped())?;
        place.send(work).await.ok_or_else(|| self.stopped())
    }

    /// The actor's mailbox, the actor started now if it is not running and
    /// may start.
    fn mailbox(&self) -> Result<Mailbox<S>, ActorStopped> {
        let mut mailbox = self.mailbox.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(open) = mailbox.as_ref().filter(|mailbox| !mailbox.is_closed()) {
            return Ok(open.clone());
        }
        if self.owner.token().is_cancelled() || self.panicked.load(Ordering::Acquire) {
            return Err(self.stopped());
        }
        let (sender, jobs, running) = Mailbox::open(MAILBOX);
        let mut state = S::default();
        let panicked = Arc::clone(&self.panicked);
        self.owner
            .actor(self.name, jobs, move |group: Vec<Job<S>>| {
                // Moves `running` into the handler, which drops it once it
                // ran its last job, panicked or ended with its runtime.
                let _running = &running;
                for job in group {
                    // Marks the actor before its mailbox closes, so a job
                    // that finds it closed after a panic starts nothing.
                    let _watch = PanicWatch(&panicked);
                    job(&mut state);
                }
            })
            .map_err(|error| ActorStopped(format!("its thread did not start: {error}")))?;
        Ok(mailbox.insert(sender).clone())
    }

    fn stopped(&self) -> ActorStopped {
        let why = if self.panicked.load(Ordering::Acquire) {
            "one of its jobs panicked"
        } else {
            "its owner stopped"
        };
        ActorStopped(why.to_owned())
    }
}

/// Sets its flag when it is dropped by a panic's unwinding.
struct PanicWatch<'a>(&'a AtomicBool);

impl Drop for PanicWatch<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests;
