//! A Run's admission (ADRs 0004, 0040, 0053 and 0087; RT-02): which of its
//! waiting requests goes next, and when. The gate in admission is an
//! interface a Product's composition passes in: Foxhound's per-domain
//! queues fill it, and a Run of a composition with none admits through no
//! gate. Beside it, the Run's rate cap, `admission.max_requests_per_minute`,
//! bounds every request it admits by the Generic Cell Rate Algorithm with
//! no burst, so admissions are spaced at least a minute's share apart. A
//! Run's admission takes its settings once, as it opens (ADR 0138).
//!
//! When the gate names a rule for its keys that paces them, such as
//! Foxhound's per-domain Politeness, each request the gate would admit also
//! takes a Permit under
//! its key from the Politeness the Run shares with the other runs of its
//! user, and gives it back once it ended: a key the Politeness refuses is
//! held, until the moment it names or until a wake says a Permit came back
//! (ADR 0053), and a key whose take the Politeness file could not answer is
//! held for that, until the Run tries it again. A key its Run refuses
//! until a moment, its site having asked for a time further away than the
//! Run waits (ADR 0118), comes with each of its requests refused until
//! then, unsent, whatever the Politeness says meanwhile. A rule that
//! paces nothing ([`SharedRule::paces`]) has the Run consult no Politeness
//! store at all: its gate backs its keys off itself (ADR 0138).
//!
//! A Run whose requests go through before-request Hooks takes each Permit
//! later, once the request's Hooks returned, in a second ask
//! ([`Admission::permit_after_hooks`], #2905): it admits a request, with
//! no Permit, only while its key is free as the Run sees it, so its Hooks
//! run while the Permit is still free for every other Run.
//!
//! What a gate's waiting requests need to survive a resume, it writes into
//! the run file as a [`Journal`], and it opens again from it. Every rule
//! here is a pure function of a monotonic reading the caller gives
//! (ADR 0059).
//!
//! A request already admitted may send a second copy beside its first,
//! such as a hedge (ADRs 0040 and 0086): [`SecondCopies`] gives the copy
//! room of its own, a Permit of its key under the same shared rule and a
//! share of the rate cap, so a key that admits one request at a time never
//! sends one, and one whose delay has not passed holds it until it has.
//!
//! Size: one concern, which waiting request a Run admits next and what
//! holds the rest: its gate, its rate cap and the Permits of the
//! Politeness it shares, read together because each admission takes all
//! three in one step and each Permit's take and give-back pass one state.
#![expect(
    clippy::disallowed_types,
    reason = "the rate cap is read and changed in one step, never across an `.await` or under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::Row;
use goodfetch_protocol::settings::ADMISSION_MAX_REQUESTS_PER_MINUTE;
use rusqlite::Connection;
use tokio::sync::broadcast;
use tokio::time::Instant;

use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::politeness::{
    Ending, Hold, Holder, OwnPoliteness, Permit, Politeness, PolitenessError, SharedRule, Taken,
    Wake,
};
use crate::run_state::SealedBatch;
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
    /// The time its key's site asked for, when that is further away than
    /// its Run waits: it is not sent, and ends (ADR 0118). Always `None`
    /// from a gate; the Politeness its Run shares sets it.
    pub refused_until: Option<SystemTime>,
}

/// How an admitted request ended, for its key's readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Its key may go again at once.
    Done,
    /// Its response asked its key to back off, and to wait until the
    /// moment its `Retry-After` names, if any, when that is later.
    BackOff(Option<Instant>),
    /// Its response asked its key to wait until this moment, such as a
    /// redirect's `Retry-After`.
    Until(Instant),
    /// The site answered that too many requests came, a 429: its key's
    /// concurrency is halved and it backs off, until the moment its
    /// `Retry-After` names when it names one.
    TooManyRequests(Option<Instant>),
}

/// Why a gate holds a key's requests (ADR 0102's wait reasons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// As many of its requests are in flight as it admits: until one ends.
    Concurrency,
    /// Its last request started less than the delay between starts ago.
    Delay,
    /// It backs off, or waits for the time a site asked for.
    BackOff,
    /// The shared Politeness file refused its take, such as one busy past
    /// its timeout: until its Run tries it again.
    PolitenessFile,
}

impl From<Hold> for Held {
    fn from(hold: Hold) -> Self {
        match hold {
            Hold::Delay => Self::Delay,
            Hold::BackOff => Self::BackOff,
        }
    }
}

/// The keys whose requests wait in a gate for one reason, as
/// [`Gate::queued`] answers them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedGroup {
    /// Why the gate holds them; `None` for the keys that are ready.
    pub held: Option<Held>,
    /// How many of their requests wait in the gate.
    pub requests: usize,
    /// How many keys.
    pub keys: usize,
    /// The first keys, as many as were asked for at most, each with until
    /// when the gate holds it, when it knows.
    pub sample: Vec<(String, Option<Instant>)>,
}

/// A Run's rate cap, `admission.max_requests_per_minute`, holding the
/// requests its gate would admit, as [`Admission::rate_held`] answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateHeld {
    /// When the cap next admits; `None` for a cap of 0, which admits
    /// nothing.
    pub until: Option<Instant>,
}

/// How a gate keys its requests' URLs, apart from the gate, so a write of
/// the run state that adds work items keys each as it adds it, as the gate
/// would key it ([`Gate::keys`]).
#[derive(Clone)]
pub struct Keys(Arc<dyn Fn(&str) -> String + Send + Sync>);

impl Keys {
    /// Keys each URL as `key` does.
    pub fn new(key: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        Self(Arc::new(key))
    }

    /// The key a request to `url` waits under.
    pub fn key(&self, url: &str) -> String {
        (self.0)(url)
    }
}

impl fmt::Debug for Keys {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Keys").finish_non_exhaustive()
    }
}

/// What a gate writes into the run file: rows it already decided, a sealed
/// batch for the file's writer (ADR 0142).
pub type Journal = SealedBatch;

/// One Run's gate in admission (ADR 0087): the queues its waiting
/// requests wait in, and the order and readiness they are admitted in.
pub trait Gate: Send {
    /// The key a request to `url` waits under.
    fn key(&self, url: &str) -> String;

    /// How it keys a request's URL, as [`Gate::key`] does, apart from it.
    fn keys(&self) -> Keys;

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

    /// How many requests of `key` wait in its queue, the one
    /// [`Gate::admit`] would admit next included: when more than one, a
    /// grant that fills the key's concurrency keeps its Run's turn for the
    /// next (ADR 0053's turns). None by default.
    fn waiting_under(&self, key: &str) -> usize {
        let _ = key;
        0
    }

    /// Holds `key` for the reason `held`: none of its requests is ready
    /// before `until`, unless [`Gate::wake`] wakes it first. A held key
    /// keeps its place.
    fn hold(&mut self, key: &str, until: Instant, held: Held);

    /// The keys whose requests wait in it at `now`, one group per reason it
    /// holds them for, the ready ones in a group of their own, each with at
    /// most `sample` of its keys: what a Run's progress says of its waits
    /// (ADR 0102). A gate keeps the groups as its holds, wakes, admissions
    /// and ends change them, so the answer costs the changes since the last
    /// one and the sample, never a walk of every key. None by default.
    fn queued(&mut self, now: Instant, sample: usize) -> Vec<QueuedGroup> {
        let _ = (now, sample);
        Vec::new()
    }

    /// How many requests wait in it, of every key, those of a key at its
    /// concurrency included, kept as they come and go. None by default.
    fn waiting(&self) -> usize {
        0
    }

    /// Ends the hold on `key`.
    fn wake(&mut self, key: &str);

    /// The rule of its keys' shared Politeness under `settings`, such as
    /// Foxhound's per-domain cap; `None` when its keys share none.
    fn rule(&self, settings: &Resolved) -> Option<SharedRule> {
        let _ = settings;
        None
    }

    /// A request of `key` it admitted ended at `now`, as `ended` says.
    fn ended(&mut self, key: &str, now: Instant, ended: Ended);

    /// Takes the waiting request `id` of `key` out of its queue, never to
    /// be admitted: its Run ended it before it was sent, such as a crawl
    /// request a learned rule merged. Nothing by default.
    fn withdraw(&mut self, key: &str, id: &str) {
        let _ = (key, id);
    }

    /// What changed since its last journal, to write into the run file, its
    /// moments as wall time read at `wall` when the monotonic clock read
    /// `now`.
    fn journal(&mut self, wall: SystemTime, now: Instant) -> Option<Journal>;
}

/// What a Product's composition passes in: opens a Run's gate.
pub trait Gates: Send + Sync {
    /// How the gate of a Run with the settings `settings` keys a request's
    /// URL, as its [`Gate::keys`] does, before the gate is open: a start
    /// keys its seeds in the write that adds them.
    fn keys(&self, settings: &Resolved) -> Keys;

    /// The gate of a Run with the settings `settings`, from what its run
    /// file `journal` holds, read at `wall` when the monotonic clock read
    /// `now`, drawing its random choices, such as a backoff's jitter, from
    /// `entropy`.
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
        entropy: Entropy,
    ) -> rusqlite::Result<Box<dyn Gate>>;
}

/// A Run's admission: its gate, its rate cap and the Politeness it shares,
/// which a request passes all of to go.
pub struct Admission {
    gate: Box<dyn Gate>,
    /// Shared with its second copies, which each take a share of it too.
    rate: SharedRate,
    shared: Option<Shared>,
    /// The keys whose site asked for a time further away than the Run
    /// waits, judged when it asked: until when their requests are refused,
    /// and that time ([`Admission::refuse`]).
    refused: BTreeMap<String, (Instant, SystemTime)>,
}

/// A Run's rate cap, which its admission and its second copies share.
type SharedRate = Arc<Mutex<Option<RateCap>>>;

/// Locks `rate`, whose holders never panic while they hold it.
fn lock(rate: &SharedRate) -> MutexGuard<'_, Option<RateCap>> {
    rate.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The Politeness a Run shares, and the Permits its requests in flight
/// hold.
struct Shared {
    politeness: Politeness,
    /// The Politeness of the Run's own it holds, which `politeness` is the
    /// handle of: its takes and give backs then wait on nothing, and its
    /// Run's driver serves what the Run's other tasks send it
    /// ([`Admission::serve_politeness`], #2171).
    own: Option<OwnPoliteness>,
    holder: Holder,
    rule: SharedRule,
    clock: Clock,
    permits: BTreeMap<String, Permit>,
    /// The requests it admitted that hold no Permit yet, each ID with its
    /// key, while its Run takes each Permit only once the request's
    /// before-request Hooks returned ([`Admission::permit_after_hooks`]);
    /// `None` while it takes each as it admits the request.
    unpermitted: Option<BTreeMap<String, String>>,
    /// The keys held until a wake.
    waiting: BTreeSet<String>,
    /// The keys whose waiter turn its holder took since its last grant of
    /// each: a refused take made it a waiter (ADR 0053's turns).
    turns: BTreeSet<String>,
    wakes: Option<broadcast::Receiver<Wake>>,
}

impl Shared {
    /// Whether the Permits its own requests in flight hold under `key`,
    /// with those admitted that will ask for one and `more` besides, reach
    /// its rule's concurrency, so a take could grant nothing.
    fn full_with(&self, key: &str, more: usize) -> bool {
        self.rule.concurrency().is_some_and(|cap| {
            let unpermitted = self.unpermitted.as_ref().map_or(0, |unpermitted| {
                unpermitted.values().filter(|under| *under == key).count()
            });
            let held = self
                .permits
                .values()
                .filter(|permit| permit.key() == key)
                .count()
                + unpermitted
                + more;
            u64::try_from(held).unwrap_or(u64::MAX) >= cap
        })
    }

    /// Takes a Permit of `key` at `wall`, from the Politeness of its Run's
    /// own it holds when it holds one; a grant on the shared Politeness
    /// keeps its holder's turn when `keeps_turn`.
    async fn take(
        &mut self,
        key: &str,
        wall: SystemTime,
        keeps_turn: bool,
    ) -> Result<Taken, PolitenessError> {
        match &mut self.own {
            Some(own) => Ok(own.take(key, &self.holder, &self.rule, wall)),
            None if keeps_turn => {
                self.politeness
                    .take_keeping_turn(key, &self.holder, self.rule.clone(), wall)
                    .await
            }
            None => {
                self.politeness
                    .take(key, &self.holder, self.rule.clone(), wall)
                    .await
            }
        }
    }

    /// Gives `permit` back now, as its request ended as `ending`; answers
    /// when its key may go again.
    async fn give_back(
        &mut self,
        permit: Permit,
        ending: Ending,
    ) -> Result<Option<SystemTime>, PolitenessError> {
        let wall = self.clock.wall();
        match &mut self.own {
            Some(own) => Ok(own.give_back(&permit, &self.rule, ending, wall)),
            None => {
                self.politeness
                    .give_back(permit, self.rule.clone(), ending, wall)
                    .await
            }
        }
    }

    /// Withdraws its holder as a waiter of every key.
    async fn stop_waiting(&mut self) -> Result<(), PolitenessError> {
        match &mut self.own {
            Some(own) => {
                own.stop_waiting(&self.holder);
                Ok(())
            }
            None => {
                self.politeness
                    .stop_waiting(&self.holder, self.clock.wall())
                    .await
            }
        }
    }
}

impl fmt::Debug for Admission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Admission")
            .field("rate", &*lock(&self.rate))
            .field("shared", &self.shared.is_some())
            .finish_non_exhaustive()
    }
}

/// What a request admitted with no Permit is answered as it asks for one
/// ([`Admission::permit`]).
#[derive(Debug)]
pub enum PermitAnswer {
    /// It holds its Permit: it is sent, at the moment its grant reserved
    /// when it reserved one ([`Admission::reserved_send`]).
    Granted(Option<SystemTime>),
    /// It asks again at `until`, or at the wake of its key `wakes` receives
    /// first, when a request's end is what makes room.
    Wait {
        until: Instant,
        wakes: Option<Wakes>,
    },
    /// Its site asked for no request to its domain before this time, further
    /// away than its Run waits (ADR 0118): it ends unsent.
    Refused(SystemTime),
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
            rate: Arc::new(Mutex::new(rate_cap_of(settings).map(RateCap::per_minute))),
            shared: None,
            refused: BTreeMap::new(),
        }
    }

    /// The same, sharing `politeness` as `holder` under the rule its gate
    /// names for `settings`, reading the wall time from `clock`, holding
    /// `own` when it is a Politeness of the Run's own that `politeness` is
    /// the handle of; unchanged when the gate names none, or one that
    /// paces nothing.
    pub(crate) fn sharing(
        mut self,
        (politeness, own): (Politeness, Option<OwnPoliteness>),
        holder: Holder,
        clock: Clock,
        settings: &Resolved,
    ) -> Self {
        if let Some(rule) = self.gate.rule(settings).filter(SharedRule::paces) {
            let wakes = Some(politeness.wakes());
            self.shared = Some(Shared {
                politeness,
                own,
                holder,
                rule,
                clock,
                permits: BTreeMap::new(),
                unpermitted: None,
                waiting: BTreeSet::new(),
                turns: BTreeSet::new(),
                wakes,
            });
        }
        self
    }

    /// Has its Run take each request's Permit only once the request's
    /// before-request Hooks returned, in a second ask ([`Admission::permit`]),
    /// as a Run with such a Hook does (#2905, the owner's decision of
    /// 2026-11-07 on #1546): it admits a request, holding no Permit, only
    /// while its key is free as the Run sees it, its own Permits and the
    /// requests that will ask for one short of the rule's concurrency and
    /// its gate holding it for nothing. Nothing changes when it shares no
    /// Politeness.
    pub fn permit_after_hooks(&mut self) {
        if let Some(shared) = &mut self.shared {
            shared.unpermitted.get_or_insert_with(BTreeMap::new);
        }
    }

    /// Whether its Run takes each Permit after its request's Hooks
    /// ([`Admission::permit_after_hooks`]).
    pub fn permits_after_hooks(&self) -> bool {
        self.shared
            .as_ref()
            .is_some_and(|shared| shared.unpermitted.is_some())
    }

    /// Whether its gate names a rule of shared Politeness under
    /// `settings` that paces its keys (ADR 0138).
    pub(crate) fn shares(&self, settings: &Resolved) -> bool {
        self.gate.rule(settings).is_some_and(|rule| rule.paces())
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

    /// The keys whose requests wait in its gate at `now`, by reason, each
    /// group with at most `sample` keys, as [`Gate::queued`] says.
    pub fn queued(&mut self, now: Instant, sample: usize) -> Vec<QueuedGroup> {
        self.gate.queued(now, sample)
    }

    /// The key a request to `url` waits under.
    pub fn key(&self, url: &str) -> String {
        self.gate.key(url)
    }

    /// How its gate keys a request's URL, apart from it ([`Gate::keys`]).
    pub fn keys(&self) -> Keys {
        self.gate.keys()
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

    /// Takes the waiting request `id` of `key` out of the gate, never to be
    /// admitted (see [`Gate::withdraw`]).
    pub fn withdraw(&mut self, key: &str, id: &str) {
        self.gate.withdraw(key, id);
    }

    /// Whether its rate cap holds at `now` the requests its gate would
    /// admit, and until when: `None` when it has no rate cap or the cap
    /// admits now (ADR 0102's `rate_cap` wait).
    pub fn rate_held(&self, now: Instant) -> Option<RateHeld> {
        let rate = (*lock(&self.rate))?;
        match rate.ready_at(now) {
            Some(ready) if ready <= now => None,
            until => Some(RateHeld { until }),
        }
    }

    /// When it next admits a request: at or before `now` when one may go
    /// now, `None` when none is waiting or the rate cap admits nothing.
    pub fn ready_at(&self, now: Instant) -> Option<Instant> {
        let gate = self.gate.ready_at(now)?;
        match *lock(&self.rate) {
            Some(rate) => Some(rate.ready_at(now)?.max(gate)),
            None => Some(gate),
        }
    }

    /// The request it admits at `now`, if one may go: when it shares
    /// Politeness, holding a Permit of its key, and each key the
    /// Politeness refuses held meanwhile. A request of a key whose site
    /// asked for a time further away than the Run waits comes with
    /// [`Admitted::refused_until`] and no Permit: it is not sent, takes no
    /// share of the rate cap, and its Run ends it.
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
            if !self.refused.is_empty()
                && let Some(key) = self.gate.next_key(now)
                && let Some(asked) = self.refused_at(&key, now)
            {
                let mut admitted = self.gate.admit(now)?;
                admitted.refused_until = Some(asked);
                return Some(admitted);
            }
            let Some(shared) = &mut self.shared else {
                return self.admit_from_gate(now);
            };
            let key = self.gate.next_key(now)?;
            if shared.unpermitted.is_some() {
                if shared.full_with(&key, 0) {
                    // The key is not free: its next request waits for one
                    // of the Run's own to end, whose ending wakes it.
                    shared.waiting.insert(key.clone());
                    self.gate.hold(&key, now + RECLAIM_CHECK, Held::Concurrency);
                    continue;
                }
                let admitted = self.admit_from_gate(now)?;
                if let Some(unpermitted) = self
                    .shared
                    .as_mut()
                    .and_then(|shared| shared.unpermitted.as_mut())
                {
                    unpermitted.insert(admitted.id.clone(), admitted.key.clone());
                }
                return Some(admitted);
            }
            if shared.full_with(&key, 0) && shared.turns.contains(&key) {
                // Its own requests in flight fill the key and it already
                // waits in turn for it: a take would grant nothing, so the
                // key waits for their ending's wake, or the reclaim check,
                // and the next take answers what a back-off or a refusal
                // asks. Its first refused take still makes it a waiter, so
                // another engine that waits after it never goes first.
                shared.waiting.insert(key.clone());
                self.gate.hold(&key, now + RECLAIM_CHECK, Held::Concurrency);
                continue;
            }
            // A grant that fills the key while another of its requests
            // waits under it keeps the Run's turn for that one, as the
            // refused take that would follow did: the Run asks the file
            // nothing more until one of its requests ends.
            let keeps_turn = shared.own.is_none()
                && shared.full_with(&key, 1)
                && self.gate.waiting_under(&key) > 1;
            let wall = shared.clock.wall();
            let taken = shared.take(&key, wall, keeps_turn).await;
            match taken {
                Ok(Taken::Granted(permit)) => {
                    if keeps_turn {
                        shared.turns.insert(key.clone());
                    } else {
                        shared.turns.remove(&key);
                    }
                    let Some(admitted) = self.admit_from_gate(now) else {
                        self.give_back(permit, Ending::Done).await;
                        return None;
                    };
                    if let Some(shared) = &mut self.shared {
                        shared.permits.insert(admitted.id.clone(), permit);
                    }
                    return Some(admitted);
                }
                Ok(Taken::Wait(Some((until, hold)))) => {
                    let until = now + until.duration_since(wall).unwrap_or_default();
                    self.gate.hold(&key, until, hold.into());
                }
                Ok(Taken::Refused(until)) => {
                    let mut admitted = self.gate.admit(now)?;
                    admitted.refused_until = Some(until);
                    return Some(admitted);
                }
                Ok(Taken::Wait(None)) => {
                    shared.turns.insert(key.clone());
                    shared.waiting.insert(key.clone());
                    self.gate.hold(&key, now + RECLAIM_CHECK, Held::Concurrency);
                }
                Err(error) => {
                    tracing::warn!(%error, run_id = %shared.holder.run_id, "the Politeness file refused a take; its key waits");
                    self.gate
                        .hold(&key, now + RECLAIM_CHECK, Held::PolitenessFile);
                }
            }
        }
    }

    /// Takes the Permit of the request it admitted, `id` of `key`, at
    /// `now`, once the request's before-request Hooks returned
    /// ([`Admission::permit_after_hooks`]): the answer its request is sent,
    /// waits or ends unsent on. A request that already holds its Permit,
    /// one that already ended and one of a Run that takes its Permits as it
    /// admits are granted at once, with no take.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a Permit taken for no request, which
    /// its Run's end or another engine's reclaim gives back.
    pub async fn permit(&mut self, id: &str, key: &str, now: Instant) -> PermitAnswer {
        if let Some(asked) = self.refused_at(key, now) {
            return PermitAnswer::Refused(asked);
        }
        let Some(shared) = &mut self.shared else {
            return PermitAnswer::Granted(None);
        };
        if let Some(permit) = shared.permits.get(id) {
            return PermitAnswer::Granted(permit.send_at());
        }
        if !shared
            .unpermitted
            .as_ref()
            .is_some_and(|unpermitted| unpermitted.contains_key(id))
        {
            return PermitAnswer::Granted(None);
        }
        // Subscribed before the take, so the wake of a give back that
        // follows its refusal is never missed.
        let wakes = Wakes {
            run_id: shared.holder.run_id.clone(),
            wakes: shared.politeness.wakes(),
        };
        let wall = shared.clock.wall();
        match shared.take(key, wall, false).await {
            Ok(Taken::Granted(permit)) => {
                shared.turns.remove(key);
                if let Some(unpermitted) = &mut shared.unpermitted {
                    unpermitted.remove(id);
                }
                let send_at = permit.send_at();
                shared.permits.insert(id.to_owned(), permit);
                PermitAnswer::Granted(send_at)
            }
            Ok(Taken::Wait(Some((until, _)))) => PermitAnswer::Wait {
                until: now + until.duration_since(wall).unwrap_or_default(),
                wakes: None,
            },
            Ok(Taken::Wait(None)) => PermitAnswer::Wait {
                until: now + RECLAIM_CHECK,
                wakes: Some(wakes),
            },
            Ok(Taken::Refused(until)) => PermitAnswer::Refused(until),
            Err(error) => {
                tracing::warn!(%error, run_id = %shared.holder.run_id, "the Politeness file refused a take; its request waits");
                PermitAnswer::Wait {
                    until: now + RECLAIM_CHECK,
                    wakes: None,
                }
            }
        }
    }

    fn admit_from_gate(&mut self, now: Instant) -> Option<Admitted> {
        let admitted = self.gate.admit(now)?;
        if let Some(rate) = lock(&self.rate).as_mut() {
            let _ = rate.admit(now);
        }
        Some(admitted)
    }

    /// Refuses the requests of `key` it admits before `until`: each comes
    /// with `asked`, the time its site asked for, as
    /// [`Admitted::refused_until`], unsent, however late it is admitted.
    /// Its Run judged that time further away than it waits at the moment
    /// the site asked (ADR 0118), so the time its Run took to learn of it
    /// never turns the refusal into a wait. Its gate holds the key for no
    /// ending meanwhile, so its waiting requests end at once.
    pub fn refuse(&mut self, key: &str, until: Instant, asked: SystemTime) {
        self.refused.insert(key.to_owned(), (until, asked));
    }

    /// The time `key`'s site asked for, while [`Admission::refuse`] refuses
    /// its requests at `now`.
    fn refused_at(&mut self, key: &str, now: Instant) -> Option<SystemTime> {
        match self.refused.get(key) {
            Some((until, asked)) if *until > now => Some(*asked),
            Some(_) => {
                self.refused.remove(key);
                None
            }
            None => None,
        }
    }

    /// The moment the request it admitted, `id`, is to be sent, which its
    /// Permit's grant reserved on a Politeness held in memory
    /// ([`Permit::send_at`]); `None` when it holds no such Permit.
    pub fn reserved_send(&self, id: &str) -> Option<SystemTime> {
        self.shared
            .as_ref()
            .and_then(|shared| shared.permits.get(id))
            .and_then(Permit::send_at)
    }

    /// A request it admitted, `id` of `key`, ended at `now`, as `ended`
    /// says: its Permit goes back to the Politeness it shares, whose rule
    /// says when its key may go again, for this Run and every other
    /// sharing it. A key it refuses ([`Admission::refuse`]) is not held
    /// for it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave its Permit held until its Run ends.
    pub async fn ended(&mut self, id: &str, key: &str, now: Instant, ended: Ended) {
        let refused = self.refused_at(key, now).is_some();
        let permit = self
            .shared
            .as_mut()
            .and_then(|shared| shared.permits.remove(id));
        let Some(permit) = permit else {
            let unpermitted = self
                .shared
                .as_mut()
                .and_then(|shared| shared.unpermitted.as_mut())
                .and_then(|unpermitted| unpermitted.remove(id))
                .is_some();
            self.gate
                .ended(key, now, if refused { Ended::Done } else { ended });
            // Its key's next request waited for it to end.
            if unpermitted {
                self.wake(Some(key));
            }
            return;
        };
        let wall = self.wall();
        let ending = match ended {
            Ended::Done => Ending::Done,
            Ended::BackOff(until) => {
                Ending::BackOff(until.map(|until| wall + until.saturating_duration_since(now)))
            }
            Ended::Until(until) => Ending::Until(wall + until.saturating_duration_since(now)),
            Ended::TooManyRequests(until) => Ending::TooManyRequests(
                until.map(|until| wall + until.saturating_duration_since(now)),
            ),
        };
        let ready_at = self.give_back(permit, ending).await;
        let local = match ready_at.map(|ready_at| ready_at.duration_since(wall)) {
            Some(Ok(wait)) if !wait.is_zero() && !refused => Ended::Until(now + wait),
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
        for permit in permits.into_values() {
            self.give_back(permit, Ending::Done).await;
        }
        let Some(shared) = &mut self.shared else {
            return;
        };
        if let Some(unpermitted) = &mut shared.unpermitted {
            unpermitted.clear();
        }
        shared.turns.clear();
        if let Err(error) = shared.stop_waiting().await {
            tracing::warn!(%error, run_id = %shared.holder.run_id, "a stopped Run's waiters were not withdrawn");
        }
    }

    /// Runs the jobs the Run's other tasks send the Politeness of its own
    /// it holds, as [`OwnPoliteness::serve`] does: its Run's driver waits
    /// on it beside its other events while the Run goes. Pending for ever
    /// when it holds none.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no job.
    pub async fn serve_politeness(&mut self) {
        match self.shared.as_mut().and_then(|shared| shared.own.as_mut()) {
            Some(own) => own.serve().await,
            None => std::future::pending().await,
        }
    }

    fn wall(&self) -> SystemTime {
        self.shared
            .as_ref()
            .map_or(SystemTime::UNIX_EPOCH, |shared| shared.clock.wall())
    }

    /// Gives `permit` back; answers when its key may go again.
    async fn give_back(&mut self, permit: Permit, ending: Ending) -> Option<SystemTime> {
        let shared = self.shared.as_mut()?;
        match shared.give_back(permit, ending).await {
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

impl Admission {
    /// Room for second copies of the requests it admitted, under the rule
    /// of its shared Politeness and its rate cap as they stand now.
    pub fn second_copies(&self) -> SecondCopies {
        SecondCopies {
            shared: self.shared.as_ref().map(|shared| BesideShared {
                politeness: shared.politeness.clone(),
                holder: shared.holder.clone(),
                rule: shared.rule.clone(),
                clock: shared.clock,
            }),
            rate: Arc::clone(&self.rate),
        }
    }
}

/// Room for a second copy of a request a Run already admitted, such as a
/// hedge's (ADRs 0040 and 0086): the copy is governed as a request of its
/// own, so it takes a Permit of its key from the Politeness the Run shares,
/// beside its first's, and a share of the Run's rate cap. Cheap to clone;
/// every clone shares the rate cap.
#[derive(Clone)]
pub struct SecondCopies {
    shared: Option<BesideShared>,
    rate: SharedRate,
}

impl fmt::Debug for SecondCopies {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecondCopies")
            .field("rate", &*lock(&self.rate))
            .field("shared", &self.shared.is_some())
            .finish_non_exhaustive()
    }
}

/// The Politeness a second copy takes its Permit from.
#[derive(Clone)]
struct BesideShared {
    politeness: Politeness,
    holder: Holder,
    rule: SharedRule,
    clock: Clock,
}

/// What a second copy's take answered.
#[derive(Debug)]
pub enum Room {
    /// The copy may go, holding this room until it ends.
    Taken(SecondCopy),
    /// Not before this moment, such as the end of its key's delay or of the
    /// rate cap's interval: try again then.
    Until(Instant),
    /// Not while the requests in flight under its key stay, nor at all
    /// when its key's site asked for a time further away than the Run
    /// waits: the copy is not sent.
    Full,
}

/// The room a second copy holds while it is in flight.
#[derive(Debug)]
pub struct SecondCopy {
    permit: Option<Permit>,
}

impl SecondCopies {
    /// Takes room for a second copy of a request of `key` at `now`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a Permit taken for no copy, which its
    /// Run's end or another engine's reclaim gives back.
    pub async fn take(&self, key: &str, now: Instant) -> Room {
        let rate_ready = match *lock(&self.rate) {
            Some(rate) => match rate.ready_at(now) {
                Some(ready_at) if ready_at > now => return Room::Until(ready_at),
                Some(_) => true,
                None => return Room::Full,
            },
            None => false,
        };
        let permit = match &self.shared {
            None => None,
            Some(shared) => {
                let wall = shared.clock.wall();
                shared.politeness.work().count(Row::SecondCopyRoundTrips);
                let taken = shared
                    .politeness
                    .take_beside(key, &shared.holder, shared.rule.clone(), wall)
                    .await;
                match taken {
                    Ok(Taken::Granted(permit)) => Some(permit),
                    Ok(Taken::Wait(Some((until, _)))) => {
                        return Room::Until(now + until.duration_since(wall).unwrap_or_default());
                    }
                    Ok(Taken::Wait(None) | Taken::Refused(_)) => return Room::Full,
                    Err(error) => {
                        tracing::warn!(%error, run_id = %shared.holder.run_id, "the Politeness file refused a second copy's take; it is not sent");
                        return Room::Full;
                    }
                }
            }
        };
        let copy = SecondCopy { permit };
        let admitted = if rate_ready {
            lock(&self.rate)
                .as_mut()
                .map_or(Ok(()), |rate| rate.admit(now))
        } else {
            Ok(())
        };
        if let Err(ready_at) = admitted {
            // The Run admitted a request of its own meanwhile.
            self.give_back(copy).await;
            return ready_at.map_or(Room::Full, Room::Until);
        }
        Room::Taken(copy)
    }

    /// Gives back the room `copy` held, once it ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave its Permit held until its Run ends,
    /// or another engine's reclaim.
    pub async fn give_back(&self, copy: SecondCopy) {
        let (Some(shared), Some(permit)) = (&self.shared, copy.permit) else {
            return;
        };
        let wall = shared.clock.wall();
        if let Err(error) = shared
            .politeness
            .note_given_back(permit, shared.rule.clone(), Ending::Done, wall)
            .await
        {
            tracing::warn!(%error, run_id = %shared.holder.run_id, "a second copy's Permit was not given back; another engine reclaims it once its Run is gone");
        }
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
    settings.get(&ADMISSION_MAX_REQUESTS_PER_MINUTE)
}

#[cfg(test)]
mod tests;
