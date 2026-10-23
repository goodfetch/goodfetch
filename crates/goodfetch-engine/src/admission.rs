//! A Run's admission (ADRs 0004, 0040, 0053 and 0087; RT-02): which of its
//! waiting requests goes next, and when. The gate in admission is an
//! interface a Product's composition passes in: Foxhound's per-domain
//! queues fill it, and a Run of a composition with none admits through no
//! gate. Beside it, the Run's rate cap, `admission.max_requests_per_minute`,
//! bounds every request it admits by the Generic Cell Rate Algorithm with
//! no burst, so admissions are spaced at least a minute's share apart. A
//! Run going on takes its settings again at each admission point, so a
//! tightened Operator policy cap reaches it there.
//!
//! When the gate names a rule for its keys, such as Foxhound's per-domain
//! Politeness, each request the gate would admit also takes a Permit under
//! its key from the Politeness the Run shares with the other runs of its
//! user, and gives it back once it ended: a key the Politeness refuses is
//! held, until the moment it names or until a wake says a Permit came back
//! (ADR 0053).
//!
//! What a gate's waiting requests need to survive a resume, it writes into
//! the run file as a [`Journal`], and it opens again from it. Every rule
//! here is a pure function of a monotonic reading the caller gives
//! (ADR 0059).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::settings::ADMISSION_MAX_REQUESTS_PER_MINUTE;
use rusqlite::Connection;
use tokio::sync::broadcast;
use tokio::time::Instant;

use crate::clock::Clock;
use crate::politeness::{Ending, Holder, Permit, PermitRule, Politeness, Taken, Wake};
use crate::settings::Resolved;

/// How long a key the Politeness holds for want of room waits for a wake
/// before its Run tries it again: only to find a holder that died without
/// giving its Permits back, whose Permits are then reclaimed. Every other
/// wait is woken or ends at its own deadline.
const RECLAIM_CHECK: Duration = Duration::from_secs(30);

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
    /// Whether it goes ahead of every request of its key waiting when it
    /// came, whatever their priority: of two such, the later goes first.
    pub forefront: bool,
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
    /// The site answered that too many requests came, a 429: its key's
    /// concurrency is halved and it backs off, until the moment its
    /// `Retry-After` names when it names one.
    TooManyRequests(Option<Instant>),
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

    /// Takes `request` into its key's queue; `false` when that queue
    /// already holds it, or is full, or may leave out requests of its key
    /// that go ahead of it, so it waits outside until the queue drains.
    fn enqueue(&mut self, request: Waiting) -> bool;

    /// When it next has a request to admit: at or before `now` when one is
    /// ready, and `None` when it holds none.
    fn ready_at(&self, now: Instant) -> Option<Instant>;

    /// The request it admits at `now`, if one is ready.
    fn admit(&mut self, now: Instant) -> Option<Admitted>;

    /// The key of the request [`Gate::admit`] would admit at `now`.
    fn next_key(&self, now: Instant) -> Option<String>;

    /// Holds `key`: none of its requests is ready before `until`, unless
    /// [`Gate::wake`] wakes it first. A held key keeps its place.
    fn hold(&mut self, key: &str, until: Instant);

    /// Ends the hold on `key`.
    fn wake(&mut self, key: &str);

    /// The rule of its keys' shared Politeness under `settings`, such as
    /// Foxhound's per-domain cap; `None` when its keys share none.
    fn rule(&self, settings: &Resolved) -> Option<Arc<dyn PermitRule>> {
        let _ = settings;
        None
    }

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

/// A Run's admission: its gate, its rate cap and the Politeness it shares,
/// which a request passes all of to go.
pub struct Admission {
    gate: Box<dyn Gate>,
    rate: Option<RateCap>,
    shared: Option<Shared>,
}

/// The Politeness a Run shares, and the Permits its requests in flight
/// hold.
struct Shared {
    politeness: Politeness,
    holder: Holder,
    rule: Arc<dyn PermitRule>,
    clock: Clock,
    permits: BTreeMap<String, Permit>,
    /// The keys held until a wake.
    waiting: BTreeSet<String>,
    wakes: Option<broadcast::Receiver<Wake>>,
}

impl fmt::Debug for Admission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Admission")
            .field("rate", &self.rate)
            .field("shared", &self.shared.is_some())
            .finish_non_exhaustive()
    }
}

/// The wakes a Run's held keys receive, which its driver waits on beside
/// its other events, and passes to [`Admission::wake`].
#[derive(Debug)]
pub struct Wakes {
    run_id: String,
    wakes: broadcast::Receiver<Wake>,
}

impl Wakes {
    /// The next key of its Run that a Permit given back woke; `None` when
    /// it fell behind, which wakes every held key.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no wake.
    pub async fn next(&mut self) -> Option<String> {
        loop {
            match self.wakes.recv().await {
                Ok(wake) if wake.run_id == self.run_id => return Some(wake.key),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => return None,
                Err(broadcast::error::RecvError::Closed) => std::future::pending().await,
            }
        }
    }
}

impl Admission {
    /// Admission through `gate` under the rate cap `settings` give.
    pub(crate) fn new(gate: Box<dyn Gate>, settings: &Resolved) -> Self {
        Self {
            gate,
            rate: rate_cap_of(settings).map(RateCap::per_minute),
            shared: None,
        }
    }

    /// The same, sharing `politeness` as `holder` under the rule its gate
    /// names for `settings`, reading the wall time from `clock`; unchanged
    /// when the gate names none.
    pub(crate) fn sharing(
        mut self,
        politeness: Politeness,
        holder: Holder,
        clock: Clock,
        settings: &Resolved,
    ) -> Self {
        if let Some(rule) = self.gate.rule(settings) {
            let wakes = Some(politeness.wakes());
            self.shared = Some(Shared {
                politeness,
                holder,
                rule,
                clock,
                permits: BTreeMap::new(),
                waiting: BTreeSet::new(),
                wakes,
            });
        }
        self
    }

    /// Whether its gate names a rule of shared Politeness under
    /// `settings`.
    pub(crate) fn shares(&self, settings: &Resolved) -> bool {
        self.gate.rule(settings).is_some()
    }

    /// Takes the rate cap of `settings`, and the rule of its shared
    /// Politeness, as a Run does at each admission point, keeping when it
    /// last admitted.
    pub fn update(&mut self, settings: &Resolved) {
        self.rate = match (self.rate, rate_cap_of(settings)) {
            (Some(rate), Some(per_minute)) => Some(rate.set_to(per_minute)),
            (None, Some(per_minute)) => Some(RateCap::per_minute(per_minute)),
            (_, None) => None,
        };
        if let Some(shared) = &mut self.shared
            && let Some(rule) = self.gate.rule(settings)
        {
            shared.rule = rule;
        }
    }

    /// The wakes of the keys its shared Politeness holds, once: `None` when
    /// it shares none, or they were taken.
    pub fn wakes(&mut self) -> Option<Wakes> {
        let shared = self.shared.as_mut()?;
        Some(Wakes {
            run_id: shared.holder.run_id.clone(),
            wakes: shared.wakes.take()?,
        })
    }

    /// Ends the hold on `key`, which a wake says may go again; every held
    /// key for `None`.
    pub fn wake(&mut self, key: Option<&str>) {
        let Some(shared) = &mut self.shared else {
            return;
        };
        let keys: Vec<String> = match key {
            Some(key) => shared.waiting.take(key).into_iter().collect(),
            None => std::mem::take(&mut shared.waiting).into_iter().collect(),
        };
        for key in keys {
            self.gate.wake(&key);
        }
    }

    /// The key a request to `url` waits under.
    pub fn key(&self, url: &str) -> String {
        self.gate.key(url)
    }

    /// How many requests of one key the gate holds at most.
    pub fn bound(&self) -> usize {
        self.gate.bound()
    }

    /// Takes `request` into its key's queue; `false` when the gate leaves
    /// it out (see [`Gate::enqueue`]).
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

    /// The request it admits at `now`, if one may go: when it shares
    /// Politeness, holding a Permit of its key, and each key the
    /// Politeness refuses held meanwhile.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a Permit taken for no request, which
    /// its Run's end or another engine's reclaim gives back.
    pub async fn admit(&mut self, now: Instant) -> Option<Admitted> {
        loop {
            if self.ready_at(now)? > now {
                return None;
            }
            let Some(shared) = &mut self.shared else {
                return self.admit_from_gate(now);
            };
            let key = self.gate.next_key(now)?;
            let wall = shared.clock.wall();
            let taken = shared
                .politeness
                .take(&key, &shared.holder, Arc::clone(&shared.rule), wall)
                .await;
            match taken {
                Ok(Taken::Granted(permit)) => {
                    let Some(admitted) = self.admit_from_gate(now) else {
                        self.give_back(permit, Ending::Done).await;
                        return None;
                    };
                    if let Some(shared) = &mut self.shared {
                        shared.permits.insert(admitted.id.clone(), permit);
                    }
                    return Some(admitted);
                }
                Ok(Taken::Wait(Some(until))) => {
                    let until = now + until.duration_since(wall).unwrap_or_default();
                    self.gate.hold(&key, until);
                }
                Ok(Taken::Wait(None)) => {
                    shared.waiting.insert(key.clone());
                    self.gate.hold(&key, now + RECLAIM_CHECK);
                }
                Err(error) => {
                    tracing::warn!(%error, run_id = %shared.holder.run_id, "the Politeness file refused a take; its key waits");
                    self.gate.hold(&key, now + RECLAIM_CHECK);
                }
            }
        }
    }

    fn admit_from_gate(&mut self, now: Instant) -> Option<Admitted> {
        let admitted = self.gate.admit(now)?;
        if let Some(rate) = &mut self.rate {
            let _ = rate.admit(now);
        }
        Some(admitted)
    }

    /// A request it admitted, `id` of `key`, ended at `now`, as `ended`
    /// says: its Permit goes back to the Politeness it shares, whose rule
    /// says when its key may go again, for this Run and every other
    /// sharing it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave its Permit held until its Run ends.
    pub async fn ended(&mut self, id: &str, key: &str, now: Instant, ended: Ended) {
        let permit = self
            .shared
            .as_mut()
            .and_then(|shared| shared.permits.remove(id));
        let Some(permit) = permit else {
            self.gate.ended(key, now, ended);
            return;
        };
        let wall = self.wall();
        let ending = match ended {
            Ended::Done => Ending::Done,
            Ended::BackOff => Ending::BackOff,
            Ended::Until(until) => Ending::Until(wall + until.saturating_duration_since(now)),
            Ended::TooManyRequests(until) => Ending::TooManyRequests(
                until.map(|until| wall + until.saturating_duration_since(now)),
            ),
        };
        let ready_at = self.give_back(permit, ending).await;
        let local = match ready_at.map(|ready_at| ready_at.duration_since(wall)) {
            Some(Ok(wait)) if !wait.is_zero() => Ended::Until(now + wait),
            _ => Ended::Done,
        };
        self.gate.ended(key, now, local);
        self.wake(Some(key));
    }

    /// Gives back every Permit it holds and withdraws its waiters, as its
    /// Run stops with requests still in flight.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the rest to another engine's reclaim.
    pub async fn close(&mut self) {
        let Some(shared) = &mut self.shared else {
            return;
        };
        let permits = std::mem::take(&mut shared.permits);
        let politeness = shared.politeness.clone();
        let holder = shared.holder.clone();
        for permit in permits.into_values() {
            self.give_back(permit, Ending::Done).await;
        }
        if let Err(error) = politeness.stop_waiting(&holder).await {
            tracing::warn!(%error, run_id = %holder.run_id, "a stopped Run's waiters were not withdrawn");
        }
    }

    fn wall(&self) -> SystemTime {
        self.shared
            .as_ref()
            .map_or(SystemTime::UNIX_EPOCH, |shared| shared.clock.wall())
    }

    /// Gives `permit` back; answers when its key may go again.
    async fn give_back(&mut self, permit: Permit, ending: Ending) -> Option<SystemTime> {
        let shared = self.shared.as_ref()?;
        let wall = shared.clock.wall();
        match shared
            .politeness
            .give_back(permit, Arc::clone(&shared.rule), ending, wall)
            .await
        {
            Ok(ready_at) => ready_at,
            Err(error) => {
                tracing::warn!(%error, run_id = %shared.holder.run_id, "a Permit was not given back; another engine reclaims it once its Run is gone");
                None
            }
        }
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
