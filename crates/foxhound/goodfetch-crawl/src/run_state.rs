//! The crawl's Run state (ADR 0142): what a running crawl decides from,
//! held in memory and owned by one actor, the delivery loop, which is its
//! only caller, so nothing here sits under a lock. The first state it owns
//! is the leasing of work items: the Session each attempt is drawn from the
//! pool, how its tier is chosen, what its Label on its domain learned and
//! the latencies of the first requests that time a race.
//!
//! A lease waits for no write (#2168): each work item's row is the one
//! the crawl kept of a change the run file's readers may not see yet, a
//! retry or a work item an attempt added, else one read on a reader, or,
//! at a level that batches attempts, from every pending work item held
//! whole while they fit within the Run's memory budget, which reads
//! nothing (#2169; [`kept`]); the draw runs here, in memory, on it; and its claim is a writer's job the
//! loop queues and does not wait for, which leases only a work item still
//! as its row was read, and before which no later job runs. The crawl
//! keeps each work item it leased out of the pending ones it reads until
//! its attempt settles, and each one that ended or was merged until a
//! read sees that change. What a draw changes is written as rows the next
//! time the loop queues a job, a lease's or an outcome's, and once more
//! when the crawl has nothing left to lease, so a draw adds no job of its
//! own and the file follows memory by one job at most.
//!
//! It also owns ADR 0140's F3 whole (#2167): every admission, the writes
//! that claim work items and those that end their attempts go through it.
//! The visited set and the pending work items are still the run file's, so
//! those operations are jobs of the Run's run-state writer, each queued by
//! the loop in its own order ([`Stored`]); #2168 and #2169 move what they
//! read into memory behind the same operations. Every other task reaches
//! it over the loop's one mailbox ([`Asking`]) and queues none of these
//! jobs with the writer itself. Its operations name no run file's type,
//! and fail with [`StateError`]. One job of a running crawl is not its
//! own yet and stays with the page's task that queues it: a handler
//! attempt's lesson to the Frontier (#2169).
//!
//! It counts the crawl's requests by state ([`Tally`]): read once from
//! the run file as the crawl opens, then counted from what each job
//! answers that it changed, so a live crawl's progress is built from this
//! state and reads no count from the run file (#2167).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures_util::FutureExt as _;
use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;

use goodfetch_engine::admission::Waiting;
use goodfetch_engine::governor::RunShare;
use goodfetch_engine::ledger::{ChargeError, UnjournaledCharges};
use goodfetch_engine::local_addresses::{SessionAddresses, Unavailable};
use goodfetch_engine::run_state::{BatchRows, BatchWriter, RunStateError};
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::{Choosing, HeldPlace, HttpFrom, PooledSession, SetTier, TierChoice};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::governor::WorkKind;
use goodfetch_session_pool::{
    Effect, LabelPools, PoolKey, PoolSettings, SendsFrom, SessionPool, Wanted,
};
use rusqlite::Connection;

use crate::labels;
use crate::tier_choice::{AttemptPlan, Sampled, TierChoices, Unit, Weighed};
use crate::tier_race::Latencies;
use crate::work_items::{Again, Changed, Leased};
use kept::{Kept, Leasing};

pub(crate) mod admission;
mod asks;
mod kept;
mod stored;
mod tally;
pub(crate) mod teacher;

pub(crate) use asks::{Asked, Asking, Escalated, Question, mailbox};
pub(crate) use stored::{Opening, QueuedEnd, Ran, Stored, Unkeyed, Within};
pub(crate) use tally::Tally;

/// Why the crawl's Run state did not take an operation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StateError {
    /// The crawl ended: no loop owns its Run state any more.
    #[error("the crawl ended, so nothing takes its Run state's operations")]
    Closed,
    /// The run file did not take a write or give a read.
    #[error(transparent)]
    RunFile(#[from] RunStateError),
    /// A claim's charges were refused or could not be journaled.
    #[error(transparent)]
    Charge(#[from] ChargeError),
}

impl StateError {
    /// The same as the run file's own error, for a seam that speaks it:
    /// what is no run file's error reads as its writer closed.
    pub(crate) fn run_file(self) -> RunStateError {
        match self {
            Self::RunFile(error) | Self::Charge(ChargeError::State(error)) => error,
            Self::Closed | Self::Charge(_) => RunStateError::Closed,
        }
    }
}

/// The local addresses a Session drawn from the pool holds: `None`
/// without `egress.local_addresses` (ADR 0044).
pub(crate) type Local = Option<Result<SessionAddresses, Unavailable>>;

/// How an attempt's tier is chosen, settled as its Session is drawn.
pub(crate) enum Chosen {
    /// On this plan.
    Planned(AttemptPlan),
    /// As a race, unless its Label on its domain, which learned this plan,
    /// is due a sample, which it then takes.
    Racing(AttemptPlan),
}

/// A Session drawn from the pool for an attempt, as it runs in the
/// browser, with its local addresses.
pub(crate) struct DrawnSession {
    pub(crate) session: ServingSession,
    pub(crate) in_browser: PooledSession,
    pub(crate) local: Local,
    /// The client it sends from.
    pub(crate) sends_from: SendsFrom,
}

/// A work item leased, with the Session drawn for its attempt, how its
/// tier is chosen, and, for a sample whose `http` run is sent from the
/// transport (`fetch.http_from`), or a race whose first request is
/// (#2421), the Session drawn for that run or request, which shares
/// nothing with the first.
pub(crate) struct Drawn {
    pub(crate) leased: Leased,
    pub(crate) session: DrawnSession,
    pub(crate) chosen: Chosen,
    pub(crate) transport: Option<DrawnSession>,
    /// The Session whose own transport reads the site files the attempt
    /// reads, robots.txt and the AI-use signal files, when neither of the
    /// others sends from one (#2422), of the Run's pool when the attempt's
    /// pool holds a login (#2418); none when its pool has no room for one
    /// and none is open, and the attempt's Session reads them from its own
    /// transport, which never happens to a pool that holds a login.
    pub(crate) site_files: Option<DrawnSession>,
}

/// What the Run's settings say of a draw, resolved as the Run opened, and
/// the Run's wall clock as the draw's lease was asked.
pub(crate) struct Drawing<'a> {
    pub(crate) labels: &'a labels::Declared,
    pub(crate) settings: PoolSettings,
    pub(crate) run_tier: SetTier,
    /// The Run's `fetch.http_from`, which a Label's own setting overrides.
    pub(crate) http_from: HttpFrom,
    pub(crate) races: bool,
    /// Whether the Run's crawl requests read site files, robots.txt or the
    /// AI-use signal files, and the Run has a transport to read them with.
    pub(crate) reads_site_files: bool,
    /// The wall clock, and in milliseconds since the Unix epoch.
    pub(crate) wall: (SystemTime, u64),
    /// The Run's share of the Resource governor, which a draw tries for a
    /// browser-page place before it opens a Session of the browser client
    /// (#2178).
    pub(crate) places: &'a RunShare,
}

/// A work item leased whose Session is still to be drawn: its pool, how
/// its tier is chosen, and the clients its draw wants.
struct Planned {
    leased: Leased,
    key: PoolKey,
    chosen: Chosen,
    sends_from: SendsFrom,
    /// Whether a Session of the transport is drawn beside it (#2421).
    from_transport: bool,
}

/// The tier choices whose count of requests served moved since the run
/// file last heard of them, by Label and domain key: the latest of each.
#[derive(Default)]
pub(crate) struct Counted(BTreeMap<(String, String), Unit>);

impl Counted {
    /// Whether none moved.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl BatchRows for Counted {
    fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()> {
        if self.0.is_empty() {
            return Ok(());
        }
        let mut record = writer.statement(crate::tier_choice::RECORD)?;
        for ((label, domain_key), unit) in self.0 {
            record.write((label, domain_key, crate::tier_choice::encoded(&unit)))?;
        }
        Ok(())
    }
}

/// What the Run's Labels learned under `auto`, with the counts the run
/// file has not heard of yet.
#[derive(Default)]
pub(crate) struct Learned {
    choices: TierChoices,
    /// The choices that counted a request since the run file last heard
    /// of them.
    counted: Counted,
}

impl Learned {
    /// What a crawl that learned `choices` goes on from.
    pub(crate) fn from(choices: TierChoices) -> Self {
        Self {
            choices,
            counted: Counted::default(),
        }
    }

    /// How an attempt of a request of `label` at `url` on `domain_key`,
    /// its HTTP sent as `http_from` says, runs at `now` (milliseconds since
    /// the Unix epoch). A choice whose count of requests served moved is
    /// kept for the run file, so a resumed crawl goes on counting toward
    /// its refresh.
    pub(crate) fn serve(
        &mut self,
        label: &str,
        domain_key: &str,
        url: &str,
        (http_from, now): (HttpFrom, u64),
    ) -> AttemptPlan {
        let (plan, unit) = self
            .choices
            .serve_attempt(label, domain_key, url, http_from, now);
        if let Some(unit) = unit {
            self.counted
                .0
                .insert((label.to_owned(), domain_key.to_owned()), unit.clone());
        }
        plan
    }

    /// Takes the sample `sampled`, as the weighing judged its runs at
    /// `now`; answers what its Label on its domain now learned, which its
    /// attempt's outcome writes into the run file in place of any count
    /// kept for it.
    pub(crate) fn classify(&mut self, sampled: &Sampled, weighed: Weighed, now: u64) -> Unit {
        let unit = self.choices.classify(sampled, weighed, now).clone();
        self.counted
            .0
            .remove(&(sampled.label.clone(), sampled.domain_key.clone()));
        unit
    }

    /// Samples `label` on `domain_key` again at once, its transport having
    /// been refused (#2419); answers the unit its choice was dropped for,
    /// which its attempt's outcome writes into the run file in place of
    /// any count kept for it.
    pub(crate) fn invalidate(&mut self, label: &str, domain_key: &str) -> Option<Unit> {
        let unit = self.choices.invalidate(label, domain_key).cloned();
        if unit.is_some() {
            self.counted
                .0
                .remove(&(label.to_owned(), domain_key.to_owned()));
        }
        unit
    }

    /// The choices that counted a request since the last call, for the
    /// run file.
    pub(crate) fn counted(&mut self) -> Counted {
        std::mem::take(&mut self.counted)
    }
}

/// At most `bound` of each key of `waiting`, each key's in the order its
/// per-domain queue admits them (`forefront` ones latest first, then
/// highest priority first, then in order), all of them in order when
/// `in_order`, as the run file's read of them answers.
fn in_queue_order(mut waiting: Vec<Waiting>, bound: usize, in_order: bool) -> Vec<Waiting> {
    waiting.sort_by_key(|waiting| {
        (
            waiting.key.clone(),
            std::cmp::Reverse(waiting.forefront.then_some(waiting.order)),
            std::cmp::Reverse(waiting.priority),
            waiting.order,
        )
    });
    let mut taken: HashMap<String, usize> = HashMap::new();
    waiting.retain(|waiting| {
        let count = taken.entry(waiting.key.clone()).or_default();
        *count += 1;
        *count <= bound
    });
    if in_order {
        waiting.sort_by_key(|waiting| waiting.order);
    }
    waiting
}

/// The crawl's Run state, which its delivery loop owns.
pub(crate) struct RunState {
    pool: SessionPool,
    learned: Learned,
    latencies: Latencies,
    /// Its requests by state. Each value of the Run's progress shares the
    /// counts it was shown, so a change copies them once and no reader
    /// waits for the loop.
    tally: Arc<Tally>,
    stored: Stored,
    /// Its pending work items as the run file's readers may not see them
    /// yet, and every one whole while they fit within the Run's memory
    /// budget (#2169).
    kept: Kept,
    /// The claims queued with the run-state writer whose answers the crawl
    /// has not heard yet: no page waits for one (#2168). Only `&mut self`
    /// reaches them, so their lock is never contended: it only lets a
    /// shared Run state cross threads.
    claims: tokio::sync::Mutex<FuturesUnordered<Claim>>,
    /// The work items leased whose request needs the browser and waits
    /// for a Session of the browser client to have room, or for a
    /// browser-page place, in the order they were leased (#2178).
    undrawn: VecDeque<Planned>,
}

/// A claim queued with the run-state writer: answers the IDs it did not
/// lease.
pub(crate) type Claim = Pin<Box<dyn Future<Output = Result<Vec<String>, ChargeError>> + Send>>;

impl RunState {
    /// The state of a crawl that draws from `pool`, learned `choices` and
    /// counts its requests on from `tally`, what is still the run file's
    /// read and written through `stored`, holding its pending work items
    /// `whole` when they fit within the Run's memory budget.
    pub(crate) fn new(
        pool: SessionPool,
        choices: TierChoices,
        (tally, whole): Opening,
        stored: Stored,
    ) -> Self {
        Self {
            pool,
            learned: Learned::from(choices),
            latencies: Latencies::default(),
            tally: Arc::new(tally),
            stored,
            kept: Kept::new(whole),
            claims: tokio::sync::Mutex::new(FuturesUnordered::new()),
            undrawn: VecDeque::new(),
        }
    }

    /// What is still the run file's, as the loop reads and writes it.
    pub(crate) fn stored(&self) -> &Stored {
        &self.stored
    }

    /// The pending work items of `key`, or of every key when `None`, at
    /// most `bound` of each key, in the order its per-domain queue admits
    /// them, as the crawl knows them, with no seal (ADR 0142): the run
    /// file's, less each the crawl leased, ended or saw merged in a change
    /// the read does not see yet, with each it knows pending in such a
    /// change, a retry or a work item an attempt added. While the crawl
    /// holds every pending work item whole, it reads nothing (#2169).
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn waiting(
        &mut self,
        key: Option<String>,
        bound: usize,
    ) -> Result<Vec<Waiting>, StateError> {
        self.kept.seen(self.stored.commits());
        // Those it leaves out could take a key's whole bound: it reads
        // past them.
        let read = bound.saturating_add(self.kept.held());
        let mut waiting = if let Some(whole) = self.kept.whole(|| self.stored.frontier_bytes()) {
            whole.waiting(key.as_deref(), read)
        } else {
            self.stored.lease_read();
            self.stored.waiting(key.clone(), read).await?
        };
        waiting.retain(|waiting| !self.kept.holds(&waiting.id));
        let mut known: HashSet<String> = waiting.iter().map(|waiting| waiting.id.clone()).collect();
        for pending in self.kept.pending(key.as_deref()) {
            if known.insert(pending.id.clone()) {
                waiting.push(pending.clone());
            }
        }
        Ok(in_queue_order(waiting, bound, key.is_none()))
    }

    /// Whether a work item is pending, as the crawl knows them, with no
    /// seal (ADR 0142): one it knows pending in a change a read does not
    /// see yet, else one the run file's read finds pending that the crawl
    /// did not lease, end or see merged since.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn any_pending(&mut self) -> Result<bool, StateError> {
        self.kept.seen(self.stored.commits());
        if self.kept.pending(None).next().is_some() {
            return Ok(true);
        }
        let read = self.kept.held().saturating_add(1);
        let pending = if let Some(whole) = self.kept.whole(|| self.stored.frontier_bytes()) {
            whole.pending(read)
        } else {
            self.stored.lease_read();
            self.stored.pending(read).await?
        };
        Ok(pending.iter().any(|id| !self.kept.holds(id)))
    }

    /// Writes the open batch and answers once it is in the file, so a read
    /// that follows sees every write answered before it, every claim
    /// included.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) async fn seal(&mut self) -> Result<(), StateError> {
        self.stored.seal().await?;
        self.kept.sealed();
        self.heard()
    }

    /// Lets the crawl's pending work items go from memory for good: some
    /// may be pending that it did not hear of, as a work item keyed once
    /// the crawl is idle. Its reads find them in the run file from then
    /// on (#2169).
    pub(crate) fn spill(&mut self) -> &mut Self {
        self.kept.spill();
        self
    }

    /// Keeps each of `ids` out of [`waiting`](Self::waiting) until a read
    /// sees the change a job of the writer made to it: it ended, or a rule
    /// merged it (ADR 0142), in the commit after those the writer had
    /// acknowledged when the job ran, given with it (#2679). One whose
    /// attempt is in flight stays leased until it settles.
    pub(crate) fn gone(&mut self, ids: impl IntoIterator<Item = (String, u64)>) {
        for (id, committed) in ids {
            self.kept.gone(id, committed);
        }
    }

    /// Takes how the attempt of `leased` settled, once its outcome's job
    /// answered, which ran once `committed` commits were acknowledged: its
    /// work item waits again as `again` says, as its next lease takes it,
    /// or else it ended.
    pub(crate) fn settled(&mut self, leased: Leased, again: Again, committed: u64) {
        self.kept
            .settled(leased, again, committed, || self.stored.frontier_bytes());
    }

    /// Takes `added`, the work items a job of the writer that ran once
    /// `committed` commits were acknowledged added or made pending again,
    /// keyed, as each one's next lease takes it, and answers how each waits
    /// for its domain's queue, in order.
    pub(crate) fn added(&mut self, added: Vec<Leased>, committed: u64) -> Vec<Waiting> {
        added
            .into_iter()
            .map(|leased| {
                self.kept
                    .added(leased, committed, || self.stored.frontier_bytes())
            })
            .collect()
    }

    /// Hears each claim the writer answered: one that failed fails the
    /// crawl's next operation, as a claim the loop waited for would have,
    /// and a work item one did not lease is traced, its attempt ending
    /// with its lease lost.
    fn heard(&mut self) -> Result<(), StateError> {
        let claims = self.claims.get_mut();
        let mut lost = false;
        while let Some(Some(answered)) = claims.next().now_or_never() {
            for id in answered? {
                tracing::debug!(work_item = %id, "a claim found its work item no longer as it was read");
                lost = true;
            }
        }
        if lost {
            // Memory no longer knows how that work item stands.
            self.kept.spill();
        }
        Ok(())
    }

    /// Counts what a job of the run file `changed` of the crawl's requests
    /// by state.
    pub(crate) fn changed(&mut self, changed: Changed) {
        if !changed.is_empty() {
            Arc::make_mut(&mut self.tally).change(changed);
        }
    }

    /// The crawl's requests by state as counted now, for a value of its
    /// Run's progress to share.
    pub(crate) fn tally(&self) -> Arc<Tally> {
        Arc::clone(&self.tally)
    }

    /// Answers `question`, what another task asks of the crawl's Run
    /// state: an escalated attempt's draw from memory
    /// ([`RunState::escalated`]), any other from the run file
    /// ([`Stored::answer`]), counting what its write changed: the work
    /// items it added, each keyed, for the loop to offer their domains'
    /// queues, when the loop waited for the write itself.
    pub(crate) async fn answer(&mut self, question: Box<Question>) -> Option<Vec<Waiting>> {
        let ask = match *question {
            Question::Stored(ask) => ask,
            Question::Escalated { escalated, answer } => {
                let drawn = self.escalated(&escalated);
                // The attempt no longer waits for it: it served nothing.
                if let Err(drawn) = answer.send(drawn) {
                    self.pool.settle(
                        &drawn.session,
                        &escalated.domain_key,
                        Effect::Neutral,
                        &escalated.settings,
                    );
                }
                return None;
            }
        };
        let (added, changed) = self.stored.answer(ask).await?;
        self.changed(changed);
        // The ask was answered just now: its job ran before at most the
        // commits acknowledged now, and a later stamp only keeps it a
        // commit longer.
        let committed = self.stored.commits();
        Some(self.added(added, committed))
    }

    /// Draws the Session whose client is its browser that an attempt which
    /// escalated from a Session's own transport runs again in (#2416), as
    /// `escalated` says: as a lease's draw does, so one already open serves
    /// it while the pool has no room, and the pool keeps it for the run
    /// file as it keeps any draw. The attempt holds the browser-page place
    /// its page takes, so the draw may open a Session for it (#2178).
    fn escalated(&mut self, escalated: &Escalated) -> DrawnSession {
        let wanted = Wanted {
            domain_key: &escalated.domain_key,
            host: &escalated.host,
            sends_from: SendsFrom::Browser,
        };
        let session = self
            .pool
            .draw(
                &escalated.pool,
                &wanted,
                &escalated.settings,
                escalated.now,
                || true,
            )
            .expect("invariant: a draw that holds a place always draws a Session");
        self.drawn_session(
            (&escalated.labels.pools, &escalated.pool),
            session,
            (SendsFrom::Browser, escalated.place.clone()),
        )
    }

    /// Leases each pending work item of `asked`, by ID with the domain key
    /// its queue admitted it under, in that order, and draws, in memory,
    /// the Session that serves each and how its tier is chosen. Each one's
    /// row is the one the crawl kept of a change the run file's readers
    /// may not see yet, else read in one read transaction beside the
    /// writer; one whose attempt is in flight, or that ended or was merged
    /// in such a change, is left out. Its claim is one job of the
    /// run-state writer, queued now and, at a level that batches attempts,
    /// not waited for (#2168); at one where each attempt is a batch of its
    /// own, it is in the file before the fetch, as before. The claim also
    /// journals `charges`, what `run` charged for them (ADR 0138), and
    /// writes the rows the draws before it decided: what they changed in
    /// the pool, and each tier choice's count of requests served. Every
    /// job queued after it runs after it, its attempts' outcomes included,
    /// and a work item it finds changed since its row was read is not
    /// leased: its attempt's outcome then finds its lease lost.
    ///
    /// # Errors
    ///
    /// The run file's, or an earlier claim's that failed.
    pub(crate) async fn lease(
        &mut self,
        run: &Run,
        charges: UnjournaledCharges,
        asked: Vec<(String, String)>,
        drawing: &Drawing<'_>,
    ) -> Result<Vec<Drawn>, StateError> {
        let leased = self.leasable(asked).await?;
        let claim = run.queue_charged(charges, self.claim(&leased)).await?;
        if self.stored.claims_first() {
            // Each claim is in the file before its fetch, as each attempt's
            // own batch promises: one page wait (`F2.page_waits.full`).
            self.stored.waited(leased.len());
            let answered = claim.await;
            self.claiming(Box::pin(std::future::ready(answered)), &leased);
        } else {
            self.claiming(Box::pin(claim), &leased);
        }
        let mut drawn = Vec::with_capacity(leased.len());
        for leased in leased {
            let planned = self.plan(drawing, leased);
            match self.draw(drawing, planned) {
                Ok(one) => drawn.push(one),
                Err(waits) => self.undrawn.push_back(*waits),
            }
        }
        Ok(drawn)
    }

    /// Draws again, in the order they were leased, the Sessions of at most
    /// `room` work items whose request waits for a Session of the browser
    /// client to have room or for a browser-page place (#2178): those
    /// drawn now, each still waiting kept in its turn.
    pub(crate) fn redraw(&mut self, drawing: &Drawing<'_>, room: usize) -> Vec<Drawn> {
        let mut drawn = Vec::new();
        let mut still = VecDeque::with_capacity(self.undrawn.len());
        while let Some(planned) = self.undrawn.pop_front() {
            if drawn.len() == room {
                still.push_back(planned);
                continue;
            }
            match self.draw(drawing, planned) {
                Ok(one) => drawn.push(one),
                Err(waits) => still.push_back(*waits),
            }
        }
        self.undrawn = still;
        drawn
    }

    /// How many work items leased wait for their Session to be drawn
    /// ([`redraw`](Self::redraw)).
    pub(crate) fn undrawn(&self) -> usize {
        self.undrawn.len()
    }

    /// Whether the work item `id` was leased and waits for its Session to
    /// be drawn.
    pub(crate) fn waits_to_draw(&self, id: &str) -> bool {
        self.undrawn
            .iter()
            .any(|planned| planned.leased.crawl_request.id == id)
    }

    /// Each pending work item of `asked`, by ID with the domain key its
    /// queue admitted it under, in that order, as its next lease takes it:
    /// the row the crawl kept of a change the run file's readers may not
    /// see yet, else the one read in one read transaction beside the
    /// writer. One whose attempt is in flight, or that ended or was merged
    /// in such a change, is left out.
    ///
    /// # Errors
    ///
    /// The run file's, or an earlier claim's that failed.
    pub(crate) async fn leasable(
        &mut self,
        asked: Vec<(String, String)>,
    ) -> Result<Vec<Leased>, StateError> {
        self.heard()?;
        self.kept.seen(self.stored.commits());
        let mut kept = HashMap::new();
        let mut unread = Vec::new();
        for (id, _) in &asked {
            match self.kept.leasing(id) {
                Leasing::Kept(leased) => {
                    kept.insert(id.clone(), *leased);
                }
                Leasing::Unleasable => {}
                Leasing::Unknown => unread.push(id.clone()),
            }
        }
        let mut read: HashMap<String, Leased> = if unread.is_empty() {
            HashMap::new()
        } else if let Some(whole) = self.kept.whole(|| self.stored.frontier_bytes()) {
            whole.leasable(&unread)
        } else {
            self.stored.lease_read();
            self.stored
                .leasable(unread)
                .await?
                .into_iter()
                .map(|leased| (leased.crawl_request.id.clone(), leased))
                .collect()
        };
        Ok(asked
            .into_iter()
            .filter_map(|(id, key)| {
                let mut leased = kept.remove(&id).or_else(|| read.remove(&id))?;
                // A work item keyed in a batch the read does not see yet
                // reads with no key: its queue's is the one it was keyed
                // under.
                leased.waiting.key = key;
                Some(leased)
            })
            .collect())
    }

    /// The job that claims `leased`, as [`leasable`](Self::leasable) read
    /// them, and writes the rows the draws before it decided: what they
    /// changed in the pool, and each tier choice's count of requests
    /// served. It answers the IDs it did not lease: each one changed since
    /// its row was read.
    pub(crate) fn claim(
        &mut self,
        leased: &[Leased],
    ) -> impl FnOnce(&Connection) -> rusqlite::Result<Vec<String>> + Send + 'static {
        let claims = leased
            .iter()
            .map(|leased| (leased.crawl_request.id.clone(), leased.lease_epoch))
            .collect();
        Stored::claim(claims, (self.pool.changes(), self.learned.counted()))
    }

    /// Takes `claim`, the queued [`claim`](Self::claim) of `leased`, which
    /// the crawl hears at its next operation, and keeps each of `leased`
    /// leased until its attempt settles.
    pub(crate) fn claiming(&mut self, claim: Claim, leased: &[Leased]) {
        self.claims.get_mut().push(claim);
        for leased in leased {
            self.kept.claimed(leased.crawl_request.id.clone());
        }
    }

    /// The Session pool.
    pub(crate) fn pool(&mut self) -> &mut SessionPool {
        &mut self.pool
    }

    /// Chooses how the tier of `leased` is chosen, and so the client its
    /// Session is drawn for, as its Label and the Run's `fetch.tier` choose
    /// and, under `auto`, on the browser for a domain a Challenge moved
    /// there (ADR 0021), else, when the Run races its tiers, as a sample
    /// while its Label on its domain is due one, else as a race, else as
    /// its Label on its domain learned, its HTTP sent from the client its
    /// Label's `fetch.http_from`, else the Run's, says, or from the browser
    /// when its pool holds a login (#2418). A choice whose count of
    /// requests served moved is kept for the run file, so a resumed crawl
    /// goes on counting toward its refresh; it is counted once, however
    /// long its draw waits.
    fn plan(&mut self, drawing: &Drawing<'_>, leased: Leased) -> Planned {
        let request = &leased.crawl_request;
        let labels = drawing.labels;
        let key = labels.pools.pool(&request.label);
        let to_browser = self.pool.browser(&leased.waiting.key);
        // A sitemap's fetch runs no handler and takes no tier choice: its
        // attempt fetches it on the Run's HTTP tier.
        let choosing =
            (!leased.sitemap).then(|| labels.settings.choose(&request.label, drawing.run_tier));
        // A request keeping its URL's fragment is served only by a tier
        // that runs the page's scripts, which the engine's choice is then
        // (ADR 0120); the start refused it on any forced tier but those.
        let keeps_fragment = !leased.sitemap && labels.fragments.keeps(&request.label);
        let browser = AttemptPlan::On(TierChoice {
            tier: FetchTier::Browser,
            forced: None,
        });
        let http_from = labels.http_from(&request.label, drawing.http_from);
        let from = if http_from.from_transport() {
            SendsFrom::Transport
        } else {
            SendsFrom::Browser
        };
        let learned = &mut self.learned;
        let mut learned = || {
            learned.serve(
                &request.label,
                &leased.waiting.key,
                &request.url,
                (http_from, drawing.wall.1),
            )
        };
        let chosen = match choosing {
            None => Chosen::Planned(AttemptPlan::Sitemap),
            Some(Choosing::Engine) if keeps_fragment => Chosen::Planned(browser),
            Some(Choosing::Engine) if drawing.races && !to_browser => Chosen::Racing(learned()),
            Some(Choosing::Forced(choice)) => Chosen::Planned(AttemptPlan::On(choice)),
            Some(Choosing::Engine) if to_browser => Chosen::Planned(browser),
            Some(Choosing::Engine) => Chosen::Planned(learned()),
        };
        // The Session is drawn for the client its plan sends from, so it
        // never serves a request of the other one. A race, and a sample
        // that takes a Label's turn to race, serve their pages from the
        // browser; a race's first request leaves from where its Label
        // learned to leave from, a Session's own transport drawn beside
        // the race's when that is the transport (#2421).
        let sends_from = match &chosen {
            Chosen::Planned(plan) => plan.sends_from(from),
            Chosen::Racing(_) => SendsFrom::Browser,
        };
        let from_transport = match &chosen {
            Chosen::Planned(AttemptPlan::Sample(sampled))
            | Chosen::Racing(AttemptPlan::Sample(sampled)) => sampled.from == SendsFrom::Transport,
            Chosen::Racing(learned) => learned.sends_from(from) == SendsFrom::Transport,
            Chosen::Planned(_) => false,
        };
        Planned {
            leased,
            key,
            chosen,
            sends_from,
            from_transport,
        }
    }

    /// Draws the Session of the pool that serves `planned`, for the client
    /// its plan sends from, preferring one whose last request went to its
    /// host. A Session of the browser client opens only while a
    /// browser-page place can be had, which the draw tries the Run's share
    /// for and holds for that Session's first page; with none, and no open
    /// Session of that client with room, it answers `planned` back to wait
    /// (#2178), boxed, as only a request that waits pays for it. A sample
    /// whose `http` run is sent from the transport, and a race whose Label
    /// learned to send its HTTP from there, draw a second Session in the
    /// same draw, for that run or the race's first request (#2421). While the Run reads site files, an attempt whose
    /// Sessions all send from the browser draws, in the same draw, a
    /// Session of the transport to read them, one already open while there
    /// is one (#2422).
    fn draw(&mut self, drawing: &Drawing<'_>, planned: Planned) -> Result<Drawn, Box<Planned>> {
        let labels = drawing.labels;
        let (key, sends_from) = (&planned.key, planned.sends_from);
        let domain_key = planned.leased.waiting.key.as_str();
        let host = goodfetch_frontier::host_of(&planned.leased.crawl_request.url);
        let mut place = None;
        let wanted = Wanted {
            domain_key,
            host,
            sends_from,
        };
        let session = self
            .pool
            .draw(key, &wanted, &drawing.settings, drawing.wall.0, || {
                place = drawing.places.try_admit(WorkKind::BrowserPage);
                place.is_some()
            });
        let Some(session) = session else {
            return Err(Box::new(planned));
        };
        let place = place.map(HeldPlace::holding).unwrap_or_default();
        let session = self.drawn_session((&labels.pools, key), session, (sends_from, place));
        let transport_wanted = Wanted {
            sends_from: SendsFrom::Transport,
            ..wanted
        };
        let transport = planned.from_transport.then(|| {
            let transport = self.transport(key, &transport_wanted, drawing);
            self.drawn_session(
                (&labels.pools, key),
                transport,
                untaken(SendsFrom::Transport),
            )
        });
        // A site file is never read from a browser page, nor from a client
        // other than a Session's own transport (#2422). A pool that holds a
        // login has its read by a Session of the Run's pool, drawn even
        // when it must make room, so its own Sessions' transports send
        // nothing (#2418).
        let login = labels.login(&planned.leased.crawl_request.label).is_some();
        let reading = if login { PoolKey::Run } else { key.clone() };
        let site_files =
            (drawing.reads_site_files && sends_from == SendsFrom::Browser && transport.is_none())
                .then(|| {
                    if login {
                        Some(self.transport(&reading, &transport_wanted, drawing))
                    } else {
                        self.pool.draw_reusing(
                            &reading,
                            domain_key,
                            SendsFrom::Transport,
                            &drawing.settings,
                            drawing.wall.0,
                        )
                    }
                })
                .flatten()
                .map(|reader| {
                    self.drawn_session(
                        (&labels.pools, &reading),
                        reader,
                        untaken(SendsFrom::Transport),
                    )
                });
        let Planned { leased, chosen, .. } = planned;
        Ok(Drawn {
            leased,
            session,
            chosen,
            transport,
            site_files,
        })
    }

    /// A Session of the pool `key` drawn for `wanted`, a request sent from
    /// a Session's own transport, which takes no browser-page place.
    fn transport(
        &mut self,
        key: &PoolKey,
        wanted: &Wanted<'_>,
        drawing: &Drawing<'_>,
    ) -> ServingSession {
        self.pool
            .draw(key, wanted, &drawing.settings, drawing.wall.0, || true)
            .expect("invariant: a draw for a Session's own transport never waits for a place")
    }

    /// `session`, drawn from the pool `key` of `pools`, sending from
    /// `sends_from`, as it runs in the browser holding `place` for its
    /// first page, with its local addresses.
    fn drawn_session(
        &self,
        (pools, key): (&LabelPools, &PoolKey),
        session: ServingSession,
        (sends_from, place): (SendsFrom, HeldPlace),
    ) -> DrawnSession {
        let in_browser = PooledSession {
            id: session.id.clone(),
            source: pools.source(key),
            address_ordinal: self.pool.address_ordinal(&session.id),
            place,
        };
        let local = self.pool.local_addresses(&session.id);
        DrawnSession {
            session,
            in_browser,
            local,
            sends_from,
        }
    }

    /// The choices a draw counted a request on since the last call, for
    /// the run file.
    pub(crate) fn counted(&mut self) -> Counted {
        self.learned.counted()
    }

    /// Takes the sample `sampled`, as [`Learned::classify`] does.
    pub(crate) fn classify(&mut self, sampled: &Sampled, weighed: Weighed, now: u64) -> Unit {
        self.learned.classify(sampled, weighed, now)
    }

    /// Samples `label` on `domain_key` again at once, its transport having
    /// been refused ([`Learned::invalidate`]).
    pub(crate) fn invalidate(&mut self, label: &str, domain_key: &str) -> Option<Unit> {
        self.learned.invalidate(label, domain_key)
    }

    /// Takes `latency`, that of the first request of a race of `label` on
    /// `domain_key`.
    pub(crate) fn time(&mut self, label: &str, domain_key: &str, latency: Duration) {
        self.latencies.record(label, domain_key, latency);
    }

    /// How long a race of `label` on `domain_key` waits for its browser's
    /// navigation.
    pub(crate) fn race_delay(&self, label: &str, domain_key: &str) -> Duration {
        self.latencies.delay(label, domain_key)
    }
}

/// A draw of a Session sending from `sends_from` that holds no
/// browser-page place.
fn untaken(sends_from: SendsFrom) -> (SendsFrom, HeldPlace) {
    (sends_from, HeldPlace::default())
}

#[cfg(test)]
mod tests;
