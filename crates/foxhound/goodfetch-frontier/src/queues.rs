//! A crawl's per-domain queues (ADRs 0004, 0005, 0021 and 0087), the gate
//! in admission Foxhound fills: each domain key's waiting requests in a
//! queue bounded at [`BOUND`], a `forefront` request ahead of every request
//! its domain held when it came, then highest priority first and then in
//! their order in the crawl, served round-robin, one request per domain per
//! turn, so a heavy domain never starves the others and neither a priority
//! nor `forefront` crosses domains. A full queue keeps the requests that go
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
//! backoff is the `politeness` module's.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::admission::{Admitted, Ended, Gate, Gates, Held, Journal, Queued, Waiting};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::politeness::PermitRule;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::settings::Resolved;
use rusqlite::Connection;
use tokio::time::Instant;

use crate::domain_key::DomainKeys;
use crate::politeness::{DomainRule, SharedEntropy};

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
}

impl Domain {
    fn is_ready(&self, now: Instant) -> bool {
        self.ready_at.is_none_or(|ready_at| ready_at <= now)
            && self.held_until.is_none_or(|held_until| held_until <= now)
    }

    /// The first moment from `now` it is ready.
    fn ready_from(&self, now: Instant) -> Instant {
        [self.ready_at, self.held_until]
            .into_iter()
            .flatten()
            .fold(now, Instant::max)
    }

    /// Whether it holds nothing more at `now`: no request waiting or in
    /// flight, and no wait.
    fn is_drained(&self, now: Instant) -> bool {
        self.waiting.is_empty() && self.in_flight == 0 && self.is_ready(now)
    }
}

/// A crawl's per-domain queues.
#[derive(Debug)]
pub struct DomainQueues {
    keys: DomainKeys,
    bound: usize,
    domains: BTreeMap<String, Domain>,
    turns: VecDeque<String>,
    idle: BTreeSet<String>,
    ids: BTreeSet<String>,
    changed: BTreeSet<String>,
    entropy: SharedEntropy,
}

impl DomainQueues {
    /// Empty queues keyed by `keys`, each holding at most `bound`, whose
    /// Politeness rule draws its backoffs from `entropy`.
    pub fn new(keys: DomainKeys, bound: usize, entropy: Entropy) -> Self {
        Self {
            keys,
            bound,
            domains: BTreeMap::new(),
            turns: VecDeque::new(),
            idle: BTreeSet::new(),
            ids: BTreeSet::new(),
            changed: BTreeSet::new(),
            entropy: SharedEntropy::new(entropy.into()),
        }
    }

    /// How many domains it keeps state for; a released one counts no more.
    pub fn domains(&self) -> usize {
        self.domains.len()
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
        if domain.waiting.is_empty() {
            self.idle.remove(&request.key);
            self.turns.push_back(request.key);
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
        true
    }

    fn withdraw(&mut self, key: &str, id: &str) {
        if !self.ids.remove(id) {
            return;
        }
        let Some(domain) = self.domains.get_mut(key) else {
            return;
        };
        domain.waiting.retain(|(_, waiting)| waiting != id);
        if domain.waiting.is_empty() {
            domain.outside = false;
            self.turns.retain(|turn| turn != key);
            if domain.in_flight == 0 {
                self.idle.insert(key.to_owned());
            }
        }
    }

    fn ready_at(&self, now: Instant) -> Option<Instant> {
        self.turns
            .iter()
            .filter_map(|key| self.domains.get(key))
            .map(|domain| domain.ready_from(now))
            .min()
    }

    fn next_key(&self, now: Instant) -> Option<String> {
        self.turns
            .iter()
            .find(|key| {
                self.domains
                    .get(*key)
                    .is_some_and(|domain| domain.is_ready(now))
            })
            .cloned()
    }

    fn hold(&mut self, key: &str, until: Instant, held: Held) {
        if let Some(domain) = self.domains.get_mut(key) {
            domain.held_until = Some(until);
            domain.held = Some(held);
        }
    }

    fn wake(&mut self, key: &str) {
        if let Some(domain) = self.domains.get_mut(key) {
            domain.held_until = None;
            domain.held = None;
        }
    }

    fn queued(&self, now: Instant) -> Vec<Queued> {
        self.turns
            .iter()
            .filter_map(|key| {
                let domain = self.domains.get(key)?;
                let held = match (domain.held_until, domain.ready_at) {
                    (Some(until), _) if until > now => {
                        let held = domain.held.unwrap_or(Held::Concurrency);
                        // A domain held for want of room waits for a wake,
                        // not for a known moment.
                        Some((held, (held != Held::Concurrency).then_some(until)))
                    }
                    (_, Some(ready_at)) if ready_at > now => Some((Held::BackOff, Some(ready_at))),
                    _ => None,
                };
                Some(Queued {
                    key: key.clone(),
                    requests: domain.waiting.len(),
                    held,
                })
            })
            .collect()
    }

    fn waiting(&self) -> usize {
        self.domains
            .values()
            .map(|domain| domain.waiting.len())
            .sum()
    }

    fn rule(&self, settings: &Resolved) -> Option<Arc<dyn PermitRule>> {
        Some(Arc::new(DomainRule::of(
            settings,
            Arc::clone(&self.entropy),
        )))
    }

    fn admit(&mut self, now: Instant) -> Option<Admitted> {
        let turn = self.turns.iter().position(|key| {
            self.domains
                .get(key)
                .is_some_and(|domain| domain.is_ready(now))
        })?;
        self.release_drained(now);
        let key = self.turns.remove(turn)?;
        let domain = self.domains.get_mut(&key)?;
        let (_, id) = domain.waiting.pop_first()?;
        domain.in_flight += 1;
        let drained = domain.waiting.is_empty();
        if drained {
            domain.outside = false;
        } else {
            self.turns.push_back(key.clone());
        }
        self.ids.remove(&id);
        Some(Admitted {
            id,
            key,
            drained,
            refused_until: None,
        })
    }

    fn ended(&mut self, key: &str, now: Instant, ended: Ended) {
        let domain = self.domains.entry(key.to_owned()).or_default();
        domain.in_flight = domain.in_flight.saturating_sub(1);
        if domain.waiting.is_empty() && domain.in_flight == 0 {
            self.idle.insert(key.to_owned());
        }
        let backed_off = domain.backoffs > 0;
        let wait_until = match ended {
            Ended::Done => {
                domain.backoffs = 0;
                None
            }
            Ended::BackOff(until) => {
                domain.backoffs = domain.backoffs.saturating_add(1);
                let backoff = now + backoff(domain.backoffs);
                Some(until.map_or(backoff, |until| until.max(backoff)))
            }
            Ended::Until(until) => Some(until),
            Ended::TooManyRequests(until) => {
                domain.backoffs = domain.backoffs.saturating_add(1);
                Some(until.unwrap_or(now + backoff(domain.backoffs)))
            }
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
        self.release_drained(now);
    }

    fn journal(&mut self, wall: SystemTime, now: Instant) -> Option<Journal> {
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
        Some(Box::new(move |connection: &Connection| {
            for (key, row) in rows {
                match row {
                    Some((ready_at, backoffs)) => connection.execute(
                        "INSERT INTO domains (key, ready_at, backoffs) VALUES (?1, ?2, ?3)
                         ON CONFLICT (key) DO UPDATE SET ready_at = ?2, backoffs = ?3",
                        (&key, ready_at, backoffs),
                    )?,
                    None => connection.execute("DELETE FROM domains WHERE key = ?1", [&key])?,
                };
            }
            Ok(())
        }))
    }
}

/// The first backoff of a domain the queues back off themselves.
const BACKOFF_INITIAL: Duration = Duration::from_secs(2);

/// The longest backoff of a domain the queues back off themselves.
const BACKOFF_MAX: Duration = Duration::from_mins(1);

/// How long a domain the queues back off themselves waits after
/// `backoffs` backoffs in a row: 2 s, then doubling up to 60 s. Only a Run
/// that shares no Politeness ends requests here with a backoff; a crawl's
/// Run always shares it, where the backoff is jittered
/// ([`crate::politeness::Backoff`]).
fn backoff(backoffs: u32) -> Duration {
    let doublings = backoffs.saturating_sub(1).min(31);
    BACKOFF_INITIAL
        .saturating_mul(1 << doublings)
        .min(BACKOFF_MAX)
}

/// A domain's key and its journal row: its wait's end in milliseconds of
/// wall time and its backoffs in a row, or none once released.
type JournalRow = (String, Option<(Option<i64>, u32)>);

/// Milliseconds of `wall` since the Unix epoch.
fn milliseconds(wall: SystemTime) -> i64 {
    wall.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
    })
}

/// The gates Foxhound's composition passes in: each crawl's per-domain
/// queues, keyed under its `crawl.domain_keys`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DomainGates;

impl Gates for DomainGates {
    fn open(
        &self,
        settings: &Resolved,
        journal: &Connection,
        wall: SystemTime,
        now: Instant,
        entropy: Entropy,
    ) -> rusqlite::Result<Box<dyn Gate>> {
        let mut queues = DomainQueues::new(DomainKeys::of(settings), BOUND, entropy);
        let wall = milliseconds(wall);
        let mut rows = journal.prepare("SELECT key, ready_at, backoffs FROM domains")?;
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
