//! What other tasks ask of the crawl's Run state (ADR 0142). The delivery
//! loop is the actor that owns it and its only caller, so every other task
//! reaches it in one message on the loop's bounded mailbox: a seed added
//! to the running crawl, a handler's enqueue, a program's links batch, a
//! re-admission of the failures, and a redirect hop's check against the
//! visited set with the record of its chain's end. None of them queues a
//! job with the run-state writer itself.
//!
//! The visited set and the pending work items are still the run file's
//! (#2167), so the loop answers from the file, in a job of the run-state
//! writer it queues in its own order among the claims and the outcomes.
//!
//! - What an attempt stages, a handler's enqueue, a program's links batch
//!   or a seed its attempt adds, and a redirect hop's question give the
//!   crawl no work item yet: the loop takes the job's place in the
//!   writer's queue and answers with the job, which the asking task waits
//!   for and the loop does not.
//! - What gives the crawl work with no attempt, a seed or a batch of
//!   seeds added to the running crawl and a re-admission, the loop waits
//!   for itself, as it waited for the keys it wrote of them before: each
//!   work item is keyed as it is added, so the loop offers it to its
//!   domain's queue as soon as it is written, writes no key apart, and
//!   then answers. The call is answered once its change is written.
//!
//! A redirect hop costs its page's task one round trip, counted as
//! `F3.hop_round_trips`, and its chain's end one more, counted as
//! `F3.hop_round_trips.end` (#2246). The end's record does not fold into
//! the attempt's outcome: it is made before the handler runs, which a
//! chain that reached another request's page never does (ADR 0120).
//! Neither promises anything outside, so neither is synced: the attempt's
//! outcome syncs them (ADR 0138's note of #1926).
//!
//! An attempt that escalates from a Session's own transport asks the loop
//! for a Session whose client is its browser, to run again in (#2416):
//! memory answers it, with a draw of the pool as a lease's are, and no job
//! of the run-state writer. It costs its attempt one round trip, counted
//! as `F3.actor_round_trips`; an attempt that does not escalate across
//! clients asks nothing.

use std::pin::Pin;
use std::time::SystemTime;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::run_state::RunStateError;
use goodfetch_fetch_tier::{ChainHop, HopAnswer, HopChecks};
use goodfetch_frontier::Hop;
use goodfetch_protocol::foxhound::crawl::Seed;
use goodfetch_protocol::foxhound::frontier::{Enqueued, EnqueuedLinks, SeedAttempt};
use goodfetch_session_pool::{PoolKey, PoolSettings};
use tokio::sync::{mpsc, oneshot};

use super::admission::{Enqueue, LinksBatch};
use super::{DrawnSession, StateError};
use crate::labels;

/// How many questions wait in the loop's mailbox before the next task
/// waits to send its own.
const MAILBOX: usize = 64;

/// The job the loop queued for a question, which the asking task waits
/// for.
pub(super) type Queued<T> = Pin<Box<dyn Future<Output = Result<T, StateError>> + Send>>;

/// Where the loop answers a question with its job.
pub(super) type Answering<T> = oneshot::Sender<Queued<T>>;

/// Where the loop answers a question whose job it waited for itself.
pub(super) type Written<T> = oneshot::Sender<Result<T, StateError>>;

/// What an admission staged with an attempt answers: `None` once the
/// attempt ended, else its answer or the call's own refusal.
pub(crate) type Staged<T> = Option<Result<T, CallError>>;

/// What a page's task asks of a redirect hop.
#[derive(Clone, Copy)]
pub(crate) enum HopAsk {
    /// Where the hop leads its request.
    Check,
    /// That its chain ended on it.
    Reached,
}

/// One message to the loop: a question the run file answers, or an
/// escalated attempt's draw, which memory answers. A stored question is
/// boxed: it is the larger variant, more than twice the other on Windows.
pub(crate) enum Question {
    /// A question the run file answers.
    Stored(Box<Ask>),
    /// The draw of a Session for an escalated attempt.
    Escalated {
        escalated: Escalated,
        answer: oneshot::Sender<DrawnSession>,
    },
}

/// The draw of a Session whose client is its browser, for an attempt that
/// escalated from a Session's own transport (#2416): from the pool `pool`
/// of the crawl's `labels`, for a request on `domain_key` to `host`, under
/// the Run's pool `settings` at `now`, holding `place`, the browser-page
/// place its page takes (#2178).
pub(crate) struct Escalated {
    pub(crate) labels: std::sync::Arc<labels::Declared>,
    pub(crate) pool: PoolKey,
    pub(crate) domain_key: String,
    pub(crate) host: String,
    pub(crate) settings: PoolSettings,
    pub(crate) now: SystemTime,
    pub(crate) place: goodfetch_fetch_tier::HeldPlace,
}

/// A question the run file answers.
pub(crate) enum Ask {
    /// A seed added to the running crawl with no attempt. `None` once the
    /// crawl closed its intake.
    Seed {
        seed: Seed,
        answer: Written<Option<Result<Enqueued, CallError>>>,
    },
    /// A seed a handler's attempt adds, staged with it. `None` once the
    /// crawl closed its intake.
    StagedSeed {
        seed: Seed,
        attempt: SeedAttempt,
        answer: Answering<Option<Staged<Enqueued>>>,
    },
    /// A batch of seeds added to the running crawl, admitted together.
    /// `None` once the crawl closed its intake.
    Seeds {
        seeds: Vec<Seed>,
        answer: Written<Option<Result<Vec<Enqueued>, CallError>>>,
    },
    /// A handler's enqueue, staged with its attempt.
    Enqueue {
        enqueue: Enqueue,
        answer: Answering<Staged<Enqueued>>,
    },
    /// A program's links batch, staged with its attempt.
    Links {
        links: LinksBatch,
        answer: Answering<Staged<EnqueuedLinks>>,
    },
    /// The re-admission of every failure, and with `blocked` of every
    /// blocked request. `None` once the crawl closed its intake.
    Readmit {
        blocked: bool,
        answer: Written<Option<Result<u64, CallError>>>,
    },
    /// A redirect hop's question.
    Hop {
        hop: ChainHop,
        asks: HopAsk,
        answer: Answering<Hop>,
    },
}

/// The other tasks' side of the mailbox: what a live crawl's calls, and
/// its pages' redirect chains, ask its Run state through.
#[derive(Clone)]
pub(crate) struct Asking {
    work: CountedWork,
    mailbox: mpsc::Sender<Question>,
}

/// The loop's side of the mailbox.
pub(crate) struct Asked(mpsc::Receiver<Question>);

/// The mailbox of a crawl whose counted work `work` counts: the handle
/// other tasks ask through, and the loop's side.
pub(crate) fn mailbox(work: CountedWork) -> (Asking, Asked) {
    let (mailbox, asked) = mpsc::channel(MAILBOX);
    (Asking { work, mailbox }, Asked(asked))
}

impl Asked {
    /// The next question a task asks; none once no task can ask.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future takes no question out of the mailbox.
    pub(crate) async fn next(&mut self) -> Option<Question> {
        self.0.recv().await
    }
}

impl Asking {
    /// Sends the loop the question `ask`; `Closed` once the loop ended.
    async fn send(&self, ask: Ask) -> Result<(), StateError> {
        self.mailbox
            .send(Question::Stored(Box::new(ask)))
            .await
            .map_err(|_| StateError::Closed)
    }

    /// Asks the loop the question `ask` makes of where it is answered,
    /// and waits for the job the loop queued for it. `Closed` once the
    /// loop ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the question is sent does not withdraw its
    /// job.
    async fn staged<T>(&self, ask: impl FnOnce(Answering<T>) -> Ask) -> Result<T, StateError> {
        let (answer, answered) = oneshot::channel();
        self.send(ask(answer)).await?;
        answered.await.map_err(|_| StateError::Closed)?.await
    }

    /// Asks the loop the question `ask` makes of where it is answered,
    /// and waits for its answer, which the loop gives once the question's
    /// job is written. `Closed` once the loop ended.
    ///
    /// # Cancel safety
    ///
    /// As [`Asking::staged`]'s.
    async fn written<T>(&self, ask: impl FnOnce(Written<T>) -> Ask) -> Result<T, StateError> {
        let (answer, answered) = oneshot::channel();
        self.send(ask(answer)).await?;
        answered.await.map_err(|_| StateError::Closed)?
    }

    /// Adds `seed` to the running crawl, staged with `attempt` when a
    /// handler's attempt adds it; `None` once the crawl closed its intake.
    ///
    /// # Errors
    ///
    /// `Closed` once the crawl ended, else the run file's.
    pub(crate) async fn seed(
        &self,
        seed: Seed,
        attempt: Option<SeedAttempt>,
    ) -> Result<Option<Staged<Enqueued>>, StateError> {
        match attempt {
            Some(attempt) => {
                self.staged(|answer| Ask::StagedSeed {
                    seed,
                    attempt,
                    answer,
                })
                .await
            }
            None => Ok(self
                .written(|answer| Ask::Seed { seed, answer })
                .await?
                .map(Some)),
        }
    }

    /// Adds `seeds` to the running crawl in one job; `None` once the crawl
    /// closed its intake.
    ///
    /// # Errors
    ///
    /// As [`Asking::seed`]'s.
    pub(crate) async fn seeds(
        &self,
        seeds: Vec<Seed>,
    ) -> Result<Option<Result<Vec<Enqueued>, CallError>>, StateError> {
        self.written(|answer| Ask::Seeds { seeds, answer }).await
    }

    /// Stages `enqueue` with its attempt.
    ///
    /// # Errors
    ///
    /// As [`Asking::seed`]'s.
    pub(crate) async fn enqueue(&self, enqueue: Enqueue) -> Result<Staged<Enqueued>, StateError> {
        self.staged(|answer| Ask::Enqueue { enqueue, answer }).await
    }

    /// Stages the batch `links` with its attempt.
    ///
    /// # Errors
    ///
    /// As [`Asking::seed`]'s.
    pub(crate) async fn links(
        &self,
        links: LinksBatch,
    ) -> Result<Staged<EnqueuedLinks>, StateError> {
        self.staged(|answer| Ask::Links { links, answer }).await
    }

    /// Re-admits every failure, and with `blocked` every blocked request,
    /// and answers how many; `None` once the crawl closed its intake. None
    /// is re-admitted, with `label_unknown`, when one names a Label the
    /// crawl no longer declares.
    ///
    /// # Errors
    ///
    /// As [`Asking::seed`]'s.
    pub(crate) async fn readmit(
        &self,
        blocked: bool,
    ) -> Result<Option<Result<u64, CallError>>, StateError> {
        self.written(|answer| Ask::Readmit { blocked, answer })
            .await
    }

    /// Asks the loop for a Session whose client is its browser, drawn as
    /// `escalated` says, for an attempt that escalated from a Session's own
    /// transport, and waits for it: one round trip. `Closed` once the loop
    /// ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before the loop answers leaves its Session with
    /// the loop, which settles it as it finds no one waits for it.
    pub(crate) async fn escalated(&self, escalated: Escalated) -> Result<DrawnSession, StateError> {
        self.work.count(Row::ActorRoundTrips);
        let (answer, answered) = oneshot::channel();
        self.mailbox
            .send(Question::Escalated { escalated, answer })
            .await
            .map_err(|_| StateError::Closed)?;
        answered.await.map_err(|_| StateError::Closed)
    }

    /// Asks the loop `asks` of `hop` and waits for its answer: one round
    /// trip. The fetch tier's own error: `Closed` once the loop ended.
    async fn hop(&self, hop: ChainHop, asks: HopAsk) -> Result<Hop, RunStateError> {
        self.work.count(match asks {
            HopAsk::Check => Row::HopRoundTrips,
            HopAsk::Reached => Row::HopEndRoundTrips,
        });
        self.staged(|answer| Ask::Hop { hop, asks, answer })
            .await
            .map_err(StateError::run_file)
    }
}

impl HopChecks for Asking {
    fn check(&self, hop: ChainHop) -> HopAnswer<'_> {
        Box::pin(self.hop(hop, HopAsk::Check))
    }

    fn reached(&self, hop: ChainHop) -> HopAnswer<'_> {
        Box::pin(self.hop(hop, HopAsk::Reached))
    }
}
