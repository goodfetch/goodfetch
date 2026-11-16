//! A crawl's per-domain Politeness (ADRs 0021, 0053, 0086 and 0118): the
//! rule its domain keys follow in the per-user Politeness file, so every
//! run of the user that shares Politeness, in any engine, obeys it
//! together. A run admits a request to a domain key only while the count in
//! flight there, across every sharing run, is below its own
//! `politeness.domain.concurrency` and below the key's ceiling a 429
//! lowered; once its own `politeness.domain.delay` passed since the last
//! request start there by any sharing run, raised to the largest
//! Crawl-delay the robots.txt of the key's hosts ask for, as any sharing
//! run read it, up to its own `site_preferences.max_crawl_delay` while
//! `site_preferences.respect_robots_txt` is on (ADR 0086; a longer one
//! leaves the delay, its host's requests skipped); and once a `Retry-After` or a
//! backoff any sharing run met has ended. Each wait of a backoff is drawn
//! with decorrelated jitter ([`Backoff`]), between
//! `politeness.backoff.initial` and three times the previous wait, never
//! above `politeness.backoff.max`. A `Retry-After` holds the key until the
//! time the site asked, when that is later than the backoff: a run waits
//! for it up to its `politeness.retry_after.max`, and refuses its requests
//! unsent while the time is further away ([`Verdict::Refused`]). A run with
//! `politeness.backoff.enabled` off ignores backoffs and `Retry-After`, its
//! own and every other run's. A 429 halves the key's ceiling (from the
//! count in flight when it had none) and backs it off; the ceiling then
//! grows back by one with each request that succeeds after a quiet
//! [`RECOVERY`] since it last changed. Each rule is a pure function of the
//! wall time it is given and of the `Entropy` its gate holds (ADR 0059).
//!
//! A crawl's pacing (spec #582's Settings) adds two opt-in rules on top,
//! neither pressing harder than the delay alone. With
//! `politeness.domain.adaptive_delay` on, the delay between starts is also
//! at least the exponentially weighted mean (weight 0.3) of the key's
//! responses' latency, as any sharing run measured it, that mean capped at
//! `politeness.backoff.max`. With `politeness.domain.rate`, a run also
//! starts a request only while the key's token bucket, which every sharing
//! run's rated starts draw on, holds a token (a GCRA of
//! `politeness.domain.burst` tokens); a start counts from its request's
//! send when that is later than its admission. A crawl request's own send
//! also waits for its delay after the key's last send and for a token of
//! the key's sends ([`Pace`]), so a send that came later than the next
//! request's admission is never overtaken within the delay or the
//! interval.
//!
//! The per-domain state lives in the `domain_limits`, `crawl_delays` and
//! `domain_pacing` tables Foxhound adds to the file; a Collie engine adds
//! none and touches none (ADR 0098).
//!
//! A run whose rule paces nothing ([`paces`]: no
//! `politeness.domain.concurrency`, no delay, no rate, no adaptive delay
//! and no Crawl-delay, robots.txt not obeyed) consults no Politeness store
//! (ADR 0138): its per-domain queues back its domains off themselves, by
//! the same rule over a domain's state they keep in memory, so the backoff,
//! a `Retry-After` and a 429's halving stay their own features.
//!
//! Size: one concern, a domain key's shared state and the rule over it:
//! its limits, pacing and Crawl-delays as plain values, how the Politeness
//! file loads and saves them, and the steps that change them, which read
//! side by side so the rule and the state it reads change together.

#![expect(
    clippy::disallowed_types,
    reason = "the rule runs inside the Politeness file's transaction, a blocking call that may run inline on a runtime thread: one Mutex guards a gate's Entropy for one draw, never held with another lock or across an await, so there is no lock order to test under loom"
)]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::politeness::{
    Ending, Hold, KeyState, MemoryTables, PermitRule, Politeness, PolitenessError,
    PolitenessTables, Step, Verdict, milliseconds, moment,
};
use goodfetch_engine::settings::{Resolved, catalog_default};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_BACKOFF_ENABLED, POLITENESS_BACKOFF_INITIAL, POLITENESS_BACKOFF_MAX,
    POLITENESS_DOMAIN_CONCURRENCY, POLITENESS_RETRY_AFTER_MAX,
};
use goodfetch_protocol::settings::{SETTING_NOT_APPLICABLE, SettingNotApplicableDetails};
use rusqlite::{Connection, OptionalExtension as _};

use crate::robots_txt;
use serde_json::Value;

mod pace;

pub use pace::Pace;

/// How long a lowered ceiling stays quiet before it grows back by one.
pub(crate) const RECOVERY: Duration = Duration::from_secs(10);

/// The table the rule keeps each domain key's shared state in.
pub(crate) const TABLES: PolitenessTables = PolitenessTables {
    name: "goodfetch-frontier",
    statements: &[
        "CREATE TABLE IF NOT EXISTS domain_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoff INTEGER,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER,
            last_send INTEGER
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS crawl_delays (
            key TEXT NOT NULL,
            location TEXT NOT NULL,
            delay INTEGER NOT NULL,
            PRIMARY KEY (key, location)
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS domain_pacing (
            key TEXT NOT NULL PRIMARY KEY,
            latency INTEGER,
            theoretical_start INTEGER,
            interval INTEGER,
            theoretical_send INTEGER
        ) STRICT",
    ],
    prepared: &[
        READ_LIMITS,
        WRITE_LIMITS,
        READ_PACING,
        WRITE_PACING,
        ASK_CRAWL_DELAY,
        DROP_CRAWL_DELAY,
        CRAWL_DELAYS,
    ],
};

// The statements the rule runs on the Politeness file, prepared when it opens.
const READ_LIMITS: &str =
    "SELECT ready_at, backoff, ceiling, ceiling_changed_at, last_start, last_send
    FROM domain_limits WHERE key = ?1";
const WRITE_LIMITS: &str = "INSERT INTO domain_limits
    (key, ready_at, backoff, ceiling, ceiling_changed_at, last_start, last_send)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
    ON CONFLICT (key) DO UPDATE SET ready_at = ?2, backoff = ?3, ceiling = ?4,
    ceiling_changed_at = ?5, last_start = ?6, last_send = ?7";
const READ_PACING: &str = "SELECT latency, theoretical_start, interval, theoretical_send
    FROM domain_pacing WHERE key = ?1";
const WRITE_PACING: &str =
    "INSERT INTO domain_pacing (key, latency, theoretical_start, interval, theoretical_send)
    VALUES (?1, ?2, ?3, ?4, ?5)
    ON CONFLICT (key) DO UPDATE SET latency = ?2, theoretical_start = ?3, interval = ?4,
    theoretical_send = ?5";
const ASK_CRAWL_DELAY: &str = "INSERT INTO crawl_delays (key, location, delay) VALUES (?1, ?2, ?3)
    ON CONFLICT (key, location) DO UPDATE SET delay = ?3";
const DROP_CRAWL_DELAY: &str = "DELETE FROM crawl_delays WHERE key = ?1 AND location = ?2";
const CRAWL_DELAYS: &str = "SELECT location, delay FROM crawl_delays WHERE key = ?1";

/// The `Entropy` a crawl's gate draws its backoffs from, shared by the
/// rules it names.
pub(crate) type SharedEntropy = Arc<Mutex<Entropy>>;

/// A domain's backoff under a run's settings: each wait drawn with
/// decorrelated jitter (ADR 0118).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Backoff {
    /// `politeness.backoff.initial`.
    initial: Duration,
    /// `politeness.backoff.max`.
    max: Duration,
}

impl Backoff {
    /// The wait that follows `previous` in a row, `None` for the first:
    /// drawn from `entropy` between `initial` and three times `previous`
    /// (three times `initial` for the first), and never above `max` nor
    /// below `initial`, which a `max` shorter than `initial` leaves as the
    /// only wait.
    pub(crate) fn next(&self, previous: Option<Duration>, entropy: &mut Entropy) -> Duration {
        let low = whole_milliseconds(self.initial);
        let high = whole_milliseconds(previous.unwrap_or(self.initial))
            .saturating_mul(3)
            .min(whole_milliseconds(self.max))
            .max(low);
        let drawn = match (high - low).checked_add(1) {
            Some(choices) => entropy.next_u64() % choices,
            None => entropy.next_u64(),
        };
        Duration::from_millis(low + drawn)
    }

    /// Its longest wait.
    fn max(&self) -> Duration {
        self.max.max(self.initial)
    }
}

fn whole_milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// One domain key's shared state, or the state a run that paces nothing
/// keeps of it in memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Limits {
    /// No request before this moment, in milliseconds.
    pub(crate) ready_at: Option<i64>,
    /// The last wait of the backoffs in a row, in milliseconds; `None`
    /// once a request succeeded.
    backoff: Option<i64>,
    /// The most requests in flight a 429 left it.
    pub(crate) ceiling: Option<u64>,
    /// When the ceiling last changed, in milliseconds.
    ceiling_changed_at: Option<i64>,
    /// The last request start by any sharing run, in milliseconds.
    last_start: Option<i64>,
    /// The last send any sharing run recorded when it sent, in
    /// milliseconds: what a crawl request's own send counts its delay from.
    last_send: Option<i64>,
}

impl Limits {
    /// Whether it keeps nothing of a backoff or a 429 for its key: no
    /// backoff in a row, and no ceiling.
    pub(crate) fn is_clear(&self) -> bool {
        self.backoff.is_none() && self.ceiling.is_none()
    }

    fn read(connection: &Connection, key: &str) -> rusqlite::Result<Self> {
        Ok(connection
            .prepare_cached(READ_LIMITS)
            .and_then(|mut cached| {
                cached.query_row([key], |row| {
                    Ok(Self {
                        ready_at: row.get(0)?,
                        backoff: row.get(1)?,
                        ceiling: row
                            .get::<_, Option<i64>>(2)?
                            .and_then(|ceiling| u64::try_from(ceiling).ok()),
                        ceiling_changed_at: row.get(3)?,
                        last_start: row.get(4)?,
                        last_send: row.get(5)?,
                    })
                })
            })
            .optional()?
            .unwrap_or_default())
    }

    fn write(&self, connection: &Connection, key: &str) -> rusqlite::Result<()> {
        connection
            .prepare_cached(WRITE_LIMITS)
            .and_then(|mut cached| {
                cached.execute((
                    key,
                    self.ready_at,
                    self.backoff,
                    self.ceiling
                        .map(|ceiling| i64::try_from(ceiling).unwrap_or(i64::MAX)),
                    self.ceiling_changed_at,
                    self.last_start,
                    self.last_send,
                ))
            })?;
        Ok(())
    }

    /// Whether one more request may start at `now` with `in_flight` in
    /// flight, under a run's own rule, its domain's hosts asking for
    /// `crawl_delays` and its pacing `pacing`.
    fn admit(
        &self,
        in_flight: u64,
        now: i64,
        rule: &DomainRule,
        crawl_delays: impl IntoIterator<Item = Duration>,
        pacing: &Pacing,
    ) -> Verdict {
        if let Some(longest) = rule.waits.longest()
            && let Some(ready_at) = self.ready_at.filter(|ready_at| *ready_at > now)
        {
            return if ready_at.saturating_sub(now) > duration_milliseconds(longest) {
                Verdict::Refused(moment(ready_at))
            } else {
                Verdict::Until(moment(ready_at), Hold::BackOff)
            };
        }
        let delayed = rule
            .pace
            .between_starts(crawl_delays, pacing.latency)
            .zip(self.last_start)
            .map(|(delay, last_start)| last_start.saturating_add(duration_milliseconds(delay)))
            .filter(|next| *next > now);
        let rated = rule
            .pace
            .rate
            .and_then(|rate| rate.waits_until(pacing.theoretical_start, now));
        if let Some(next) = delayed.max(rated) {
            return Verdict::Until(moment(next), Hold::Delay);
        }
        let cap = match (rule.concurrency, self.ceiling) {
            (Some(own), Some(ceiling)) => Some(own.min(ceiling)),
            (own, ceiling) => own.or(ceiling),
        };
        if cap.is_some_and(|cap| in_flight >= cap) {
            return Verdict::Busy;
        }
        Verdict::Go
    }

    /// Takes the next start for a request that may go `spacing` after
    /// `after`: no sooner than `spacing` after the last start any sharing
    /// run took either, which it then becomes. Answers when it is.
    fn start_taken(&mut self, after: SystemTime, spacing: Duration) -> SystemTime {
        let after = milliseconds_from(after);
        let start = self
            .last_start
            .map_or(after, |last| last.max(after))
            .saturating_add(duration_milliseconds(spacing));
        self.last_start = Some(start);
        moment(start)
    }

    /// The state after a request ended at `now` as `ending` says, with
    /// `others` still in flight, under what a run waits for, `waits`,
    /// drawing a backoff from `entropy`.
    pub(crate) fn ended(
        mut self,
        others: u64,
        ending: Ending,
        now: i64,
        waits: &Waits,
        entropy: &mut Entropy,
    ) -> Self {
        let (backs_off, asked) = match ending {
            Ending::Done => {
                self.backoff = None;
                if let (Some(ceiling), Some(changed_at)) = (self.ceiling, self.ceiling_changed_at)
                    && now >= changed_at.saturating_add(duration_milliseconds(RECOVERY))
                {
                    self.ceiling = Some(ceiling.saturating_add(1));
                    self.ceiling_changed_at = Some(now);
                }
                return self;
            }
            Ending::BackOff(asked) => (true, asked),
            Ending::Until(asked) => (false, Some(asked)),
            Ending::TooManyRequests(asked) => {
                let base = self.ceiling.unwrap_or(others.saturating_add(1));
                self.ceiling = Some((base / 2).max(1));
                self.ceiling_changed_at = Some(now);
                (true, asked)
            }
        };
        let Some(backoff) = &waits.backoff else {
            return self;
        };
        let mut until = asked.map(milliseconds);
        if backs_off {
            let previous = self
                .backoff
                .and_then(|wait| u64::try_from(wait).ok())
                .map(Duration::from_millis);
            let wait = duration_milliseconds(backoff.next(previous, entropy));
            self.backoff = Some(wait);
            until = until.max(Some(now.saturating_add(wait)));
        }
        if let Some(until) = until {
            self.ready_at = Some(self.ready_at.map_or(until, |ready_at| ready_at.max(until)));
        }
        self
    }
}

/// One domain key's shared pacing (spec #582's Settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pacing {
    /// The weighted mean of its responses' latency, in milliseconds.
    latency: Option<i64>,
    /// Its token bucket's theoretical start (GCRA's theoretical arrival
    /// time): the bucket is full again at this moment, in milliseconds.
    theoretical_start: Option<i64>,
    /// The interval the last rated start took, in milliseconds.
    interval: Option<i64>,
    /// Its sends' token bucket's theoretical start: the moment the rated
    /// runs' sends may again go a burst at once, in milliseconds.
    theoretical_send: Option<i64>,
}

impl Pacing {
    fn read(connection: &Connection, key: &str) -> rusqlite::Result<Self> {
        Ok(connection
            .prepare_cached(READ_PACING)
            .and_then(|mut cached| {
                cached.query_row([key], |row| {
                    Ok(Self {
                        latency: row.get(0)?,
                        theoretical_start: row.get(1)?,
                        interval: row.get(2)?,
                        theoretical_send: row.get(3)?,
                    })
                })
            })
            .optional()?
            .unwrap_or_default())
    }

    fn write(&self, connection: &Connection, key: &str) -> rusqlite::Result<()> {
        connection
            .prepare_cached(WRITE_PACING)
            .and_then(|mut cached| {
                cached.execute((
                    key,
                    self.latency,
                    self.theoretical_start,
                    self.interval,
                    self.theoretical_send,
                ))
            })?;
        Ok(())
    }
}

/// The weighted mean of a key's latency after one more response of
/// `measured` milliseconds: the new one weighs 0.3, the mean before 0.7,
/// in whole milliseconds rounded to the nearest; the first is its own.
fn averaged(previous: Option<i64>, measured: i64) -> i64 {
    previous.map_or(measured, |previous| {
        let mean = (i128::from(measured) * 3 + i128::from(previous) * 7 + 5) / 10;
        i64::try_from(mean).unwrap_or(i64::MAX)
    })
}

/// The first whole millisecond at or after `wall`, as the Politeness file
/// keeps moments: a start counted from it is never sooner than from `wall`.
fn milliseconds_from(wall: SystemTime) -> i64 {
    let whole = milliseconds(wall);
    let past = wall
        .duration_since(moment(whole))
        .is_ok_and(|past| !past.is_zero());
    whole.saturating_add(i64::from(past))
}

fn duration_milliseconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// The longest a crawl under `settings` waits for its domain: the longer of
/// `politeness.retry_after.max` and `politeness.backoff.max`; `None` when
/// `politeness.backoff.enabled` is off, which waits for nothing. A site
/// that asks for a time further away gets its requests refused unsent
/// (ADR 0118).
pub fn longest_wait(settings: &Resolved) -> Option<Duration> {
    Waits::of(settings).longest()
}

/// Refuses a crawl whose `switch`, such as `fetch.hedge.enabled`, sends a
/// second copy of a request beside its first while `settings` admit one
/// request at a time to a domain, where the copy could never start (ADR
/// 0086): the one rule hedges and `fetch.tier_race` share. A crawl that
/// raises `politeness.domain.concurrency`, which is recorded as pressing
/// harder, lifts it; the second copy then takes a slot of its own, as any
/// crawl request does.
///
/// # Errors
///
/// `setting_not_applicable` naming `switch` and the layer that set it.
pub fn refuse_second_copies(settings: &Resolved, switch: &Setting) -> Result<(), CallError> {
    let Some(switched) = settings.effective(switch) else {
        return Ok(());
    };
    let one_at_a_time = settings
        .get::<Option<u64>>(&POLITENESS_DOMAIN_CONCURRENCY)
        .is_some_and(|concurrency| concurrency < 2);
    if switched.value != Value::Bool(true) || !one_at_a_time {
        return Ok(());
    }
    let name = switch.name;
    let concurrency = POLITENESS_DOMAIN_CONCURRENCY.name;
    Err(CallError::new(
        &SETTING_NOT_APPLICABLE,
        format!(
            "`{name}` sends a second copy of a request to its domain, which `{concurrency}` admits only one request at a time to: raise `{concurrency}` to 2 or more, or switch `{name}` off."
        ),
        &SettingNotApplicableDetails {
            key: name.to_owned(),
            layer: switched.layer,
            fix: format!("`{concurrency}` of 2 or more"),
        },
    ))
}

/// What a crawl waits for under its own settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Waits {
    /// `None` when `politeness.backoff.enabled` is off.
    backoff: Option<Backoff>,
    /// `politeness.retry_after.max`.
    retry_after_max: Duration,
}

impl Waits {
    pub(crate) fn of(settings: &Resolved) -> Self {
        Self {
            backoff: settings
                .get::<bool>(&POLITENESS_BACKOFF_ENABLED)
                .then(|| Backoff {
                    initial: settings.get(&POLITENESS_BACKOFF_INITIAL),
                    max: settings.get(&POLITENESS_BACKOFF_MAX),
                }),
            retry_after_max: settings.get(&POLITENESS_RETRY_AFTER_MAX),
        }
    }

    fn longest(&self) -> Option<Duration> {
        let backoff = self.backoff?;
        Some(self.retry_after_max.max(backoff.max()))
    }

    /// The backoff it draws, `None` while `politeness.backoff.enabled` is
    /// off.
    pub(crate) fn backoff(&self) -> Option<Backoff> {
        self.backoff
    }
}

impl Default for Waits {
    /// What a crawl waits for under the catalog's defaults: a backoff from
    /// 2 s up to 60 s, and a `Retry-After` up to 15 minutes.
    fn default() -> Self {
        Self {
            backoff: catalog_default::<bool>(&POLITENESS_BACKOFF_ENABLED).then(|| Backoff {
                initial: catalog_default(&POLITENESS_BACKOFF_INITIAL),
                max: catalog_default(&POLITENESS_BACKOFF_MAX),
            }),
            retry_after_max: catalog_default(&POLITENESS_RETRY_AFTER_MAX),
        }
    }
}

/// Whether a crawl under `settings` paces its domain keys: it caps them
/// with `politeness.domain.concurrency`, or spaces or rates their starts
/// ([`Pace`]). One that does not consults no Politeness store for them
/// (ADR 0138).
pub fn paces(settings: &Resolved) -> bool {
    settings
        .get::<Option<u64>>(&POLITENESS_DOMAIN_CONCURRENCY)
        .is_some()
        || Pace::of(settings).paces()
}

/// A crawl's per-domain rule under its own settings.
#[derive(Clone, Debug)]
pub(crate) struct DomainRule {
    concurrency: Option<u64>,
    /// Its delay between starts and its rate.
    pub(crate) pace: Pace,
    waits: Waits,
    entropy: SharedEntropy,
}

impl DomainRule {
    /// The rule `settings` give a crawl, drawing its backoffs from
    /// `entropy`.
    pub(crate) fn of(settings: &Resolved, entropy: SharedEntropy) -> Self {
        Self {
            concurrency: settings.get(&POLITENESS_DOMAIN_CONCURRENCY),
            pace: Pace::of(settings),
            waits: Waits::of(settings),
            entropy,
        }
    }
}

/// The parts of a domain key's state a change reads; the others load as if
/// never written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Parts {
    limits: bool,
    pacing: bool,
    crawl_delays: bool,
}

impl Parts {
    /// Nothing.
    pub(crate) const NONE: Self = Self {
        limits: false,
        pacing: false,
        crawl_delays: false,
    };
    /// Its limits alone.
    pub(crate) const LIMITS: Self = Self {
        limits: true,
        ..Self::NONE
    };
    /// Its pacing alone.
    pub(crate) const PACING: Self = Self {
        pacing: true,
        ..Self::NONE
    };
    /// Its limits and its pacing.
    pub(crate) const PACED: Self = Self {
        limits: true,
        pacing: true,
        ..Self::NONE
    };
    /// All of it.
    pub(crate) const ALL: Self = Self {
        limits: true,
        pacing: true,
        crawl_delays: true,
    };
}

/// The parts of a domain key's state a change wrote, the only ones saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Wrote {
    limits: bool,
    pacing: bool,
    /// The location whose Crawl-delay it asked for or dropped.
    crawl_delay: Option<String>,
}

impl Wrote {
    /// Its limits.
    const LIMITS: Self = Self {
        limits: true,
        pacing: false,
        crawl_delay: None,
    };
    /// Its pacing.
    const PACING: Self = Self {
        limits: false,
        pacing: true,
        crawl_delay: None,
    };

    /// What it and `other` wrote.
    fn and(self, other: Self) -> Self {
        Self {
            limits: self.limits || other.limits,
            pacing: self.pacing || other.pacing,
            crawl_delay: self.crawl_delay.or(other.crawl_delay),
        }
    }
}

/// One domain key's shared state, as plain values: its limits, its pacing
/// and the Crawl-delay the robots.txt at each of its hosts' locations asks
/// for. The Politeness file loads and saves it ([`KeyState`]); the rules
/// change it in memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DomainState {
    limits: Limits,
    pacing: Pacing,
    crawl_delays: BTreeMap<String, Duration>,
}

/// The domain keys' states a Run's own Politeness keeps in memory, by key.
#[derive(Default)]
pub(crate) struct Domains(HashMap<String, DomainState>);

impl Domains {
    /// The state kept under `key`, or none yet.
    fn of<'a>(tables: &'a MemoryTables, key: &str) -> Option<&'a DomainState> {
        tables.get::<Self>().and_then(|domains| domains.0.get(key))
    }
}

impl KeyState for DomainState {
    type Key = str;
    type Reads = Parts;
    type Writes = Wrote;

    fn load(connection: &Connection, key: &str, reads: Parts) -> rusqlite::Result<Self> {
        Ok(Self {
            limits: if reads.limits {
                Limits::read(connection, key)?
            } else {
                Limits::default()
            },
            pacing: if reads.pacing {
                Pacing::read(connection, key)?
            } else {
                Pacing::default()
            },
            crawl_delays: if reads.crawl_delays {
                crawl_delays(connection, key)?
            } else {
                BTreeMap::new()
            },
        })
    }

    fn save(&self, connection: &Connection, key: &str, wrote: &Wrote) -> rusqlite::Result<()> {
        if wrote.limits {
            self.limits.write(connection, key)?;
        }
        if wrote.pacing {
            self.pacing.write(connection, key)?;
        }
        if let Some(location) = &wrote.crawl_delay {
            match self.crawl_delays.get(location) {
                Some(delay) => {
                    connection
                        .prepare_cached(ASK_CRAWL_DELAY)
                        .and_then(|mut cached| {
                            cached.execute((key, location, duration_milliseconds(*delay)))
                        })?
                }
                None => connection
                    .prepare_cached(DROP_CRAWL_DELAY)
                    .and_then(|mut cached| cached.execute((key, location)))?,
            };
        }
        Ok(())
    }

    fn load_from(tables: &MemoryTables, key: &str, reads: Parts) -> Self {
        let Some(kept) = Domains::of(tables, key) else {
            return Self::default();
        };
        Self {
            limits: if reads.limits {
                kept.limits
            } else {
                Limits::default()
            },
            pacing: if reads.pacing {
                kept.pacing
            } else {
                Pacing::default()
            },
            crawl_delays: if reads.crawl_delays {
                kept.crawl_delays.clone()
            } else {
                BTreeMap::new()
            },
        }
    }

    fn save_to(&self, tables: &mut MemoryTables, key: &str, wrote: &Wrote) {
        let kept = tables
            .get_mut::<Domains>()
            .0
            .entry(key.to_owned())
            .or_default();
        if wrote.limits {
            kept.limits = self.limits;
        }
        if wrote.pacing {
            kept.pacing = self.pacing;
        }
        if let Some(location) = &wrote.crawl_delay {
            match self.crawl_delays.get(location) {
                Some(delay) => kept.crawl_delays.insert(location.clone(), *delay),
                None => kept.crawl_delays.remove(location),
            };
        }
    }
}

impl DomainState {
    /// The Crawl-delays the robots.txt of its hosts ask for.
    fn asked(&self) -> impl Iterator<Item = Duration> + '_ {
        self.crawl_delays.values().copied()
    }

    /// The parts [`send_taken`](Self::send_taken) reads for a run of
    /// `pace`.
    pub(crate) fn send_reads(pace: Option<&Pace>) -> Parts {
        if pace.is_some() {
            Parts::ALL
        } else {
            Parts::PACED
        }
    }

    /// Takes the start of a request sent at `now`, a crawl request's own by
    /// a run of `pace`; when the key's last send or sends leave that run's
    /// delay or rate unmet, it takes nothing and answers when to ask again.
    /// The send is checked at the whole millisecond it falls in and
    /// recorded at the next one: the last send is kept rounded up, so a
    /// check rounded up as well would let a send go up to 1 ms before the
    /// delay or the interval after it.
    pub(crate) fn send_taken(
        &mut self,
        now: SystemTime,
        pace: Option<Pace>,
    ) -> (Option<SystemTime>, Wrote) {
        let checked = milliseconds(now);
        let now = milliseconds_from(now);
        let mut wrote = Wrote::default();
        if let Some(pace) = pace {
            if let Some(until) =
                pace.send_waits_until(&self.limits, &self.pacing, self.asked(), checked)
            {
                return (Some(moment(until)), wrote);
            }
            if let Some(rate) = pace.rate {
                self.pacing.theoretical_send = Some(rate.taken(self.pacing.theoretical_send, now));
                wrote = Wrote::PACING;
            }
        }
        self.limits.last_send = Some(self.limits.last_send.map_or(now, |last| last.max(now)));
        (None, wrote.and(self.start_noted(moment(now))))
    }

    /// Records a second start at `now`: the later of it and the last start
    /// becomes the last, and the rate's token moves to it
    /// ([`start_moved`](Self::start_moved)).
    pub(crate) fn start_noted(&mut self, now: SystemTime) -> Wrote {
        let at = milliseconds(now);
        self.limits.last_start = Some(self.limits.last_start.map_or(at, |last| last.max(at)));
        Wrote::LIMITS.and(self.start_moved(now))
    }

    /// Takes the next start for a request that may go `spacing` after
    /// `after`: no sooner than `spacing` after the last start any sharing
    /// run took either, which it then becomes. Answers when it is.
    pub(crate) fn start_taken(
        &mut self,
        after: SystemTime,
        spacing: Duration,
    ) -> (SystemTime, Wrote) {
        (self.limits.start_taken(after, spacing), Wrote::LIMITS)
    }

    /// Records that the robots.txt at `location` asks for the Crawl-delay
    /// `delay`, or for none, replacing what it asked before.
    pub(crate) fn crawl_delay_asked(&mut self, location: String, delay: Option<Duration>) -> Wrote {
        match delay {
            Some(delay) => self.crawl_delays.insert(location.clone(), delay),
            None => self.crawl_delays.remove(&location),
        };
        Wrote {
            crawl_delay: Some(location),
            ..Wrote::default()
        }
    }

    /// Records that a response arrived `latency` after its request was
    /// sent, into the weighted mean of its latency.
    pub(crate) fn latency_noted(&mut self, latency: Duration) -> Wrote {
        self.pacing.latency = Some(averaged(
            self.pacing.latency,
            duration_milliseconds(latency),
        ));
        Wrote::PACING
    }

    /// Moves the token a request sent at `sent` took to its send: while the
    /// bucket is still refilling past `sent`, it is full again no sooner
    /// than one interval after the send, so a start whose send came after
    /// its admission never lets the next come sooner than the rate (ADR
    /// 0086's starts are sends). It only ever moves the bucket later.
    pub(crate) fn start_moved(&mut self, sent: SystemTime) -> Wrote {
        let sent = milliseconds_from(sent);
        let (Some(theoretical), Some(interval)) =
            (self.pacing.theoretical_start, self.pacing.interval)
        else {
            return Wrote::default();
        };
        if theoretical <= sent {
            return Wrote::default();
        }
        self.pacing.theoretical_start = Some(theoretical.max(sent.saturating_add(interval)));
        Wrote::PACING
    }
}

/// The moment a document after a robots.txt fetched at `after` counts its
/// start from, while that fetch is less than `spacing` before `now`: the
/// document may have been admitted before its Crawl-delay was known.
/// `None` when its admission kept the delay.
fn document_waits(
    after: Option<SystemTime>,
    spacing: Duration,
    now: SystemTime,
) -> Option<SystemTime> {
    after.filter(|after| *after + spacing > now)
}

/// The fetch of the robots.txt copy at a location, in milliseconds, and
/// its domain key's limits: what a document's start after that robots.txt
/// reads ([`DocumentStart::taken`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DocumentStart {
    /// When the copy was fetched, `None` without one or when the attempt
    /// fetched it itself.
    copy_fetched_at: Option<i64>,
    limits: Limits,
}

/// What a document's start after a robots.txt reads: `fetched` when its
/// attempt fetched it, and `spacing` and `now`, which decide whether it
/// reads the key's limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DocumentReads {
    fetched: Option<SystemTime>,
    spacing: Duration,
    now: SystemTime,
}

impl DocumentStart {
    /// The moment its document counts from: `fetched`, else the copy's
    /// fetch. The copy keeps the millisecond its fetch ended in: it ended
    /// before the next.
    fn after(fetched: Option<SystemTime>, copy_fetched_at: Option<i64>) -> Option<SystemTime> {
        fetched.or_else(|| copy_fetched_at.map(|at| moment(at.saturating_add(1))))
    }

    /// Takes the next start for a document whose attempt read the
    /// robots.txt as `reads` says: while that robots.txt's fetch is less
    /// than `spacing` before `now`, no sooner than `spacing` after that
    /// fetch and after the last start any sharing run took
    /// ([`Limits::start_taken`]); answers when it is. Otherwise its
    /// admission kept the delay: `None`.
    pub(crate) fn taken(&mut self, reads: DocumentReads) -> (Option<SystemTime>, bool) {
        let Some(after) = document_waits(
            Self::after(reads.fetched, self.copy_fetched_at),
            reads.spacing,
            reads.now,
        ) else {
            return (None, false);
        };
        (Some(self.limits.start_taken(after, reads.spacing)), true)
    }
}

impl KeyState for DocumentStart {
    /// The domain key, then the robots.txt's location.
    type Key = (String, String);
    type Reads = DocumentReads;
    /// Whether it took a start.
    type Writes = bool;

    fn load(
        connection: &Connection,
        (key, location): &(String, String),
        reads: DocumentReads,
    ) -> rusqlite::Result<Self> {
        let copy_fetched_at = match reads.fetched {
            Some(_) => None,
            None => robots_txt::stored(connection, location)?.map(|copy| copy.fetched_at),
        };
        let limits = match document_waits(
            Self::after(reads.fetched, copy_fetched_at),
            reads.spacing,
            reads.now,
        ) {
            Some(_) => Limits::read(connection, key)?,
            None => Limits::default(),
        };
        Ok(Self {
            copy_fetched_at,
            limits,
        })
    }

    fn save(
        &self,
        connection: &Connection,
        (key, _): &(String, String),
        took: &bool,
    ) -> rusqlite::Result<()> {
        if *took {
            self.limits.write(connection, key)?;
        }
        Ok(())
    }

    fn load_from(
        tables: &MemoryTables,
        (key, location): &(String, String),
        reads: DocumentReads,
    ) -> Self {
        let copy_fetched_at = match reads.fetched {
            Some(_) => None,
            None => robots_txt::kept_in(tables, location).map(|copy| copy.fetched_at),
        };
        let limits = match document_waits(
            Self::after(reads.fetched, copy_fetched_at),
            reads.spacing,
            reads.now,
        ) {
            Some(_) => Domains::of(tables, key)
                .map(|kept| kept.limits)
                .unwrap_or_default(),
            None => Limits::default(),
        };
        Self {
            copy_fetched_at,
            limits,
        }
    }

    fn save_to(&self, tables: &mut MemoryTables, (key, _): &(String, String), took: &bool) {
        if *took {
            tables
                .get_mut::<Domains>()
                .0
                .entry(key.clone())
                .or_default()
                .limits = self.limits;
        }
    }
}

impl PermitRule for DomainRule {
    type State = DomainState;

    fn paces(&self) -> bool {
        self.concurrency.is_some() || self.pace.paces()
    }

    fn reads(&self, step: Step) -> Parts {
        match step {
            Step::Take => Parts::ALL,
            Step::GiveBack => Parts::LIMITS,
        }
    }

    fn admit(&self, state: &DomainState, in_flight: u64, now: SystemTime) -> Verdict {
        state.limits.admit(
            in_flight,
            milliseconds(now),
            self,
            state.asked(),
            &state.pacing,
        )
    }

    fn started(&self, state: &mut DomainState, now: SystemTime) -> Wrote {
        let now = milliseconds(now);
        state.limits.last_start = Some(now);
        let Some(rate) = self.pace.rate else {
            return Wrote::LIMITS;
        };
        state.pacing.theoretical_start = Some(rate.taken(state.pacing.theoretical_start, now));
        state.pacing.interval = Some(rate.interval);
        Wrote::LIMITS.and(Wrote::PACING)
    }

    fn ended(
        &self,
        state: &mut DomainState,
        in_flight: u64,
        ending: Ending,
        now: SystemTime,
    ) -> (Option<SystemTime>, Wrote) {
        let now = milliseconds(now);
        state.limits = {
            let mut entropy = self.entropy.lock().unwrap_or_else(PoisonError::into_inner);
            state
                .limits
                .ended(in_flight, ending, now, &self.waits, &mut entropy)
        };
        let longest = self.waits.longest().map(duration_milliseconds);
        let ready_at = state
            .limits
            .ready_at
            .filter(|ready_at| {
                *ready_at > now
                    && longest.is_some_and(|longest| ready_at.saturating_sub(now) <= longest)
            })
            .map(moment);
        (ready_at, Wrote::LIMITS)
    }
}

/// A second request under the Permit a request of `key` holds started at
/// `now`, such as its document after its origin's robots.txt: every run
/// sharing `politeness` counts its delay from it (ADR 0086).
///
/// # Errors
///
/// When the Politeness file's transaction failed.
///
/// # Cancel safety
///
/// Dropping the future does not withdraw a transaction already queued.
pub async fn started_again(
    politeness: &Politeness,
    key: &str,
    now: SystemTime,
) -> Result<(), PolitenessError> {
    politeness
        .change(
            key.to_owned(),
            Parts::PACED,
            move |state: &mut DomainState| ((), state.start_noted(now)),
        )
        .await
}

/// A request of `key` sent now: every run sharing `politeness` counts its
/// delay and its rate's interval from the moment `clock` reads once the
/// file records the send, so a send that waited for the file, which
/// another engine or run held, never counts from before that wait (ADR
/// 0086: a start is its send, the later of the two instants). A crawl
/// request's own send, by a run of `pace`, also waits for its delay after
/// the key's last send by any sharing run, the delay as the key's latency
/// lengthened it meanwhile, and takes a token of the key's sends under its
/// rate, every sharing rated run's together: when either holds it, it
/// records nothing and answers when to ask again, which the caller waits
/// for, so its start is taken when the request goes. An admission that
/// counted from an earlier start therefore never lets a request reach the
/// site within the delay or the interval of one whose send came later than
/// its own admission. Hops and a handler's navigations pass `None`: they
/// wait for nothing here, and their sends still count.
///
/// # Errors
///
/// When the Politeness file's transaction failed.
///
/// # Cancel safety
///
/// Dropping the future does not withdraw a transaction already queued.
pub async fn started_at_send(
    politeness: &Politeness,
    key: &str,
    clock: Clock,
    pace: Option<Pace>,
) -> Result<Option<SystemTime>, PolitenessError> {
    politeness
        .change(
            key.to_owned(),
            DomainState::send_reads(pace.as_ref()),
            move |state: &mut DomainState| state.send_taken(clock.wall(), pace),
        )
        .await
}

/// Takes the next start of `key`, for every run sharing `politeness`, for
/// a request that may go `spacing` after `after`, such as a handler's
/// navigation after the last start of its own key: no sooner than
/// `spacing` after the last start any sharing run took either (ADR 0086).
/// Answers when it is; the caller waits until then.
///
/// # Errors
///
/// When the Politeness file's transaction failed.
///
/// # Cancel safety
///
/// Dropping the future does not withdraw a transaction already queued.
pub async fn start_after(
    politeness: &Politeness,
    key: &str,
    after: SystemTime,
    spacing: Duration,
) -> Result<SystemTime, PolitenessError> {
    politeness
        .change(
            key.to_owned(),
            Parts::LIMITS,
            move |state: &mut DomainState| state.start_taken(after, spacing),
        )
        .await
}

/// Takes the next start of `key`, for every run sharing `politeness`, for
/// a document whose attempt read the robots.txt at `location` at `now`
/// ([`DocumentStart::taken`]): `spacing` after that robots.txt's fetch,
/// `fetched` when the attempt fetched it, while that fetch is less than
/// `spacing` before, and after the last start any sharing run took. So two
/// runs that read robots.txt at once, before either knew its Crawl-delay,
/// start their documents `spacing` apart, and the site receives neither
/// sooner than `spacing` after the robots.txt request (ADR 0086). Answers
/// when it is, for the caller to wait until then; `None` when the
/// document's admission kept the delay.
///
/// # Errors
///
/// When the Politeness file's transaction failed.
///
/// # Cancel safety
///
/// Dropping the future does not withdraw a transaction already queued.
pub async fn start_after_robots_txt(
    politeness: &Politeness,
    key: &str,
    location: &str,
    fetched: Option<SystemTime>,
    spacing: Duration,
    now: SystemTime,
) -> Result<Option<SystemTime>, PolitenessError> {
    let reads = DocumentReads {
        fetched,
        spacing,
        now,
    };
    politeness
        .change(
            (key.to_owned(), location.to_owned()),
            reads,
            move |state: &mut DocumentStart| state.taken(reads),
        )
        .await
}

/// When the last request of `key` started, by any run sharing
/// `politeness`; `None` before the first.
///
/// # Errors
///
/// When the Politeness file's transaction failed.
///
/// # Cancel safety
///
/// Dropping the future only loses the reading.
pub async fn last_start(
    politeness: &Politeness,
    key: &str,
) -> Result<Option<SystemTime>, PolitenessError> {
    let last = politeness
        .change(key.to_owned(), Parts::LIMITS, |state: &mut DomainState| {
            (state.limits.last_start, Wrote::default())
        })
        .await?;
    Ok(last
        .and_then(|last| u64::try_from(last).ok())
        .map(|last| SystemTime::UNIX_EPOCH + Duration::from_millis(last)))
}

/// The Crawl-delay the robots.txt at each location of `key`'s hosts asks
/// for.
fn crawl_delays(
    connection: &Connection,
    key: &str,
) -> rusqlite::Result<BTreeMap<String, Duration>> {
    let mut statement = connection.prepare_cached(CRAWL_DELAYS)?;
    statement
        .query_map([key], |row| {
            let delay = row.get::<_, i64>(1)?;
            Ok((
                row.get::<_, String>(0)?,
                Duration::from_millis(u64::try_from(delay).unwrap_or(0)),
            ))
        })?
        .collect()
}

/// Records, for every run sharing `politeness`, that the robots.txt at
/// `location` asks the requests of `key` for the Crawl-delay `delay`, or
/// for none (ADR 0086): each such run then waits it between starts on
/// `key`, up to its own `site_preferences.max_crawl_delay`.
///
/// # Errors
///
/// When the Politeness file's transaction failed.
///
/// # Cancel safety
///
/// Dropping the future does not withdraw a transaction already queued.
pub async fn crawl_delay_read(
    politeness: &Politeness,
    key: &str,
    location: &str,
    delay: Option<Duration>,
) -> Result<(), PolitenessError> {
    let location = location.to_owned();
    politeness
        .change(
            key.to_owned(),
            Parts::NONE,
            move |state: &mut DomainState| ((), state.crawl_delay_asked(location, delay)),
        )
        .await
}

/// Records, for every run sharing `politeness`, that a response of `key`
/// arrived `latency` after its request was sent: each run with
/// `politeness.domain.adaptive_delay` on then waits at least the weighted
/// mean of that latency between starts on `key`, up to its own
/// `politeness.backoff.max`.
///
/// # Errors
///
/// When the Politeness file's transaction failed.
///
/// # Cancel safety
///
/// Dropping the future does not withdraw a transaction already queued.
pub async fn latency_measured(
    politeness: &Politeness,
    key: &str,
    latency: Duration,
) -> Result<(), PolitenessError> {
    politeness
        .change(
            key.to_owned(),
            Parts::PACING,
            move |state: &mut DomainState| ((), state.latency_noted(latency)),
        )
        .await
}

#[cfg(test)]
mod tests;
