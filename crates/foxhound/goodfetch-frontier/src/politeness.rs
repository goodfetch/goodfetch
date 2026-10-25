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
//! run's rated starts draw on, holds a token ([`Rate`], a GCRA of
//! `politeness.domain.burst` tokens); a start counts from its request's
//! send when that is later than its admission, and a request's own send
//! also waits for a token of the key's sends, so a send that came later
//! than the next request's admission is never overtaken within the
//! interval.
//!
//! The per-domain state lives in the `domain_limits`, `crawl_delays` and
//! `domain_pacing` tables Foxhound adds to the file; a Collie engine adds
//! none and touches none (ADR 0098).

#![expect(
    clippy::disallowed_types,
    reason = "the rule runs inside the Politeness file's transaction, a blocking call that may run inline on a runtime thread: one Mutex guards a gate's Entropy for one draw, never held with another lock or across an await, so there is no lock order to test under loom"
)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::politeness::{
    Ending, Hold, PermitRule, Politeness, PolitenessError, PolitenessTables, Verdict, milliseconds,
    moment,
};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_BACKOFF_ENABLED, POLITENESS_BACKOFF_INITIAL, POLITENESS_BACKOFF_MAX,
    POLITENESS_DOMAIN_ADAPTIVE_DELAY, POLITENESS_DOMAIN_BURST, POLITENESS_DOMAIN_CONCURRENCY,
    POLITENESS_DOMAIN_DELAY, POLITENESS_DOMAIN_RATE, POLITENESS_RETRY_AFTER_MAX,
    SITE_PREFERENCES_MAX_CRAWL_DELAY, SITE_PREFERENCES_RESPECT_ROBOTS_TXT,
};
use goodfetch_protocol::settings::{SETTING_NOT_APPLICABLE, SettingNotApplicableDetails};
use rusqlite::{Connection, OptionalExtension as _};

use crate::robots_txt;
use serde_json::Value;

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
            last_start INTEGER
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
};

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

/// One domain key's shared state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Limits {
    /// No request before this moment, in milliseconds.
    ready_at: Option<i64>,
    /// The last wait of the backoffs in a row, in milliseconds; `None`
    /// once a request succeeded.
    backoff: Option<i64>,
    /// The most requests in flight a 429 left it.
    ceiling: Option<u64>,
    /// When the ceiling last changed, in milliseconds.
    ceiling_changed_at: Option<i64>,
    /// The last request start by any sharing run, in milliseconds.
    last_start: Option<i64>,
}

impl Limits {
    fn read(connection: &Connection, key: &str) -> rusqlite::Result<Self> {
        Ok(connection
            .query_row(
                "SELECT ready_at, backoff, ceiling, ceiling_changed_at, last_start
                 FROM domain_limits WHERE key = ?1",
                [key],
                |row| {
                    Ok(Self {
                        ready_at: row.get(0)?,
                        backoff: row.get(1)?,
                        ceiling: row
                            .get::<_, Option<i64>>(2)?
                            .and_then(|ceiling| u64::try_from(ceiling).ok()),
                        ceiling_changed_at: row.get(3)?,
                        last_start: row.get(4)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    fn write(&self, connection: &Connection, key: &str) -> rusqlite::Result<()> {
        connection.execute(
            "INSERT INTO domain_limits (key, ready_at, backoff, ceiling, ceiling_changed_at, last_start)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (key) DO UPDATE SET ready_at = ?2, backoff = ?3, ceiling = ?4,
                 ceiling_changed_at = ?5, last_start = ?6",
            (
                key,
                self.ready_at,
                self.backoff,
                self.ceiling
                    .map(|ceiling| i64::try_from(ceiling).unwrap_or(i64::MAX)),
                self.ceiling_changed_at,
                self.last_start,
            ),
        )?;
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
        crawl_delays: &[Duration],
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
            .between_starts(crawl_delays, pacing.latency)
            .zip(self.last_start)
            .map(|(delay, last_start)| last_start.saturating_add(duration_milliseconds(delay)))
            .filter(|next| *next > now);
        let rated = rule
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

    /// The state after a request ended at `now` as `ending` says, with
    /// `others` still in flight, under a run's own rule, drawing a backoff
    /// from `entropy`.
    fn ended(
        mut self,
        others: u64,
        ending: Ending,
        now: i64,
        rule: &DomainRule,
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
        let Some(backoff) = &rule.waits.backoff else {
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
            .query_row(
                "SELECT latency, theoretical_start, interval, theoretical_send
                 FROM domain_pacing WHERE key = ?1",
                [key],
                |row| {
                    Ok(Self {
                        latency: row.get(0)?,
                        theoretical_start: row.get(1)?,
                        interval: row.get(2)?,
                        theoretical_send: row.get(3)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    fn write(&self, connection: &Connection, key: &str) -> rusqlite::Result<()> {
        connection.execute(
            "INSERT INTO domain_pacing (key, latency, theoretical_start, interval, theoretical_send)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (key) DO UPDATE SET latency = ?2, theoretical_start = ?3, interval = ?4,
                 theoretical_send = ?5",
            (
                key,
                self.latency,
                self.theoretical_start,
                self.interval,
                self.theoretical_send,
            ),
        )?;
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

/// A run's per-domain rate: a token bucket of `burst` tokens, one coming
/// back every `interval`, as GCRA keeps it, one theoretical start per key
/// for the admissions and one for the sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    /// A minute divided by `politeness.domain.rate`, rounded up to the
    /// millisecond so the rate is never exceeded.
    interval: i64,
    /// `politeness.domain.burst`, at least 1.
    burst: u64,
}

impl Rate {
    /// The rate `settings` give a crawl; `None` without
    /// `politeness.domain.rate`, or with one of 0.
    pub fn of(settings: &Resolved) -> Option<Self> {
        Self::per_minute(
            settings.count(POLITENESS_DOMAIN_RATE.name)?,
            settings.count(POLITENESS_DOMAIN_BURST.name).unwrap_or(1),
        )
    }

    /// `rate` requests a minute in bursts of `burst`, a burst of 0 read as
    /// 1; `None` for a rate of 0.
    pub fn per_minute(rate: u64, burst: u64) -> Option<Self> {
        (rate > 0).then(|| Self {
            interval: i64::try_from(60_000_u64.div_ceil(rate)).unwrap_or(i64::MAX),
            burst: burst.max(1),
        })
    }

    /// The moment a start at `now` must wait for, when the bucket whose
    /// theoretical start is `theoretical` holds no token; `None` when one
    /// is there.
    fn waits_until(&self, theoretical: Option<i64>, now: i64) -> Option<i64> {
        let tolerance = self
            .interval
            .saturating_mul(i64::try_from(self.burst - 1).unwrap_or(i64::MAX));
        theoretical
            .map(|theoretical| theoretical.saturating_sub(tolerance))
            .filter(|earliest| *earliest > now)
    }

    /// The theoretical start after a start at `now` took a token.
    fn taken(&self, theoretical: Option<i64>, now: i64) -> i64 {
        theoretical
            .map_or(now, |theoretical| theoretical.max(now))
            .saturating_add(self.interval)
    }
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
    let Some(switched) = settings.get(switch.name) else {
        return Ok(());
    };
    let one_at_a_time = settings
        .count(POLITENESS_DOMAIN_CONCURRENCY.name)
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
struct Waits {
    /// `None` when `politeness.backoff.enabled` is off.
    backoff: Option<Backoff>,
    /// `politeness.retry_after.max`.
    retry_after_max: Duration,
}

impl Waits {
    fn of(settings: &Resolved) -> Self {
        let duration = |name| settings.duration(name).unwrap_or_default();
        Self {
            backoff: (settings.value(POLITENESS_BACKOFF_ENABLED.name).as_bool() != Some(false))
                .then(|| Backoff {
                    initial: duration(POLITENESS_BACKOFF_INITIAL.name),
                    max: duration(POLITENESS_BACKOFF_MAX.name),
                }),
            retry_after_max: duration(POLITENESS_RETRY_AFTER_MAX.name),
        }
    }

    fn longest(&self) -> Option<Duration> {
        let backoff = self.backoff?;
        Some(self.retry_after_max.max(backoff.max()))
    }
}

/// A crawl's per-domain rule under its own settings.
#[derive(Clone, Debug)]
pub(crate) struct DomainRule {
    concurrency: Option<u64>,
    delay: Option<Duration>,
    /// `site_preferences.max_crawl_delay`; `None` while
    /// `site_preferences.respect_robots_txt` is off, which waits for no
    /// Crawl-delay.
    max_crawl_delay: Option<Duration>,
    /// `politeness.backoff.max`, which caps the latency's mean, while
    /// `politeness.domain.adaptive_delay` is on; `None` while it is off.
    adaptive_max: Option<Duration>,
    /// `politeness.domain.rate` and `politeness.domain.burst`.
    rate: Option<Rate>,
    waits: Waits,
    entropy: SharedEntropy,
}

impl DomainRule {
    /// The rule `settings` give a crawl, drawing its backoffs from
    /// `entropy`.
    pub(crate) fn of(settings: &Resolved, entropy: SharedEntropy) -> Self {
        Self {
            concurrency: settings.count(POLITENESS_DOMAIN_CONCURRENCY.name),
            delay: settings
                .duration(POLITENESS_DOMAIN_DELAY.name)
                .filter(|delay| !delay.is_zero()),
            max_crawl_delay: (settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name)
                != Some(false))
            .then(|| settings.duration(SITE_PREFERENCES_MAX_CRAWL_DELAY.name))
            .flatten(),
            adaptive_max: (settings.boolean(POLITENESS_DOMAIN_ADAPTIVE_DELAY.name) == Some(true))
                .then(|| {
                    settings
                        .duration(POLITENESS_BACKOFF_MAX.name)
                        .unwrap_or_default()
                }),
            rate: Rate::of(settings),
            waits: Waits::of(settings),
            entropy,
        }
    }

    /// The delay between starts on a domain key whose hosts ask for
    /// `crawl_delays` (ADR 0086) and whose responses' latency averages
    /// `latency` milliseconds: the longest of `politeness.domain.delay`,
    /// each Crawl-delay up to `site_preferences.max_crawl_delay`, so a
    /// shorter one never shortens it and a longer one never raises it, and,
    /// with the adaptive delay on, that latency up to
    /// `politeness.backoff.max`; `None` for no wait.
    fn between_starts(&self, crawl_delays: &[Duration], latency: Option<i64>) -> Option<Duration> {
        let honoured = self.max_crawl_delay.and_then(|max| {
            crawl_delays
                .iter()
                .copied()
                .filter(|asked| *asked <= max)
                .max()
        });
        let adaptive = self.adaptive_max.zip(latency).map(|(max, latency)| {
            Duration::from_millis(u64::try_from(latency).unwrap_or_default()).min(max)
        });
        self.delay
            .max(honoured)
            .max(adaptive)
            .filter(|delay| !delay.is_zero())
    }
}

impl PermitRule for DomainRule {
    fn admit(
        &self,
        connection: &Connection,
        key: &str,
        in_flight: u64,
        now: SystemTime,
    ) -> rusqlite::Result<Verdict> {
        let crawl_delays = crawl_delays(connection, key)?;
        let pacing = Pacing::read(connection, key)?;
        Ok(Limits::read(connection, key)?.admit(
            in_flight,
            milliseconds(now),
            self,
            &crawl_delays,
            &pacing,
        ))
    }

    fn started(&self, connection: &Connection, key: &str, now: SystemTime) -> rusqlite::Result<()> {
        let mut limits = Limits::read(connection, key)?;
        let now = milliseconds(now);
        limits.last_start = Some(now);
        limits.write(connection, key)?;
        if let Some(rate) = self.rate {
            let mut pacing = Pacing::read(connection, key)?;
            pacing.theoretical_start = Some(rate.taken(pacing.theoretical_start, now));
            pacing.interval = Some(rate.interval);
            pacing.write(connection, key)?;
        }
        Ok(())
    }

    fn ended(
        &self,
        connection: &Connection,
        key: &str,
        in_flight: u64,
        ending: Ending,
        now: SystemTime,
    ) -> rusqlite::Result<Option<SystemTime>> {
        let now = milliseconds(now);
        let limits = {
            let mut entropy = self.entropy.lock().unwrap_or_else(PoisonError::into_inner);
            Limits::read(connection, key)?.ended(in_flight, ending, now, self, &mut entropy)
        };
        limits.write(connection, key)?;
        let longest = self.waits.longest().map(duration_milliseconds);
        Ok(limits
            .ready_at
            .filter(|ready_at| {
                *ready_at > now
                    && longest.is_some_and(|longest| ready_at.saturating_sub(now) <= longest)
            })
            .map(moment))
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
    let key = key.to_owned();
    politeness
        .transact(move |connection| start_noted(connection, &key, now))
        .await
}

/// A request of `key` sent now: every run sharing `politeness` counts its
/// delay and its rate's interval from the moment `clock` reads once the
/// file records the send, so a send that waited for the file, which
/// another engine or run held, never counts from before that wait (ADR
/// 0086: a start is its send, the later of the two instants). A crawl
/// request's own send by a run of `rate` also takes a token of the key's
/// sends, every sharing rated run's together: when the sends before it
/// left none, it records nothing and answers the moment the next comes
/// back, which the caller waits for before asking again, so the token and
/// the start are taken when the request goes. An admission's token therefore
/// never lets a request reach the site within the interval of one whose
/// send came later than its own admission. Hops and runs with no rate
/// pass `None`.
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
    rate: Option<Rate>,
) -> Result<Option<SystemTime>, PolitenessError> {
    let key = key.to_owned();
    politeness
        .transact(move |connection| send_taken(connection, &key, clock.wall(), rate))
        .await
}

/// Takes in `connection` the start of a request of `key` sent at `now` by
/// a run of `rate`; when the key's sends left that run no token, it takes
/// nothing and answers when to ask again.
pub(crate) fn send_taken(
    connection: &Connection,
    key: &str,
    now: SystemTime,
    rate: Option<Rate>,
) -> rusqlite::Result<Option<SystemTime>> {
    let now = milliseconds_from(now);
    if let Some(rate) = rate {
        let mut pacing = Pacing::read(connection, key)?;
        if let Some(until) = rate.waits_until(pacing.theoretical_send, now) {
            return Ok(Some(moment(until)));
        }
        pacing.theoretical_send = Some(rate.taken(pacing.theoretical_send, now));
        pacing.write(connection, key)?;
    }
    start_noted(connection, key, moment(now))?;
    Ok(None)
}

/// Records in `connection` a second start of `key` at `now`: the later of
/// it and the last start becomes the last, and the rate's token moves to
/// it ([`start_moved`]).
fn start_noted(connection: &Connection, key: &str, now: SystemTime) -> rusqlite::Result<()> {
    let mut limits = Limits::read(connection, key)?;
    let at = milliseconds(now);
    limits.last_start = Some(limits.last_start.map_or(at, |last| last.max(at)));
    limits.write(connection, key)?;
    start_moved(connection, key, now)
}

/// Takes in `connection` the next start of `key` for a request that may go
/// `spacing` after `after`: no sooner than `spacing` after the last start
/// any sharing run took either, which it then becomes. Answers when it is.
pub(crate) fn start_taken(
    connection: &Connection,
    key: &str,
    after: SystemTime,
    spacing: Duration,
) -> rusqlite::Result<SystemTime> {
    let mut limits = Limits::read(connection, key)?;
    let after = milliseconds_from(after);
    let start = limits
        .last_start
        .map_or(after, |last| last.max(after))
        .saturating_add(duration_milliseconds(spacing));
    limits.last_start = Some(start);
    limits.write(connection, key)?;
    Ok(moment(start))
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
    let key = key.to_owned();
    politeness
        .transact(move |connection| start_taken(connection, &key, after, spacing))
        .await
}

/// Takes in `connection` the next start of `key` for a document after the
/// robots.txt at `location` its attempt read at `now`: `fetched`, when its
/// attempt fetched it, that fetch's response arrival, else the fetch of the
/// copy it read. While that fetch is less than `spacing` before `now`, the
/// document may have been admitted before its Crawl-delay was known, so it
/// starts no sooner than `spacing` after that fetch and after the last
/// start any sharing run took ([`start_taken`]); answers when it is.
/// Otherwise its admission kept the delay: `None`.
pub(crate) fn document_start_taken(
    connection: &Connection,
    key: &str,
    location: &str,
    fetched: Option<SystemTime>,
    spacing: Duration,
    now: SystemTime,
) -> rusqlite::Result<Option<SystemTime>> {
    let after = match fetched {
        Some(fetched) => fetched,
        // The copy keeps the millisecond its fetch ended in: it ended
        // before the next.
        None => match robots_txt::stored(connection, location)? {
            Some(copy) => moment(copy.fetched_at.saturating_add(1)),
            None => return Ok(None),
        },
    };
    if after + spacing <= now {
        return Ok(None);
    }
    start_taken(connection, key, after, spacing).map(Some)
}

/// Takes the next start of `key`, for every run sharing `politeness`, for
/// a document whose attempt read the robots.txt at `location` at `now`
/// ([`document_start_taken`]): `spacing` after that robots.txt's fetch,
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
    let (key, location) = (key.to_owned(), location.to_owned());
    politeness
        .transact(move |connection| {
            document_start_taken(connection, &key, &location, fetched, spacing, now)
        })
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
    let key = key.to_owned();
    let last = politeness
        .transact(move |connection| Ok(Limits::read(connection, &key)?.last_start))
        .await?;
    Ok(last
        .and_then(|last| u64::try_from(last).ok())
        .map(|last| SystemTime::UNIX_EPOCH + Duration::from_millis(last)))
}

/// Records in `connection` that the robots.txt at `location` asks the
/// requests of `key` for the Crawl-delay `delay`, or for none, replacing
/// what it asked before.
pub(crate) fn crawl_delay_asked(
    connection: &Connection,
    key: &str,
    location: &str,
    delay: Option<Duration>,
) -> rusqlite::Result<()> {
    match delay {
        Some(delay) => connection.execute(
            "INSERT INTO crawl_delays (key, location, delay) VALUES (?1, ?2, ?3)
             ON CONFLICT (key, location) DO UPDATE SET delay = ?3",
            (key, location, duration_milliseconds(delay)),
        )?,
        None => connection.execute(
            "DELETE FROM crawl_delays WHERE key = ?1 AND location = ?2",
            (key, location),
        )?,
    };
    Ok(())
}

/// The Crawl-delays the robots.txt of `key`'s hosts ask for.
fn crawl_delays(connection: &Connection, key: &str) -> rusqlite::Result<Vec<Duration>> {
    let mut statement =
        connection.prepare_cached("SELECT delay FROM crawl_delays WHERE key = ?1")?;
    statement
        .query_map([key], |row| row.get::<_, i64>(0))?
        .map(|delay| delay.map(|delay| Duration::from_millis(u64::try_from(delay).unwrap_or(0))))
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
    let (key, location) = (key.to_owned(), location.to_owned());
    politeness
        .transact(move |connection| crawl_delay_asked(connection, &key, &location, delay))
        .await
}

/// Records in `connection` that a response of `key` arrived `latency`
/// after its request was sent, into the weighted mean of its latency.
pub(crate) fn latency_noted(
    connection: &Connection,
    key: &str,
    latency: Duration,
) -> rusqlite::Result<()> {
    let mut pacing = Pacing::read(connection, key)?;
    pacing.latency = Some(averaged(pacing.latency, duration_milliseconds(latency)));
    pacing.write(connection, key)
}

/// Moves in `connection` the token a request of `key` sent at `sent` took
/// to its send: while the bucket is still refilling past `sent`, it is
/// full again no sooner than one interval after the send, so a start whose
/// send came after its admission never lets the next come sooner than the
/// rate (ADR 0086's starts are sends). It only ever moves the bucket later.
pub(crate) fn start_moved(
    connection: &Connection,
    key: &str,
    sent: SystemTime,
) -> rusqlite::Result<()> {
    let mut pacing = Pacing::read(connection, key)?;
    let sent = milliseconds_from(sent);
    let (Some(theoretical), Some(interval)) = (pacing.theoretical_start, pacing.interval) else {
        return Ok(());
    };
    if theoretical <= sent {
        return Ok(());
    }
    pacing.theoretical_start = Some(theoretical.max(sent.saturating_add(interval)));
    pacing.write(connection, key)
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
    let key = key.to_owned();
    politeness
        .transact(move |connection| latency_noted(connection, &key, latency))
        .await
}

#[cfg(test)]
mod tests;
