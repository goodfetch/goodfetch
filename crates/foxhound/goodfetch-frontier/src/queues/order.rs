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

use std::collections::BTreeMap;

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

/// A key's turn, which its domain holds while it takes one (#3488).
#[derive(Clone, Copy, Debug)]
pub(super) struct Turn {
    /// Its number: a later turn has a higher one.
    number: u64,
    rank: usize,
    /// Where it is indexed: among the ready keys, the keys asleep until a
    /// moment, or neither while it is full. A turn asleep may since be
    /// among the ready keys, its wait over; one in neither index, though
    /// ready or asleep, was taken by a free slot.
    standing: Standing,
}

/// The turns of the keys with a request waiting, indexed by when each is
/// ready and, among the ready ones, by rank and turn.
///
/// Its readings never go back, as a monotonic clock's: a choice at one
/// moment takes every key whose wait ended by then among the ready ones.
///
/// Each key's [`Turn`] is held by its domain, which the caller hands in,
/// so no step looks a key up by its name; the indexes order by numbers
/// alone, a turn's number being its own, and hold the key's name, which
/// moves between them as its standing changes (#3488).
#[derive(Debug)]
pub(super) struct Turns {
    order: DomainOrder,
    /// The next turn's number; a later turn has a higher one.
    next: u64,
    ready: BTreeMap<(usize, u64), Box<str>>,
    /// The keys asleep, each with its rank.
    asleep: BTreeMap<(Instant, u64), (usize, Box<str>)>,
}

impl Turns {
    /// No turns yet, under `order`.
    pub(super) fn new(order: DomainOrder) -> Self {
        Self {
            order,
            next: 0,
            ready: BTreeMap::new(),
            asleep: BTreeMap::new(),
        }
    }

    /// `key`, whose turn `turn` holds, takes a turn at the back, with
    /// `in_flight` requests in flight and ready as `standing` says.
    pub(super) fn join(
        &mut self,
        turn: &mut Option<Turn>,
        key: &str,
        in_flight: usize,
        standing: Standing,
    ) {
        let name = turn.take().and_then(|before| self.unindex(before));
        let joined = Turn {
            number: self.next,
            rank: self.order.rank(in_flight),
            standing,
        };
        self.next += 1;
        self.index(joined, name.unwrap_or_else(|| Box::from(key)));
        *turn = Some(joined);
    }

    /// The key whose turn `turn` holds takes no more turns.
    pub(super) fn leave(&mut self, turn: &mut Option<Turn>) {
        if let Some(before) = turn.take() {
            self.unindex(before);
        }
    }

    /// `key`, whose turn `turn` holds, keeps its turn, now with
    /// `in_flight` requests in flight and ready as `standing` says;
    /// nothing when it takes no turn or a free slot took it.
    pub(super) fn stand(
        &mut self,
        turn: &mut Option<Turn>,
        key: &str,
        in_flight: usize,
        standing: Standing,
    ) {
        let Some(before) = *turn else {
            return;
        };
        let stood = Turn {
            rank: self.order.rank(in_flight),
            standing,
            ..before
        };
        if (stood.rank, stood.standing) == (before.rank, before.standing) {
            return;
        }
        let name = match before.standing {
            Standing::Full => Box::from(key),
            Standing::Ready | Standing::From(_) => match self.unindex(before) {
                Some(name) => name,
                None => return,
            },
        };
        self.index(stood, name);
        *turn = Some(stood);
    }

    /// The key a free slot goes to at `now`, if one is ready.
    pub(super) fn next(&self, now: Instant) -> Option<&str> {
        let ready = self
            .ready
            .first_key_value()
            .map(|((rank, turn), key)| (*rank, *turn, &**key));
        let woken = self
            .asleep
            .iter()
            .take_while(|((from, _), _)| *from <= now)
            .map(|((_, turn), (rank, key))| (*rank, *turn, &**key))
            .min();
        ready.into_iter().chain(woken).min().map(|(_, _, key)| key)
    }

    /// Takes the key a free slot goes to at `now` out of the turns, if one
    /// is ready. The caller then has it take a turn again at the back,
    /// if it still has a request waiting, or leave.
    pub(super) fn take(&mut self, now: Instant) -> Option<String> {
        while let Some(entry) = self.asleep.first_entry()
            && entry.key().0 <= now
        {
            let ((_, turn), (rank, key)) = entry.remove_entry();
            self.ready.insert((rank, turn), key);
        }
        self.ready.pop_first().map(|(_, key)| key.into_string())
    }

    /// The first moment from `now` a key is ready; `None` while every key
    /// is full or none takes a turn.
    pub(super) fn ready_at(&self, now: Instant) -> Option<Instant> {
        if self.ready.is_empty() {
            self.asleep
                .first_key_value()
                .map(|((from, _), _)| (*from).max(now))
        } else {
            Some(now)
        }
    }

    /// Indexes `turn` under its standing, holding `key`'s name there.
    fn index(&mut self, turn: Turn, key: Box<str>) {
        match turn.standing {
            Standing::Ready => {
                self.ready.insert((turn.rank, turn.number), key);
            }
            Standing::From(from) => {
                self.asleep.insert((from, turn.number), (turn.rank, key));
            }
            Standing::Full => {}
        }
    }

    /// Takes `turn` out of its index, answering the name it held there;
    /// `None` when it is full or a free slot took it.
    fn unindex(&mut self, turn: Turn) -> Option<Box<str>> {
        match turn.standing {
            Standing::Ready => self.ready.remove(&(turn.rank, turn.number)),
            Standing::From(from) => self
                .asleep
                .remove(&(from, turn.number))
                .map(|(_, key)| key)
                .or_else(|| self.ready.remove(&(turn.rank, turn.number))),
            Standing::Full => None,
        }
    }
}
