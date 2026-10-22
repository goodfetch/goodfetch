//! A crawl's per-domain Politeness (ADRs 0021, 0053, 0086 and 0118): the
//! rule its domain keys follow in the per-user Politeness file, so every
//! run of the user that shares Politeness, in any engine, obeys it
//! together. A run admits a request to a domain key only while the count in
//! flight there, across every sharing run, is below its own
//! `politeness.domain.concurrency` and below the key's ceiling a 429
//! lowered; once its own `politeness.domain.delay` passed since the last
//! request start there by any sharing run; and once a `Retry-After` or a
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
//! The per-domain state lives in the `domain_limits` table Foxhound adds to
//! the file; a Collie engine adds none and touches none (ADR 0098).

#![expect(
    clippy::disallowed_types,
    reason = "the rule runs inside the Politeness file's transaction, a blocking call that may run inline on a runtime thread: one Mutex guards a gate's Entropy for one draw, never held with another lock or across an await, so there is no lock order to test under loom"
)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::politeness::{
    Ending, PermitRule, PolitenessTables, Verdict, milliseconds, moment,
};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_BACKOFF_ENABLED, POLITENESS_BACKOFF_INITIAL, POLITENESS_BACKOFF_MAX,
    POLITENESS_DOMAIN_CONCURRENCY, POLITENESS_DOMAIN_DELAY, POLITENESS_RETRY_AFTER_MAX,
};
use rusqlite::{Connection, OptionalExtension as _};

/// How long a lowered ceiling stays quiet before it grows back by one.
pub(crate) const RECOVERY: Duration = Duration::from_secs(10);

/// The table the rule keeps each domain key's shared state in.
pub(crate) const TABLES: PolitenessTables = PolitenessTables {
    name: "goodfetch-frontier",
    statements: &["CREATE TABLE IF NOT EXISTS domain_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoff INTEGER,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER
        ) STRICT"],
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
    /// flight, under a run's own rule.
    fn admit(&self, in_flight: u64, now: i64, rule: &DomainRule) -> Verdict {
        if let Some(longest) = rule.waits.longest()
            && let Some(ready_at) = self.ready_at.filter(|ready_at| *ready_at > now)
        {
            return if ready_at.saturating_sub(now) > duration_milliseconds(longest) {
                Verdict::Refused(moment(ready_at))
            } else {
                Verdict::Until(moment(ready_at))
            };
        }
        if let (Some(delay), Some(last_start)) = (rule.delay, self.last_start) {
            let next = last_start.saturating_add(duration_milliseconds(delay));
            if next > now {
                return Verdict::Until(moment(next));
            }
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
            waits: Waits::of(settings),
            entropy,
        }
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
        Ok(Limits::read(connection, key)?.admit(in_flight, milliseconds(now), self))
    }

    fn started(&self, connection: &Connection, key: &str, now: SystemTime) -> rusqlite::Result<()> {
        let mut limits = Limits::read(connection, key)?;
        limits.last_start = Some(milliseconds(now));
        limits.write(connection, key)
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

#[cfg(test)]
mod tests;
