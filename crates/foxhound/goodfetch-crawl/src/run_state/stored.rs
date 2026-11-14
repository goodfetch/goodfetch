//! The run file behind the crawl's Run state (ADR 0142; #2167): the
//! visited set, the pending work items, the claims and the outcomes are
//! still the run file's, so each operation of the Run state that needs
//! them is a job of the Run's run-state writer, or a read on its readers,
//! and every one of those jobs and reads of a running crawl is here. The
//! delivery loop, the actor that owns the Run state, queues them in its
//! own order and names no run file; #2168 and #2169 move what they read
//! into memory behind the same operations.
//!
//! Each write goes out through the writer as a sealed batch where its rows
//! are already decided (the domain keys, the tier choices' counts, the
//! per-domain queues' journal), and as a job where it still reads the
//! file to decide (an admission, a claim, an outcome).
//!
//! Each job that changes the crawl's requests by state answers what it
//! changed ([`Changed`]), which the Run state counts in memory: once the
//! crawl opened, no count is read from the run file (#2167).

use std::pin::Pin;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::admission::{Journal, Keys, Waiting};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::run_state::{
    BatchRows as _, BatchWriter, RunStateError, SealedBatch, Syncing,
};
use goodfetch_engine::runs::CompletenessReason;
use goodfetch_fetch_tier::ChainHop;
use goodfetch_frontier::sitemap;
use goodfetch_protocol::run::PointInTime;
use goodfetch_session_pool::{Changes, SessionPool};
use rusqlite::Connection;

use super::admission::{self, Enqueue, LinksBatch};
use super::asks::{self, Answering, Ask, HopAsk, Queued};
use super::tally::Tally;
use super::{Counted, StateError};
use crate::attempt_outcome::{self, Ended, Ending, Outcome, Written};
use crate::completeness;
use crate::intake::Intake;
use crate::invariants::Invariants;
use crate::staging::Staging;
use crate::work_items::{self, Changed, Failure, Leased, Positions, Unsuccessful};

/// What an outcome's job answers: how its work item ended, each pending
/// request a rule its page taught merged, by ID with its domain key, and
/// what it changed of the crawl's requests by state.
pub(crate) type EndWritten = Result<Written, StateError>;

/// What a write the loop waited for gave the crawl: the work items it
/// added, each keyed as it was added, for the loop to offer their
/// domains' queues, and what it changed of the crawl's requests by state,
/// for the Run state to count.
pub(crate) type Given = (Vec<Waiting>, Changed);

/// An outcome's job, queued, which answers once it is written.
pub(crate) type QueuedEnd = Pin<Box<dyn Future<Output = EndWritten> + Send>>;

/// How an attempt's outcome is written: it ends one attempt. The bytes of
/// its rows are not counted before its job runs.
const ATTEMPT_ENDED: Syncing = Syncing::Attempt { ended: 1, bytes: 0 };

/// The run file of a running crawl, as its Run state reads and writes it:
/// the Run's run-state writer, with what each admission is judged within,
/// the Run's domain policy and its invariants, built once (#1896), where
/// its attempts stage, its intake, and how the per-domain queues key a
/// work item.
#[derive(Clone)]
pub(crate) struct Stored {
    writer: goodfetch_engine::run_state::RunState,
    egress: Arc<Policy>,
    invariants: Invariants,
    staging: Staging,
    intake: Intake,
    keys: Keys,
    work: CountedWork,
}

/// When the delivery loop found work items with no domain key.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Unkeyed {
    /// As the crawl opens: the seeds its start wrote, which no gate had
    /// keyed.
    Opening,
    /// While the crawl runs, once it is idle. Every admission keys the
    /// work items it adds, in the write that adds them (#2167), so none
    /// is expected: one found is a work item an admission left out of
    /// every domain's queue, which would hold the crawl for ever. A
    /// sitemap's read was the last admission to leave any, so the write
    /// of their keys counts on its row, `F3.transactions.sitemap`.
    Running,
}

/// What a crawl's admissions are judged within and staged in: its domain
/// policy, its invariants, its staging and its intake.
pub(crate) struct Within {
    pub(crate) egress: Arc<Policy>,
    pub(crate) invariants: Invariants,
    pub(crate) staging: Staging,
    pub(crate) intake: Intake,
}

impl Stored {
    /// The run file `writer` writes, admitting `within`, each work item it
    /// adds with no attempt keyed as `keys` key it, its counted work
    /// counted on `work`.
    pub(crate) fn new(
        writer: goodfetch_engine::run_state::RunState,
        within: Within,
        keys: Keys,
        work: CountedWork,
    ) -> Self {
        let Within {
            egress,
            invariants,
            staging,
            intake,
        } = within;
        Self {
            writer,
            egress,
            invariants,
            staging,
            intake,
            keys,
            work,
        }
    }

    /// Answers `ask`. What an attempt stages, and a redirect hop's
    /// question, is answered with its job, queued with the run-state
    /// writer now, in the loop's order, which its task waits for: `None`.
    /// What gives the crawl work with no attempt, a seed, a batch of seeds
    /// or a re-admission, is written first and then answered: the work
    /// items it added, each keyed as it was added, for the loop to offer
    /// their domains' queues, and what it changed of the crawl's requests
    /// by state.
    pub(crate) async fn answer(&self, ask: Box<Ask>) -> Option<Given> {
        // A program's links batch and a redirect hop promise nothing
        // outside: their attempt's outcome syncs them. A handler's enqueue
        // is synced as it was; #2168 answers it from memory.
        let unsynced = Syncing::Deferred { charged: 0 };
        let this = self.clone();
        match *ask {
            Ask::Seed { seed, answer } => {
                let written = self.writer.write(move |connection| {
                    let mut positions = Positions::keying(connection, this.keys.clone())?;
                    let admitted = this.intake.admitting(|| {
                        admission::seed(
                            connection,
                            &mut positions,
                            (&this.egress, &this.invariants, &this.work),
                            &seed,
                            None,
                        )
                    })?;
                    // With no attempt, none ended: it always answers.
                    Ok((admitted.flatten(), given(connection, &mut positions)?))
                });
                return Some(answered(answer, written.await));
            }
            Ask::Seeds { seeds, answer } => {
                let written = self.writer.write(move |connection| {
                    let mut positions = Positions::keying(connection, this.keys.clone())?;
                    let admitted = this.intake.admitting(|| {
                        admission::seeds(
                            connection,
                            &mut positions,
                            (&this.egress, &this.invariants, &this.work),
                            seeds,
                        )
                    })?;
                    Ok((admitted, given(connection, &mut positions)?))
                });
                return Some(answered(answer, written.await));
            }
            Ask::Readmit { blocked, answer } => {
                let written = self.writer.write(move |connection| {
                    let mut changed = Changed::default();
                    let readmitted = this
                        .intake
                        .admitting(|| work_items::readmit(connection, blocked, &mut changed))?
                        .map(|readmitted| u64::try_from(readmitted.len()).unwrap_or(u64::MAX));
                    // Each keeps its domain key: its queue takes it when
                    // it is filled again.
                    Ok((readmitted, (Vec::new(), changed)))
                });
                return Some(answered(answer, written.await));
            }
            Ask::StagedSeed {
                seed,
                attempt,
                answer,
            } => {
                // No page makes this call again once its attempt ended, so
                // it is synced as a seed added with no attempt is.
                self.queue(answer, Syncing::Required, move |connection| {
                    this.intake.admitting(|| {
                        this.staging.admit(
                            connection,
                            &attempt.crawl_request_id,
                            attempt.lease_epoch,
                            |connection, positions| {
                                admission::seed(
                                    connection,
                                    positions,
                                    (&this.egress, &this.invariants, &this.work),
                                    &seed,
                                    Some(&attempt),
                                )
                            },
                        )
                    })
                })
                .await;
            }
            Ask::Enqueue { enqueue, answer } => {
                self.queue(answer, Syncing::Required, move |connection| {
                    this.enqueue(connection, &enqueue)
                })
                .await;
            }
            Ask::Links { links, answer } => {
                // The write of its own a program's batch asks the writer
                // for; a code-free page's batch rides its outcome's
                // (#2245).
                self.work.count(Row::BatchTransactions);
                self.queue(answer, unsynced, move |connection| {
                    this.links(connection, &links)
                })
                .await;
            }
            Ask::Hop { hop, asks, answer } => {
                if let HopAsk::Check = asks {
                    self.work.count(Row::HopTransactions);
                }
                self.queue(answer, unsynced, move |connection| {
                    this.hop(connection, &hop, asks)
                })
                .await;
            }
        }
        None
    }

    /// Queues `job` with the run-state writer now, synced as `syncing`
    /// says, and answers `answer` with it; a task that is gone, its Run's
    /// cancellation having taken it, is answered nothing.
    async fn queue<T, F>(&self, answer: Answering<T>, syncing: Syncing, job: F)
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        // The asking page's task waits for the writer's answer.
        self.waited(1);
        let queued: Queued<T> = match self.writer.reserve().await {
            Ok(place) => {
                let written = place.write_with(syncing, job);
                Box::pin(async move { written.await.map_err(StateError::from) })
            }
            Err(error) => Box::pin(std::future::ready(Err(error.into()))),
        };
        drop(answer.send(queued));
    }

    /// Counts `pages` waits of a page for the run file's writer
    /// (`F2.page_waits`).
    pub(crate) fn waited(&self, pages: usize) {
        self.work
            .count_many(Row::RunPageWaits, u64::try_from(pages).unwrap_or(u64::MAX));
    }

    /// Stages `enqueue` with its attempt.
    fn enqueue(
        &self,
        connection: &Connection,
        enqueue: &Enqueue,
    ) -> crate::staging::Answered<goodfetch_protocol::foxhound::frontier::Enqueued> {
        let request = &enqueue.request;
        self.staging.admit(
            connection,
            &request.crawl_request_id,
            request.lease_epoch,
            |connection, positions| {
                admission::enqueue(
                    connection,
                    positions,
                    (&self.egress, &self.invariants),
                    (request, enqueue.payload.as_ref()),
                    enqueue.document.as_ref(),
                    &self.work,
                )
            },
        )
    }

    /// Stages the batch `links` with its attempt, and notes with it the
    /// requests it added through links suspected traps: their Provenance
    /// entries wait for the attempt's commit, which writes those of the
    /// marks it committed (ADR 0035).
    fn links(
        &self,
        connection: &Connection,
        links: &LinksBatch,
    ) -> crate::staging::Answered<goodfetch_protocol::foxhound::frontier::EnqueuedLinks> {
        let (work_item, lease_epoch) = (
            links.request.crawl_request_id.as_str(),
            links.request.lease_epoch,
        );
        let answered = self.staging.admit(
            connection,
            work_item,
            lease_epoch,
            |connection, positions| admission::links(connection, positions, links),
        )?;
        Ok(answered.map(|answered| {
            answered.map(|admitted| {
                self.staging
                    .suspected(work_item, lease_epoch, admitted.suspected);
                admitted.links
            })
        }))
    }

    /// A redirect hop's check against the visited set, or the record of
    /// its chain's end.
    fn hop(
        &self,
        connection: &Connection,
        hop: &ChainHop,
        asks: HopAsk,
    ) -> rusqlite::Result<goodfetch_frontier::Hop> {
        match asks {
            HopAsk::Check => goodfetch_frontier::admit_hop(
                connection,
                self.invariants.frontier(),
                &self.egress,
                &hop.crawl_request,
                (&hop.url, &hop.method, hop.body.as_deref()),
            ),
            HopAsk::Reached => goodfetch_frontier::redirect_reached(
                connection,
                &hop.crawl_request,
                &hop.url,
                &hop.method,
                hop.body.as_deref(),
            ),
        }
    }

    /// What the crawl's Run state starts from as it opens: the Session
    /// pool the run file holds, drawing on `entropy`, and its requests by
    /// state, which it counts in memory from then on.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn open(&self, entropy: Entropy) -> Result<(SessionPool, Tally), StateError> {
        Ok(self
            .writer
            .read(move |connection| {
                Ok((
                    SessionPool::open(connection, entropy)?,
                    Tally::read(connection)?,
                ))
            })
            .await?)
    }

    /// The first `at_once` work items not keyed yet, in order: each one's
    /// URL, and how it waits once keyed, its key still empty.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn unkeyed(
        &self,
        at_once: usize,
    ) -> Result<Vec<(String, Waiting)>, StateError> {
        Ok(self
            .writer
            .read(move |connection| work_items::unkeyed(connection, at_once))
            .await?)
    }

    /// Writes `keys`, each work item's ID with the domain key the loop
    /// keyed it under, `found` as the crawl opens or while it runs, where
    /// the write counts as `F3.transactions.sitemap` ([`Unkeyed`]). Keys
    /// promise nothing outside (#2117): the outcome
    /// of the attempt that claims the item syncs them, and a power cut
    /// that takes them leaves the items unkeyed, which the next look keys
    /// again.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn key(
        &self,
        keys: Vec<(String, String)>,
        found: Unkeyed,
    ) -> Result<(), StateError> {
        if matches!(found, Unkeyed::Running) {
            self.work.count(Row::SitemapTransactions);
        }
        Ok(self
            .writer
            .write_batch(
                Syncing::Deferred { charged: 0 },
                SealedBatch::of(work_items::DomainKeys(keys)),
            )
            .await?)
    }

    /// The pending work items of `key`, or of every key when `None`, at
    /// most `bound` of each key, in the order its per-domain queue admits
    /// them.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn waiting(
        &self,
        key: Option<String>,
        bound: usize,
    ) -> Result<Vec<Waiting>, StateError> {
        Ok(self
            .writer
            .read(move |connection| work_items::waiting(connection, key.as_deref(), bound))
            .await?)
    }

    /// Writes the open batch, at a level that batches attempts, and
    /// answers once it is in the file, so a read that follows sees every
    /// write answered before it (ADR 0142); with none open it writes
    /// nothing.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn seal(&self) -> Result<(), StateError> {
        Ok(self.writer.seal().await?)
    }

    /// How many commits the run file's writer acknowledged: a read that
    /// starts after sees them all.
    pub(crate) fn commits(&self) -> u64 {
        *self.writer.commits().borrow()
    }

    /// Whether a work item is pending.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn any_pending(&self) -> Result<bool, StateError> {
        Ok(self.writer.read(work_items::any_pending).await?)
    }

    /// The job of a claim: leases each pending work item of `ids`, in that
    /// order, after writing the rows the draws before it decided, what
    /// they changed in the Session pool, `changes`, and each tier choice's
    /// count of requests served, `counted`.
    pub(super) fn claim(
        ids: Vec<String>,
        (changes, counted): (Changes, Counted),
    ) -> impl FnOnce(&Connection) -> rusqlite::Result<Vec<Leased>> + Send + 'static {
        move |connection| {
            if !changes.is_empty() {
                changes.write(connection)?;
            }
            counted.write(&BatchWriter::over(connection))?;
            work_items::lease(connection, &ids)
        }
    }

    /// How the sitemap's fetch `work_item` under `lease_epoch` ends once
    /// its `document` is read: its entries admitted under the Run's domain
    /// policy and within its invariants (ADR 0093), each keyed as it is
    /// added, in its outcome's one job, so its read asks the writer for no
    /// write of its own.
    pub(crate) fn sitemap_read(
        &self,
        (work_item, lease_epoch): (String, u64),
        document: sitemap::Document,
    ) -> Ending {
        Ending::Sitemap {
            work_item,
            lease_epoch,
            egress: Arc::clone(&self.egress),
            invariants: self.invariants.shared_frontier(),
            document,
            keys: self.keys.clone(),
            work: self.work.clone(),
        }
    }

    /// Queues `outcome`, an ended attempt's, as one job of the run-state
    /// writer (ADR 0138), now, in the loop's order, and answers the job,
    /// which the loop waits for with the others it queued beside it. At a
    /// level that batches attempts, it rides the open batch and is
    /// answered once in it (ADR 0142).
    pub(crate) async fn end(&self, outcome: Outcome) -> QueuedEnd {
        match self.writer.reserve().await {
            Ok(place) => {
                let written =
                    place.write_with(ATTEMPT_ENDED, move |connection| outcome.write(connection));
                Box::pin(async move { written.await.map_err(StateError::from) })
            }
            Err(error) => Box::pin(std::future::ready(Err(error.into()))),
        }
    }

    /// Ends the attempt of `work_item` under `lease_epoch` `unsuccessful`
    /// at `ended_at`, in a job of its own: one whose outcome's job failed.
    /// It rides as an outcome does. Answers how it ended, and what that
    /// changed of the crawl's requests by state.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn end_lost(
        &self,
        (work_item, lease_epoch): (String, u64),
        unsuccessful: Unsuccessful,
        ended_at: PointInTime,
    ) -> Result<(Ended, Changed), StateError> {
        Ok(self
            .writer
            .write_with(ATTEMPT_ENDED, move |connection| {
                let mut changed = Changed::default();
                let ended = attempt_outcome::end(
                    connection,
                    (&work_item, lease_epoch),
                    unsuccessful,
                    &ended_at,
                    &mut changed,
                )?;
                Ok((ended, changed.with_sitemaps(connection)?))
            })
            .await?)
    }

    /// Leases each pending work item of `unsent` and ends it `failed` with
    /// its failure at `ended_at`, in one job: a request its domain's site
    /// asked not to send, which no attempt runs. It rides as an outcome
    /// does, ending the attempts it counts. Answers what that changed of
    /// the crawl's requests by state.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn end_unsent(
        &self,
        unsent: Vec<(String, Failure)>,
        ended_at: PointInTime,
    ) -> Result<Changed, StateError> {
        let ended = Syncing::Attempt {
            ended: u64::try_from(unsent.len()).unwrap_or(u64::MAX),
            bytes: 0,
        };
        Ok(self
            .writer
            .write_with(ended, move |connection| {
                let mut changed = Changed::default();
                for (id, failure) in unsent {
                    for leased in work_items::lease(connection, std::slice::from_ref(&id))? {
                        work_items::end(
                            connection,
                            &id,
                            leased.lease_epoch,
                            Unsuccessful::Failed(failure.clone()),
                            &ended_at,
                            &mut changed,
                        )?;
                    }
                }
                changed.with_sitemaps(connection)
            })
            .await?)
    }

    /// Writes `counted`, the tier choices' counts the run file has not
    /// heard of, in a batch of their own: no lease follows the last draws
    /// to carry them.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn count(&self, counted: Counted) -> Result<(), StateError> {
        Ok(self
            .writer
            .write_batch(Syncing::Deferred { charged: 0 }, SealedBatch::of(counted))
            .await?)
    }

    /// Closes the crawl's intake when no work item is pending or leased,
    /// and answers whether it closed, in one job that first writes
    /// `counted` and gives back each work item still leased, whose attempt
    /// ended with its outcome unwritten: retried while it has retries left
    /// of `retry_max`, else failed with `unwritten`, at `ended_at`, which
    /// it answers as what it changed of the crawl's requests by state. Its
    /// answer promises nothing outside (#2117): the Run's end, which
    /// follows it, syncs.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn close_if_drained(
        &self,
        counted: Counted,
        (unwritten, retry_max): (Failure, u32),
        ended_at: PointInTime,
    ) -> Result<(bool, Changed), StateError> {
        let intake = self.intake.clone();
        Ok(self
            .writer
            .write_deferred(move |connection| {
                counted.write(&BatchWriter::over(connection))?;
                // No attempt is in flight: a work item still leased is
                // one whose attempt ended with its outcome unwritten. It
                // is retried or fails before the crawl may finish.
                let mut changed = Changed::default();
                work_items::recover_leases(
                    connection,
                    &unwritten,
                    retry_max,
                    &ended_at,
                    &mut changed,
                )?;
                Ok((
                    intake.close_if_drained(connection)?,
                    changed.with_sitemaps(connection)?,
                ))
            })
            .await?)
    }

    /// What cut the crawl short of its scope: its bounds and caps, as its
    /// Completeness reasons.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn short_of_scope(&self) -> Result<Vec<CompletenessReason>, StateError> {
        Ok(self.writer.read(completeness::short_of_scope).await?)
    }

    /// Returns the leased work items to pending, without counting them.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn release(&self) -> Result<(), StateError> {
        Ok(self.writer.write(work_items::release_leases).await?)
    }

    /// Writes `journal`, what the per-domain queues changed. A domain's
    /// hold or backoff joins the next group that syncs (ADR 0138's note of
    /// #1926).
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn journal(&self, journal: Journal) -> Result<(), StateError> {
        Ok(self
            .writer
            .write_batch(Syncing::Deferred { charged: 0 }, journal)
            .await?)
    }
}

/// What the admission that ran with `positions` in `connection`'s
/// transaction gave the crawl: the work items it added, keyed, and what
/// adding them changed of the crawl's requests by state.
fn given(connection: &Connection, positions: &mut Positions) -> rusqlite::Result<Given> {
    let mut changed = Changed::default();
    let added = changed.adds(positions);
    Ok((added, changed.with_sitemaps(connection)?))
}

/// Answers `answer` with what its job `written` answered, and gives what
/// the job gave the crawl, nothing when it failed; a task that is gone is
/// answered nothing.
fn answered<T>(answer: asks::Written<T>, written: Result<(T, Given), RunStateError>) -> Given {
    let (answered, given) = match written {
        Ok((answered, given)) => (Ok(answered), given),
        Err(error) => (Err(error.into()), Given::default()),
    };
    drop(answer.send(answered));
    given
}
