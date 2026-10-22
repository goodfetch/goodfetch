//! A crawl's per-domain queues (ADRs 0004, 0021 and 0087), the gate in
//! admission Foxhound fills: each domain key's waiting requests in a queue
//! bounded at [`BOUND`], highest priority first and then in their order in
//! the crawl, served round-robin, one request per domain per turn, so a heavy
//! domain never starves the others and a priority never crosses domains.
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

use goodfetch_engine::admission::{Admitted, Ended, Gate, Gates, Journal, Waiting};
use goodfetch_engine::politeness::PermitRule;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::crawl::CRAWL_DOMAIN_KEYS;
use rusqlite::Connection;
use tokio::time::Instant;

use crate::domain_key::DomainKeys;
use crate::politeness::{DomainRule, backoff};

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

/// One domain's queue and state.
#[derive(Debug, Default)]
struct Domain {
    waiting: BTreeSet<(Reverse<i64>, i64, String)>,
    in_flight: usize,
    ready_at: Option<Instant>,
    backoffs: u32,
    /// Until when the shared Politeness holds it, unless a wake comes
    /// first; never journaled, since the Politeness file keeps its cause.
    held_until: Option<Instant>,
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
}

impl DomainQueues {
    /// Empty queues keyed by `keys`, each holding at most `bound`.
    pub fn new(keys: DomainKeys, bound: usize) -> Self {
        Self {
            keys,
            bound,
            domains: BTreeMap::new(),
            turns: VecDeque::new(),
            idle: BTreeSet::new(),
            ids: BTreeSet::new(),
            changed: BTreeSet::new(),
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
        let Waiting {
            id,
            key,
            priority,
            order,
        } = request;
        if self.ids.contains(&id) {
            return false;
        }
        let domain = self.domains.entry(key.clone()).or_default();
        if domain.waiting.len() >= self.bound {
            return false;
        }
        if domain.waiting.is_empty() {
            self.idle.remove(&key);
            self.turns.push_back(key);
        }
        domain
            .waiting
            .insert((Reverse(priority), order, id.clone()));
        self.ids.insert(id);
        true
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

    fn hold(&mut self, key: &str, until: Instant) {
        if let Some(domain) = self.domains.get_mut(key) {
            domain.held_until = Some(until);
        }
    }

    fn wake(&mut self, key: &str) {
        if let Some(domain) = self.domains.get_mut(key) {
            domain.held_until = None;
        }
    }

    fn rule(&self, settings: &Resolved) -> Option<Arc<dyn PermitRule>> {
        Some(Arc::new(DomainRule::of(settings)))
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
        let (_, _, id) = domain.waiting.pop_first()?;
        domain.in_flight += 1;
        let drained = domain.waiting.is_empty();
        if !drained {
            self.turns.push_back(key.clone());
        }
        self.ids.remove(&id);
        Some(Admitted { id, key, drained })
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
            Ended::BackOff => {
                domain.backoffs = domain.backoffs.saturating_add(1);
                Some(now + backoff(domain.backoffs))
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
    ) -> rusqlite::Result<Box<dyn Gate>> {
        let overrides = settings
            .value(CRAWL_DOMAIN_KEYS.name)
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(host, key)| Some((host.clone(), key.as_str()?.to_owned())));
        let mut queues = DomainQueues::new(DomainKeys::new(overrides), BOUND);
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
