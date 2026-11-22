//! A state actor's mailbox: each job its callers send is answered by the
//! job, or refused once the actor ended without running it.
//!
//! A place taken before the actor's mailbox closed can still send once the
//! actor dropped the mailbox and the jobs it held (Tokio,
//! `Receiver::close`): such a job stays where no one reads it, so its
//! caller learns from the actor's end that no answer will come (issue
//! 3327).

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::{CancellationToken, DropGuard};

/// A job the actor runs on its state.
pub(super) type Job<S> = Box<dyn FnOnce(&mut S) + Send>;

/// The callers' side of an actor's mailbox.
#[derive(Debug)]
pub(super) struct Mailbox<S> {
    jobs: mpsc::Sender<Job<S>>,
    /// Cancelled once the actor ended, after its last job.
    ended: CancellationToken,
}

/// What the actor holds while it may still run a job: dropped with it, it
/// tells the callers that it ended.
#[derive(Debug)]
pub(super) struct Running {
    _ended: DropGuard,
}

/// A place for one job in a mailbox.
#[derive(Debug)]
pub(super) struct Place<'a, S> {
    permit: mpsc::Permit<'a, Job<S>>,
    ended: &'a CancellationToken,
}

impl<S> Mailbox<S> {
    /// A mailbox that holds up to `capacity` waiting jobs, the actor's
    /// side of it, and what the actor holds while it runs.
    pub(super) fn open(capacity: usize) -> (Self, mpsc::Receiver<Job<S>>, Running) {
        let (jobs, received) = mpsc::channel(capacity);
        let ended = CancellationToken::new();
        let running = Running {
            _ended: ended.clone().drop_guard(),
        };
        (Self { jobs, ended }, received, running)
    }

    /// Whether the actor closed its side.
    pub(super) fn is_closed(&self) -> bool {
        self.jobs.is_closed()
    }

    /// A place for one job, once one is free, or `None` once the actor
    /// closed its side.
    pub(super) async fn reserve(&self) -> Option<Place<'_, S>> {
        let permit = self.jobs.reserve().await.ok()?;
        Some(Place {
            permit,
            ended: &self.ended,
        })
    }
}

impl<S> Clone for Mailbox<S> {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
            ended: self.ended.clone(),
        }
    }
}

impl<S> Place<'_, S> {
    /// Sends `work`, and answers what it returned, or `None` once the actor
    /// ended without running it.
    pub(super) async fn send<T, F>(self, work: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut S) -> T + Send + 'static,
    {
        let (answer, mut answered) = oneshot::channel();
        self.permit.send(Box::new(move |state| {
            let _ = answer.send(work(state));
        }));
        tokio::select! {
            biased;
            answer = &mut answered => answer.ok(),
            // The actor ran its last job: the answer is in now, or never.
            () = self.ended.cancelled() => answered.try_recv().ok(),
        }
    }
}

#[cfg(test)]
mod tests;
