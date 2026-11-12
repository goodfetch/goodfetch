//! The pages' tasks of a crawl's delivery (ADR 0142): each runs one
//! attempt at a time and tells the delivery loop, the actor that owns the
//! crawl's Run state, of its attempt's end in one message on the loop's
//! bounded mailbox; the answer carries the next attempt it runs, so a page
//! costs the loop one round trip, counted as `F3.actor_round_trips`, and
//! no task of its own. A task the loop
//! has nothing for ends, and the loop starts one again when it leases more
//! work than tasks wait for.

use std::sync::Arc;

use goodfetch_counted_work::Row;
use goodfetch_engine::CallContext;
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::{FetchAdmission, HandOver, PooledSession};
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinError, JoinSet};

use super::attempt::{Attempt, Tiering, attempt};
use crate::code_free::CodeFree;
use crate::lease::WorkItemLease;
use crate::progress::SharedInFlight;
use crate::work_items::Leased;

/// How many attempts' ends wait in the loop's mailbox before the next
/// page's task waits to send its own.
const MAILBOX: usize = 64;

/// An attempt to run: the Connection it is served on, by its place in the
/// delivery's list and its call context, its work item as leased, its
/// Session, its lease, how it is fetched and handed over, and the HTTP
/// admission it holds until it ends.
pub(super) struct Start {
    pub(super) server: (usize, CallContext),
    pub(super) leased: (Leased, (ServingSession, PooledSession), Counted),
    pub(super) lease: WorkItemLease,
    pub(super) fetching: (Tiering, (Arc<HandOver>, Option<Arc<CodeFree>>)),
    pub(super) admission: FetchAdmission,
}

/// A page's task's one message to the loop: its attempt's end. The answer
/// is the next attempt the task runs; dropped, the task ends.
struct Ended {
    attempt: Attempt,
    next: oneshot::Sender<Start>,
}

/// The pages' tasks of one crawl, as its delivery loop holds them.
pub(super) struct Pages {
    run: Run,
    in_flight: SharedInFlight,
    tasks: JoinSet<()>,
    mailbox: mpsc::Sender<Ended>,
    ended: mpsc::Receiver<Ended>,
    /// The tasks whose attempt ended and that wait for their next one.
    waiting: Vec<oneshot::Sender<Start>>,
    /// How many attempts run now.
    running: usize,
}

impl Pages {
    /// No task yet, for `run`'s crawl, whose requests in flight
    /// `in_flight` shows.
    pub(super) fn new(run: &Run, in_flight: &SharedInFlight) -> Self {
        let (mailbox, ended) = mpsc::channel(MAILBOX);
        Self {
            run: run.clone(),
            in_flight: Arc::clone(in_flight),
            tasks: JoinSet::new(),
            mailbox,
            ended,
            waiting: Vec::new(),
            running: 0,
        }
    }

    /// Whether no attempt runs.
    pub(super) fn idle(&self) -> bool {
        self.running == 0
    }

    /// Runs the attempt `start`: on a task that waits for its next one, as
    /// the answer to its message, else on a new task.
    pub(super) fn start(&mut self, mut start: Start) {
        self.running += 1;
        while let Some(waiting) = self.waiting.pop() {
            match waiting.send(start) {
                Ok(()) => return,
                // The task is gone: the Run's cancellation took it.
                Err(back) => start = back,
            }
        }
        self.tasks.spawn(page(
            self.run.clone(),
            Arc::clone(&self.in_flight),
            self.mailbox.clone(),
            start,
        ));
    }

    /// Ends each task that waits for a next attempt the loop has not: the
    /// loop calls it once it leased what it could.
    pub(super) fn dismiss(&mut self) {
        self.waiting.clear();
    }

    /// The attempt that ended first, then every other that ended by now,
    /// in the order they ended, so they settle together and the loop's own
    /// work, its admission's included, runs once for them all rather than
    /// once for each (#1765). Their tasks wait for their next attempts.
    /// None when a task ended instead: one that panicked ran an attempt,
    /// which no longer runs and is left out, as nothing is known of it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future takes no attempt out of the mailbox and loses
    /// no task's end.
    pub(super) async fn ended(&mut self) -> Vec<Attempt> {
        let first = tokio::select! {
            biased;
            Some(first) = self.ended.recv() => first,
            Some(joined) = self.tasks.join_next() => {
                if joined.is_err_and(|error: JoinError| error.is_panic()) {
                    self.running = self.running.saturating_sub(1);
                }
                return Vec::new();
            }
        };
        let mut attempts = Vec::new();
        let mut next = Some(first);
        while let Some(Ended {
            attempt,
            next: waiting,
        }) = next
        {
            self.running = self.running.saturating_sub(1);
            self.waiting.push(waiting);
            attempts.push(attempt);
            next = self.ended.try_recv().ok();
        }
        attempts
    }

    /// Abandons every attempt in flight, as the Run stops at once.
    pub(super) async fn shutdown(&mut self) {
        self.waiting.clear();
        self.tasks.shutdown().await;
        self.running = 0;
    }
}

/// One page's task: runs `start`, holding its HTTP admission until it
/// ends, unless its document's fetch gave it back first; tells the loop of
/// its end through `mailbox`; and runs the attempt the answer carries, until
/// the loop has none for it.
async fn page(run: Run, in_flight: SharedInFlight, mailbox: mpsc::Sender<Ended>, mut start: Start) {
    loop {
        let Start {
            server: (server, context),
            leased,
            lease,
            fetching,
            admission,
        } = start;
        // Boxed: an attempt's future is large, and the task keeps it only
        // while the attempt runs.
        let ended = Box::pin(attempt(
            run.clone(),
            server,
            context,
            leased,
            lease,
            fetching,
            Arc::clone(&in_flight),
        ))
        .await;
        admission.give_back();
        let (next, answer) = oneshot::channel();
        let ended = Ended {
            attempt: ended,
            next,
        };
        run.metrics().counted_work().count(Row::ActorRoundTrips);
        if mailbox.send(ended).await.is_err() {
            return;
        }
        match answer.await {
            Ok(next) => start = next,
            Err(_) => return,
        }
    }
}
