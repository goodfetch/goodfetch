//! A Run's admission (ADRs 0004, 0040 and 0087; RT-02): which of its
//! waiting requests goes next, and when. The gate in admission is an
//! interface a Product's composition passes in: Foxhound's per-domain
//! queues fill it, and a Run of a composition with none admits through no
//! gate. Beside it, the Run's rate cap, `admission.max_requests_per_minute`,
//! bounds every request it admits by the Generic Cell Rate Algorithm with
//! no burst, so admissions are spaced at least a minute's share apart. A
//! Run going on takes its settings again at each admission point, so a
//! tightened Operator policy cap reaches it there.
//!
//! What a gate's waiting requests need to survive a resume, it writes into
//! the run file as a [`Journal`], and it opens again from it. Every rule
//! here is a pure function of a monotonic reading the caller gives
//! (ADR 0059).

use std::fmt;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::settings::ADMISSION_MAX_REQUESTS_PER_MINUTE;
use rusqlite::Connection;
use tokio::time::Instant;

use crate::settings::Resolved;

/// A request waiting for admission, as a gate receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    /// What its Run calls it, such as a work item's ID.
    pub id: String,
    /// The key it waits under, which the gate gave its URL.
    pub key: String,
    /// Its priority among the requests of its key: higher goes first.
    pub priority: i64,
    /// Its place among the requests of its key and priority, such as its
    /// position in its Run: lower goes first.
    pub order: i64,
}

/// A request a gate admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admitted {
    /// What its Run calls it.
    pub id: String,
    /// Its key.
    pub key: String,
    /// Whether its key's queue holds nothing more, so the Run may give it
    /// the next requests of that key it keeps.
    pub drained: bool,
}

/// How an admitted request ended, for its key's readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Its key may go again at once.
    Done,
    /// Its response asked its key to back off.
    BackOff,
    /// Its response asked its key to wait until this moment, such as a
    /// `Retry-After`.
    Until(Instant),
}

/// What a gate writes into the run file, in one of its writer's commits.
pub type Journal = Box<dyn FnOnce(&Connection) -> rusqlite::Result<()> + Send>;

/// One Run's gate in admission (ADR 0087): the queues its waiting
/// requests wait in, and the order and readiness they are admitted in.
pub trait Gate: Send {
    /// The key a request to `url` waits under.
    fn key(&self, url: &str) -> String;

    /// How many requests of one key it holds at most.
    fn bound(&self) -> usize;

    /// Takes `request` into its key's queue; `false` when that queue is
    /// full or already holds it.
    fn enqueue(&mut self, request: Waiting) -> bool;

    /// When it next has a request to admit: at or before `now` when one is
    /// ready, and `None` when it holds none.
    fn ready_at(&self, now: Instant) -> Option<Instant>;

    /// The request it admits at `now`, if one is ready.
    fn admit(&mut self, now: Instant) -> Option<Admitted>;

    /// A request of `key` it admitted ended at `now`, as `ended` says.
    fn ended(&mut self, key: &str, now: Instant, ended: Ended);

    /// What changed since its last journal, to write into the run file, its
    /// moments as wall time read at `wall` when the monotonic clock read
    /// `now`.
    fn journal(&mut self, wall: SystemTime, now: Instant) -> Option<Journal>;
}

/// What a Product's composition passes in: opens a Run's gate.
pub trait Gates: Send + Sync {
    /// The gate of a Run with the settings `settings`, from what its run
    /// file `journal` holds, read at `wall` when the monotonic clock read
    /// `now`.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    fn open(
        &self,
        settings: &Resolved,
        journal: &Connection,
        wall: SystemTime,
        now: Instant,
    ) -> rusqlite::Result<Box<dyn Gate>>;
}

/// A Run's admission: its gate and its rate cap, which a request passes
/// both of to go.
pub struct Admission {
    gate: Box<dyn Gate>,
    rate: Option<RateCap>,
}

impl fmt::Debug for Admission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Admission")
            .field("rate", &self.rate)
            .finish_non_exhaustive()
    }
}

impl Admission {
    /// Admission through `gate` under the rate cap `settings` give.
    pub(crate) fn new(gate: Box<dyn Gate>, settings: &Resolved) -> Self {
        Self {
            gate,
            rate: rate_cap_of(settings).map(RateCap::per_minute),
        }
    }

    /// Takes the rate cap of `settings`, as a Run does at each admission
    /// point, keeping when it last admitted.
    pub fn update(&mut self, settings: &Resolved) {
        self.rate = match (self.rate, rate_cap_of(settings)) {
            (Some(rate), Some(per_minute)) => Some(rate.set_to(per_minute)),
            (None, Some(per_minute)) => Some(RateCap::per_minute(per_minute)),
            (_, None) => None,
        };
    }

    /// The key a request to `url` waits under.
    pub fn key(&self, url: &str) -> String {
        self.gate.key(url)
    }

    /// How many requests of one key the gate holds at most.
    pub fn bound(&self) -> usize {
        self.gate.bound()
    }

    /// Takes `request` into its key's queue; `false` when that queue is full
    /// or already holds it.
    pub fn enqueue(&mut self, request: Waiting) -> bool {
        self.gate.enqueue(request)
    }

    /// When it next admits a request: at or before `now` when one may go
    /// now, `None` when none is waiting or the rate cap admits nothing.
    pub fn ready_at(&self, now: Instant) -> Option<Instant> {
        let gate = self.gate.ready_at(now)?;
        match &self.rate {
            Some(rate) => Some(rate.ready_at(now)?.max(gate)),
            None => Some(gate),
        }
    }

    /// The request it admits at `now`, if one may go.
    pub fn admit(&mut self, now: Instant) -> Option<Admitted> {
        if self.ready_at(now)? > now {
            return None;
        }
        let admitted = self.gate.admit(now)?;
        if let Some(rate) = &mut self.rate {
            let _ = rate.admit(now);
        }
        Some(admitted)
    }

    /// A request of `key` it admitted ended at `now`, as `ended` says.
    pub fn ended(&mut self, key: &str, now: Instant, ended: Ended) {
        self.gate.ended(key, now, ended);
    }

    /// What the gate changed since its last journal, to write into the run
    /// file, read at `wall` when the monotonic clock read `now`.
    pub fn journal(&mut self, wall: SystemTime, now: Instant) -> Option<Journal> {
        self.gate.journal(wall, now)
    }
}

/// A rate cap: at most `n` admissions a minute, the Generic Cell Rate
/// Algorithm with no burst, so two admissions are at least a minute's
/// `n`th apart. A cap of 0 admits nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateCap {
    interval: Option<Duration>,
    next: Option<Instant>,
}

impl RateCap {
    /// At most `per_minute` admissions a minute.
    pub fn per_minute(per_minute: u64) -> Self {
        Self {
            interval: interval(per_minute),
            next: None,
        }
    }

    /// The same cap set to `per_minute`, keeping when it last admitted.
    #[must_use]
    pub fn set_to(self, per_minute: u64) -> Self {
        Self {
            interval: interval(per_minute),
            ..self
        }
    }

    /// The earliest moment from `now` it admits, `None` when it admits
    /// nothing.
    pub fn ready_at(&self, now: Instant) -> Option<Instant> {
        let _ = self.interval?;
        Some(self.next.map_or(now, |next| next.max(now)))
    }

    /// Admits one request at `now`, or answers when it may.
    ///
    /// # Errors
    ///
    /// The earliest moment it admits, `None` when it admits nothing.
    pub fn admit(&mut self, now: Instant) -> Result<(), Option<Instant>> {
        let interval = self.interval.ok_or(None)?;
        let ready_at = self.next.map_or(now, |next| next.max(now));
        if ready_at > now {
            return Err(Some(ready_at));
        }
        self.next = Some(now + interval);
        Ok(())
    }
}

/// A minute's `per_minute`th, rounded up so `per_minute` admissions never
/// fit in less than a minute; none for a cap of 0.
fn interval(per_minute: u64) -> Option<Duration> {
    let minute = Duration::from_mins(1).as_nanos();
    let per_minute = u128::from(per_minute);
    if per_minute == 0 {
        return None;
    }
    let nanoseconds = minute.div_ceil(per_minute);
    Some(Duration::from_nanos(
        u64::try_from(nanoseconds).unwrap_or(u64::MAX),
    ))
}

/// The rate cap `settings` give a Run, `None` when unset.
pub(crate) fn rate_cap_of(settings: &Resolved) -> Option<u64> {
    settings.count(ADMISSION_MAX_REQUESTS_PER_MINUTE.name)
}

#[cfg(test)]
mod tests;
