//! A crawl's per-domain queues (ADRs 0004, 0005, 0021 and 0087), the gate
//! in admission Foxhound fills: each domain key's waiting requests in a
//! queue bounded at [`BOUND`], a `forefront` request ahead of every request
//! its domain held when it came, then highest priority first and then in
//! their order in the crawl. A free request slot goes to the ready domain
//! with the fewest of the Run's requests in flight, the first in turn among
//! those with as few, or at `crawl.domain_order: round_robin` to each ready
//! domain in turn, one request each (the `order` module): either way a
//! heavy domain never starves the others, and neither a priority nor
//! `forefront` crosses domains. A full queue keeps the requests that go
//! first: one that goes ahead of its last takes that one's place, which
//! waits in the run file until the queue drains and fills again in order.
//! A domain waiting out a `Retry-After` or a backoff is not ready: it keeps
//! its place, holds no capacity, and the next ready domain goes; so is a
//! domain the per-user Politeness holds, until its deadline or a wake
//! (the `politeness` module's rule, ADR 0053). A domain
//! that holds no request, none in flight and no wait is released, so the
//! state stays bounded.
//!
//! The waiting requests are the crawl's pending work items, which the run
//! file keeps; what the queues add is each domain's wait and backoff, which
//! they journal in their own table and open again from on a resume. The
//! backoff is the `politeness` module's: while the Run shares no Politeness,
//! its rule pacing nothing (ADR 0138), the queues back each domain off
//! themselves by that rule over the state they keep of it in memory, as an
//! ending of its requests says: the backoff under `politeness.backoff.*`, a
//! `Retry-After`, and a 429's halving of the count in flight.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::admission::{
    Admitted, Ended, Gate, Gates, Held, Journal, Keys, QueuedGroup, Waiting,
};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::politeness::{Ending, SharedRule, milliseconds};
use goodfetch_engine::run_state::{BatchRows, BatchWriter, RunStateTables, SealedBatch};
use goodfetch_engine::settings::Resolved;
use rusqlite::Connection;
use tokio::time::Instant;

use crate::domain_key::DomainKeys;
use crate::politeness::{self, DomainRule, Limits, SharedEntropy};

mod order;

use order::{DomainOrder, Standing, Turns};

/// How many requests one domain's queue holds at most; the rest wait in
/// the run file until it drains.
pub(crate) const BOUND: usize = 64;

/// The table the queues journal each domain's wait in.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier",
    statements: &["CREATE TABLE IF NOT EXISTS domains (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoffs INTEGER NOT NULL
        ) STRICT"],
};

/// Where a request stands in its domain's queue (ADRs 0004 and 0005), the
/// lowest first: a `forefront` one ahead of every other, the later of two
/// first, so each goes ahead of every request its domain held when it came;
/// then the highest priority; then the crawl's own order, so a retried
/// request keeps its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Place {
    forefront: Reverse<Option<i64>>,
    priority: Reverse<i64>,
    order: i64,
}

impl Place {
    /// The place of `request`.
    fn of(request: &Waiting) -> Self {
        Self {
            forefront: Reverse(request.forefront.then_some(request.order)),
            priority: Reverse(request.priority),
            order: request.order,
        }
    }
}

/// One domain's queue and state.
#[derive(Debug, Default)]
struct Domain {
    waiting: BTreeSet<(Place, String)>,
    /// Whether requests of it may wait outside the queue, in the run file,
    /// since it filled: then a request that does not go ahead of its last
    /// one waits outside too, until the queue drains and fills again in
    /// order.
    outside: bool,
    in_flight: usize,
    ready_at: Option<Instant>,
    backoffs: u32,
    /// Until when the shared Politeness holds it, unless a wake comes
    /// first, and why; never journaled, since the Politeness file keeps
    /// its cause.
    held_until: Option<Instant>,
    held: Option<Held>,
    /// Whether the run file holds a row of it.
    journaled: bool,
    /// The Politeness rule's state of it, while the Run shares none: its
    /// backoff, the moment it waits until and the ceiling a 429 left it,
    /// its moments in milliseconds of wall time.
    limits: Limits,
    /// The wait it was last counted under, by its place in
    /// [`DomainQueues::waits`], while it has waiting requests, and until
    /// when that wait lasts, if it ends by itself.
    counted: Option<(usize, Option<Instant>)>,
}

/// Why a domain's waiting requests wait, as a Run's progress groups them:
/// ready, then held for each reason, by its place in
/// [`DomainQueues::waits`].
const WAITS: [Option<Held>; 5] = [
    None,
    Some(Held::Concurrency),
    Some(Held::Delay),
    Some(Held::BackOff),
    Some(Held::PolitenessFile),
];

/// The place of `held` in [`WAITS`].
fn place_of(held: Option<Held>) -> usize {
    match held {
        None => 0,
        Some(Held::Concurrency) => 1,
        Some(Held::Delay) => 2,
        Some(Held::BackOff) => 3,
        Some(Held::PolitenessFile) => 4,
    }
}

/// The domains counted under one wait, and their waiting requests.
#[derive(Debug, Default)]
struct Waits {
    requests: usize,
    domains: BTreeSet<String>,
}

impl Domain {
    fn is_ready(&self, now: Instant) -> bool {
        self.held_until.is_none_or(|held_until| held_until <= now)
            && self.ready_at.is_none_or(|ready_at| ready_at <= now)
            && !self.is_full()
    }

    /// Whether as many of its requests are in flight as the ceiling a 429
    /// left it admits.
    fn is_full(&self) -> bool {
        self.limits
            .ceiling
            .is_some_and(|ceiling| u64::try_from(self.in_flight).unwrap_or(u64::MAX) >= ceiling)
    }

    /// When it is ready, whatever the moment: once its hold and its wait
    /// are over, and only once one of its requests ends while it is full.
    fn standing(&self) -> Standing {
        if self.is_full() {
            return Standing::Full;
        }
        match [self.ready_at, self.held_until].into_iter().flatten().max() {
            Some(from) => Standing::From(from),
            None => Standing::Ready,
        }
    }

    /// Whether it holds nothing more at `now`: no request waiting or in
    /// flight, no wait, and no backoff or ceiling it keeps itself.
    fn is_drained(&self, now: Instant) -> bool {
        self.waiting.is_empty()
            && self.in_flight == 0
            && self.is_ready(now)
            && self.limits.is_clear()
    }

    /// Why its requests wait at `now`, and until when, if that wait ends by
    /// itself: its hold while it lasts, for want of room when no reason
    /// was given; its backoff while it lasts; for want of room while it is
    /// full; else nothing.
    fn waits_at(&self, now: Instant) -> (Option<Held>, Option<Instant>) {
        match (self.held_until, self.ready_at) {
            (Some(until), _) if until > now => {
                (Some(self.held.unwrap_or(Held::Concurrency)), Some(until))
            }
            (_, Some(ready_at)) if ready_at > now => (Some(Held::BackOff), Some(ready_at)),
            _ if self.is_full() => (Some(Held::Concurrency), None),
            _ => (None, None),
        }
    }
}

/// A crawl's per-domain queues.
///
/// They keep what a Run's progress shows of their waits as each change
/// comes, so reading it costs the changes since the last read and not a
/// walk of every domain (#1547): each domain with waiting requests is
/// counted under the wait it was last found in, a domain whose hold, wake,
/// end or queue changed is found anew at the next read, and so is one
/// whose wait ended by itself, which `expiring` orders by its end.
#[derive(Debug)]
pub struct DomainQueues {
    keys: DomainKeys,
    bound: usize,
    domains: BTreeMap<String, Domain>,
    /// The turns of the domains with a request waiting.
    turns: Turns,
    idle: BTreeSet<String>,
    ids: BTreeSet<String>,
    changed: BTreeSet<String>,
    entropy: SharedEntropy,
    /// What the Run waits for under its settings, which the queues back
    /// each domain off by while it shares no Politeness.
    waiting_for: politeness::Waits,
    /// A wall time and the monotonic reading taken with it, which the
    /// queues read each moment's wall time from: the last their Run gave.
    anchor: Option<(SystemTime, Instant)>,
    /// The requests waiting in every domain's queue.
    waiting: usize,
    /// The domains counted under each wait of [`WAITS`].
    waits: [Waits; 5],
    /// The domains to count anew at the next read.
    recount: BTreeSet<String>,
    /// The counted domains whose wait ends by itself, by when it ends.
    expiring: BTreeSet<(Instant, String)>,
}

impl DomainQueues {
    /// Empty queues keyed by `keys`, each holding at most `bound`, whose
    /// Politeness rule draws its backoffs from `entropy`, waiting as the
    /// catalog's defaults say until they take a Run's settings.
    pub fn new(keys: DomainKeys, bound: usize, entropy: Entropy) -> Self {
        Self {
            keys,
            bound,
            domains: BTreeMap::new(),
            turns: Turns::new(DomainOrder::default()),
            idle: BTreeSet::new(),
            ids: BTreeSet::new(),
            changed: BTreeSet::new(),
            entropy: SharedEntropy::new(entropy.into()),
            waiting_for: politeness::Waits::default(),
            anchor: None,
            waiting: 0,
            waits: Default::default(),
            recount: BTreeSet::new(),
            expiring: BTreeSet::new(),
        }
    }

    /// The wall time of the monotonic reading `at`, from the queues'
    /// anchor, which the first reading sets when their Run gave none.
    fn wall(&mut self, at: Instant) -> SystemTime {
        let (wall, read) = *self.anchor.get_or_insert((UNIX_EPOCH, at));
        if at >= read {
            wall + (at - read)
        } else {
            wall.checked_sub(read - at).unwrap_or(wall)
        }
    }

    /// `key` keeps its turn, as its state now stands.
    fn stand(&mut self, key: &str) {
        if let Some(domain) = self.domains.get(key) {
            self.turns.stand(key, domain.in_flight, domain.standing());
        }
    }

    /// How many domains it keeps state for; a released one counts no more.
    pub fn domains(&self) -> usize {
        self.domains.len()
    }

    /// The queue of `key` went from `before` waiting requests to `after`:
    /// the wait it is counted under counts them, a domain left with none is
    /// counted no more, and one that had none is counted at the next read.
    fn queue_changed(&mut self, key: &str, before: usize, after: usize) {
        self.waiting = self.waiting + after - before;
        if before == 0 && after > 0 {
            self.recount.insert(key.to_owned());
        }
        let Some(domain) = self.domains.get_mut(key) else {
            return;
        };
        let Some((place, ends)) = domain.counted else {
            return;
        };
        let waits = &mut self.waits[place];
        waits.requests = waits.requests + after - before;
        if after == 0 {
            waits.domains.remove(key);
            domain.counted = None;
            if let Some(ends) = ends {
                self.expiring.remove(&(ends, key.to_owned()));
            }
        }
    }

    /// Counts `key` under the wait it is in at `now`, in place of the one
    /// it was counted under; none when its queue holds nothing.
    fn recount(&mut self, key: &str, now: Instant) {
        let Some(domain) = self.domains.get_mut(key) else {
            return;
        };
        let requests = domain.waiting.len();
        if let Some((place, ends)) = domain.counted.take() {
            let waits = &mut self.waits[place];
            waits.requests -= requests;
            waits.domains.remove(key);
            if let Some(ends) = ends {
                self.expiring.remove(&(ends, key.to_owned()));
            }
        }
        if requests == 0 {
            return;
        }
        let (held, ends) = domain.waits_at(now);
        let place = place_of(held);
        domain.counted = Some((place, ends));
        let waits = &mut self.waits[place];
        waits.requests += requests;
        waits.domains.insert(key.to_owned());
        if let Some(ends) = ends {
            self.expiring.insert((ends, key.to_owned()));
        }
    }

    /// Releases each idle domain that holds nothing more at `now`: only a
    /// domain with no request waiting or in flight is idle.
    fn release_drained(&mut self, now: Instant) {
        let drained: Vec<String> = self
            .idle
            .iter()
            .filter(|key| {
                self.domains
                    .get(*key)
                    .is_none_or(|domain| domain.is_drained(now))
            })
            .cloned()
            .collect();
        for key in drained {
            self.idle.remove(&key);
            if self
                .domains
                .remove(&key)
                .is_some_and(|domain| domain.journaled)
            {
                self.changed.insert(key);
            }
        }
    }
}

impl Gate for DomainQueues {
    fn key(&self, url: &str) -> String {
        self.keys.key(url)
    }

    fn keys(&self) -> Keys {
        let keys = self.keys.clone();
        Keys::new(move |url| keys.key(url))
    }

    fn bound(&self) -> usize {
        self.bound
    }

    fn enqueue(&mut self, request: Waiting) -> bool {
        if self.ids.contains(&request.id) {
            return false;
        }
        let place = (Place::of(&request), request.id);
        let domain = self.domains.entry(request.key.clone()).or_default();
        if domain.outside && domain.waiting.last().is_none_or(|last| place > *last) {
            return false;
        }
        let before = domain.waiting.len();
        if before == 0 {
            self.idle.remove(&request.key);
            self.turns
                .join(&request.key, domain.in_flight, domain.standing());
        }
        self.ids.insert(place.1.clone());
        domain.waiting.insert(place);
        if domain.waiting.len() >= self.bound {
            domain.outside = true;
        }
        if domain.waiting.len() > self.bound
            && let Some((_, put_out)) = domain.waiting.pop_last()
        {
            self.ids.remove(&put_out);
        }
        let after = domain.waiting.len();
        self.queue_changed(&request.key, before, after);
        true
    }

    fn withdraw(&mut self, key: &str, id: &str) {
        if !self.ids.remove(id) {
            return;
        }
        let Some(domain) = self.domains.get_mut(key) else {
            return;
        };
        let before = domain.waiting.len();
        domain.waiting.retain(|(_, waiting)| waiting != id);
        let after = domain.waiting.len();
        if after == 0 {
            domain.outside = false;
            self.turns.leave(key);
            if domain.in_flight == 0 {
                self.idle.insert(key.to_owned());
            }
        }
        self.queue_changed(key, before, after);
    }

    fn ready_at(&self, now: Instant) -> Option<Instant> {
        self.turns.ready_at(now)
    }

    fn next_key(&self, now: Instant) -> Option<String> {
        self.turns.next(now).map(str::to_owned)
    }

    fn hold(&mut self, key: &str, until: Instant, held: Held) {
        if let Some(domain) = self.domains.get_mut(key) {
            domain.held_until = Some(until);
            domain.held = Some(held);
            self.recount.insert(key.to_owned());
            self.stand(key);
        }
    }

    fn wake(&mut self, key: &str) {
        if let Some(domain) = self.domains.get_mut(key) {
            domain.held_until = None;
            domain.held = None;
            self.recount.insert(key.to_owned());
            self.stand(key);
        }
    }

    fn queued(&mut self, now: Instant, sample: usize) -> Vec<QueuedGroup> {
        while let Some((ends, _)) = self.expiring.first()
            && *ends <= now
        {
            if let Some((_, key)) = self.expiring.pop_first() {
                self.recount.insert(key);
            }
        }
        for key in std::mem::take(&mut self.recount) {
            self.recount(&key, now);
        }
        WAITS
            .iter()
            .zip(&self.waits)
            .filter(|(_, waits)| !waits.domains.is_empty())
            .map(|(held, waits)| QueuedGroup {
                held: *held,
                requests: waits.requests,
                keys: waits.domains.len(),
                sample: waits
                    .domains
                    .iter()
                    .take(sample)
                    .map(|key| {
                        let ends = self
                            .domains
                            .get(key)
                            .and_then(|domain| domain.counted)
                            .and_then(|(_, ends)| ends);
                        // A domain held for want of room waits for a wake,
                        // not for a known moment.
                        let shown = ends.filter(|_| *held != Some(Held::Concurrency));
                        (key.clone(), shown)
                    })
                    .collect(),
            })
            .collect()
    }

    fn waiting(&self) -> usize {
        self.waiting
    }

    fn rule(&self, settings: &Resolved) -> Option<SharedRule> {
        Some(SharedRule::new(DomainRule::of(
            settings,
            Arc::clone(&self.entropy),
        )))
    }

    fn admit(&mut self, now: Instant) -> Option<Admitted> {
        let key = self.turns.take(now)?;
        self.release_drained(now);
        let domain = self.domains.get_mut(&key)?;
        let before = domain.waiting.len();
        let (_, id) = domain.waiting.pop_first()?;
        domain.in_flight += 1;
        let drained = domain.waiting.is_empty();
        if drained {
            domain.outside = false;
        } else {
            self.turns.join(&key, domain.in_flight, domain.standing());
        }
        self.queue_changed(&key, before, before - 1);
        self.ids.remove(&id);
        Some(Admitted {
            id,
            key,
            drained,
            refused_until: None,
        })
    }

    fn ended(&mut self, key: &str, now: Instant, ended: Ended) {
        let wall = self.wall(now);
        // The moment a shared Politeness named comes as `Until`, its rule
        // already applied; any other ending has the queues apply that rule
        // themselves, over the state they keep of the domain, as a Run
        // sharing no Politeness passes them every ending (ADR 0138).
        let ending = match ended {
            Ended::Done => Some(Ending::Done),
            Ended::BackOff(until) => Some(Ending::BackOff(until.map(|until| self.wall(until)))),
            Ended::Until(_) => None,
            Ended::TooManyRequests(until) => {
                Some(Ending::TooManyRequests(until.map(|until| self.wall(until))))
            }
        };
        let domain = self.domains.entry(key.to_owned()).or_default();
        domain.in_flight = domain.in_flight.saturating_sub(1);
        if domain.waiting.is_empty() && domain.in_flight == 0 {
            self.idle.insert(key.to_owned());
        }
        let backed_off = domain.backoffs > 0;
        let wall_now = milliseconds(wall);
        let others = u64::try_from(domain.in_flight).unwrap_or(u64::MAX);
        let wait_until = match ending {
            None => match ended {
                Ended::Until(until) => Some(until),
                _ => None,
            },
            Some(ending) => {
                // A request that ended well changes nothing of a domain
                // whose state is clear, so most endings draw no lock.
                if !(matches!(ending, Ending::Done) && domain.limits.is_clear()) {
                    let mut entropy = self
                        .entropy
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    domain.limits = domain.limits.ended(
                        others,
                        ending,
                        wall_now,
                        &self.waiting_for,
                        &mut entropy,
                    );
                }
                domain
                    .limits
                    .ready_at
                    .and_then(|ready_at| u64::try_from(ready_at - wall_now).ok())
                    .filter(|wait| *wait > 0)
                    .map(|wait| now + Duration::from_millis(wait))
            }
        };
        domain.backoffs = match ended {
            Ended::Done => 0,
            Ended::BackOff(_) | Ended::TooManyRequests(_)
                if self.waiting_for.backoff().is_some() =>
            {
                domain.backoffs.saturating_add(1)
            }
            _ => domain.backoffs,
        };
        if let Some(until) = wait_until {
            domain.ready_at = Some(
                domain
                    .ready_at
                    .map_or(until, |ready_at| ready_at.max(until)),
            );
        } else if domain.is_ready(now) {
            domain.ready_at = None;
        }
        if wait_until.is_some() || backed_off || domain.journaled {
            self.changed.insert(key.to_owned());
        }
        self.recount.insert(key.to_owned());
        self.stand(key);
        self.release_drained(now);
    }

    fn journal(&mut self, wall: SystemTime, now: Instant) -> Option<Journal> {
        self.anchor = Some((wall, now));
        if self.changed.is_empty() {
            return None;
        }
        let rows: Vec<JournalRow> = std::mem::take(&mut self.changed)
            .into_iter()
            .map(|key| {
                let row = self.domains.get_mut(&key).and_then(|domain| {
                    let ready_at = domain
                        .ready_at
                        .filter(|ready_at| *ready_at > now)
                        .map(|ready_at| milliseconds(wall + (ready_at - now)));
                    domain.journaled = ready_at.is_some() || domain.backoffs > 0;
                    domain.journaled.then_some((ready_at, domain.backoffs))
                });
                (key, row)
            })
            .collect();
        Some(SealedBatch::of(JournalRows(rows)))
    }
}

/// A domain's key and its journal row: its wait's end in milliseconds of
/// wall time and its backoffs in a row, or none once released.
type JournalRow = (String, Option<(Option<i64>, u32)>);

/// The journal rows of the domains that changed, as memory holds them: a
/// Run-state batch's rows (ADR 0142).
struct JournalRows(Vec<JournalRow>);

impl BatchRows for JournalRows {
    fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()> {
        let mut held = writer.statement(
            "INSERT INTO domains (key, ready_at, backoffs) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET ready_at = ?2, backoffs = ?3",
        )?;
        let mut released = writer.statement("DELETE FROM domains WHERE key = ?1")?;
        for (key, row) in self.0 {
            match row {
                Some((ready_at, backoffs)) => held.write((&key, ready_at, backoffs))?,
                None => released.write([&key])?,
            }
        }
        Ok(())
    }
}

/// The gates Foxhound's composition passes in: each crawl's per-domain
/// queues, keyed under its `crawl.domain_keys` and served in its
/// `crawl.domain_order`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DomainGates;

impl Gates for DomainGates {
    fn keys(&self, settings: &Resolved) -> Keys {
        let keys = DomainKeys::of(settings);
        Keys::new(move |url| keys.key(url))
    }

    fn open(
        &self,
        settings: &Resolved,
        journal: &Connection,
        wall: SystemTime,
        now: Instant,
        entropy: Entropy,
    ) -> rusqlite::Result<Box<dyn Gate>> {
        let mut queues = DomainQueues::new(DomainKeys::of(settings), BOUND, entropy);
        queues.waiting_for = politeness::Waits::of(settings);
        queues.turns = Turns::new(DomainOrder::of(settings));
        queues.anchor = Some((wall, now));
        let wall = milliseconds(wall);
        let mut rows = journal.prepare_cached("SELECT key, ready_at, backoffs FROM domains")?;
        let rows = rows.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })?;
        for row in rows {
            let (key, ready_at, backoffs) = row?;
            let wait = ready_at
                .and_then(|ready_at| u64::try_from(ready_at.saturating_sub(wall)).ok())
                .filter(|wait| *wait > 0)
                .map(|wait| now + Duration::from_millis(wait));
            queues.idle.insert(key.clone());
            queues.domains.insert(
                key,
                Domain {
                    ready_at: wait,
                    backoffs,
                    journaled: true,
                    ..Domain::default()
                },
            );
        }
        Ok(Box::new(queues))
    }
}

#[cfg(test)]
mod tests;
