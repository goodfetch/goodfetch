//! The order a crawl's free request slot goes to its domain keys in (ADR
//! 0004 and its note of spec #2194), under `crawl.domain_order`: to the
//! ready key with the fewest of the Run's requests in flight, ties going
//! to the key whose turn came first, or, at `round_robin`, to each ready
//! key in turn. A key takes a turn at the back when its first request
//! comes and again after each request it is admitted, keeps it while it
//! waits, and leaves the turns once it holds no request waiting.
//!
//! The keys are indexed by when they are ready, so a choice costs a
//! logarithm of the keys with a request waiting, plus the keys whose wait
//! ended since the last admission, and never a walk of every key (#2197).

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::crawl::CRAWL_DOMAIN_ORDER;
use tokio::time::Instant;

/// Which ready key a free request slot goes to (`crawl.domain_order`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DomainOrder {
    /// The one with the fewest of the Run's requests in flight, the first
    /// in turn among those with as few.
    #[default]
    FewestInFlight,
    /// Each in turn, one request each.
    RoundRobin,
}

impl DomainOrder {
    /// The order `settings` name.
    pub(crate) fn of(settings: &Resolved) -> Self {
        match settings.get(&CRAWL_DOMAIN_ORDER) {
            "round_robin" => Self::RoundRobin,
            _ => Self::FewestInFlight,
        }
    }

    /// What a key with `in_flight` requests in flight ranks by, the lowest
    /// first, before its turn.
    fn rank(self, in_flight: usize) -> usize {
        match self {
            Self::FewestInFlight => in_flight,
            Self::RoundRobin => 0,
        }
    }
}

/// When a domain key is ready, whatever the moment: now, from a moment on,
/// or only once one of its requests in flight ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    Ready,
    From(Instant),
    Full,
}

/// A key's place in the turns.
#[derive(Clone, Copy, Debug)]
struct Turn {
    /// Its number: a later turn has a higher one.
    number: u64,
    rank: usize,
    /// Where it is indexed: among the ready keys, the keys asleep until a
    /// moment, or neither while it is full.
    standing: Standing,
}

/// The turns of the keys with a request waiting, indexed by when each is
/// ready and, among the ready ones, by rank and turn.
///
/// Its readings never go back, as a monotonic clock's: a choice at one
/// moment takes every key whose wait ended by then among the ready ones.
#[derive(Debug)]
pub(super) struct Turns {
    order: DomainOrder,
    /// The next turn's number; a later turn has a higher one.
    next: u64,
    keys: BTreeMap<String, Turn>,
    ready: BTreeSet<(usize, u64, String)>,
    asleep: BTreeSet<(Instant, u64, String)>,
}

impl Turns {
    /// No turns yet, under `order`.
    pub(super) fn new(order: DomainOrder) -> Self {
        Self {
            order,
            next: 0,
            keys: BTreeMap::new(),
            ready: BTreeSet::new(),
            asleep: BTreeSet::new(),
        }
    }

    /// `key` takes a turn at the back, with `in_flight` requests in flight
    /// and ready as `standing` says.
    pub(super) fn join(&mut self, key: &str, in_flight: usize, standing: Standing) {
        let turn = Turn {
            number: self.next,
            rank: self.order.rank(in_flight),
            standing,
        };
        self.next += 1;
        self.leave(key);
        self.index(key, turn);
        self.keys.insert(key.to_owned(), turn);
    }

    /// `key` takes no more turns.
    pub(super) fn leave(&mut self, key: &str) {
        if let Some(turn) = self.keys.remove(key) {
            self.unindex(key, turn);
        }
    }

    /// `key` keeps its turn, now with `in_flight` requests in flight and
    /// ready as `standing` says; nothing when it takes no turn.
    pub(super) fn stand(&mut self, key: &str, in_flight: usize, standing: Standing) {
        let Some(turn) = self.keys.get(key).copied() else {
            return;
        };
        self.unindex(key, turn);
        let turn = Turn {
            rank: self.order.rank(in_flight),
            standing,
            ..turn
        };
        self.index(key, turn);
        self.keys.insert(key.to_owned(), turn);
    }

    /// The key a free slot goes to at `now`, if one is ready.
    pub(super) fn next(&self, now: Instant) -> Option<&str> {
        let ready = self
            .ready
            .first()
            .map(|(rank, turn, key)| (*rank, *turn, key.as_str()));
        let woken = self
            .asleep
            .iter()
            .take_while(|(from, _, _)| *from <= now)
            .filter_map(|(_, turn, key)| {
                let rank = self.keys.get(key)?.rank;
                Some((rank, *turn, key.as_str()))
            })
            .min();
        ready.into_iter().chain(woken).min().map(|(_, _, key)| key)
    }

    /// Takes the key a free slot goes to at `now` out of the turns, if one
    /// is ready: it takes a turn again at the back if it still has a
    /// request waiting.
    pub(super) fn take(&mut self, now: Instant) -> Option<String> {
        while let Some((from, _, _)) = self.asleep.first()
            && *from <= now
        {
            let Some((_, turn, key)) = self.asleep.pop_first() else {
                break;
            };
            if let Some(kept) = self.keys.get_mut(&key) {
                kept.standing = Standing::Ready;
                self.ready.insert((kept.rank, turn, key));
            }
        }
        let (_, _, key) = self.ready.pop_first()?;
        self.keys.remove(&key);
        Some(key)
    }

    /// The first moment from `now` a key is ready; `None` while every key
    /// is full or none takes a turn.
    pub(super) fn ready_at(&self, now: Instant) -> Option<Instant> {
        if self.ready.is_empty() {
            self.asleep.first().map(|(from, _, _)| (*from).max(now))
        } else {
            Some(now)
        }
    }

    fn index(&mut self, key: &str, turn: Turn) {
        match turn.standing {
            Standing::Ready => {
                self.ready.insert((turn.rank, turn.number, key.to_owned()));
            }
            Standing::From(from) => {
                self.asleep.insert((from, turn.number, key.to_owned()));
            }
            Standing::Full => {}
        }
    }

    fn unindex(&mut self, key: &str, turn: Turn) {
        match turn.standing {
            Standing::Ready => {
                self.ready.remove(&(turn.rank, turn.number, key.to_owned()));
            }
            Standing::From(from) => {
                self.asleep.remove(&(from, turn.number, key.to_owned()));
            }
            Standing::Full => {}
        }
    }
}
