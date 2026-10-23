//! A crawl's per-domain Politeness (ADRs 0021, 0053 and 0086): the rule its
//! domain keys follow in the per-user Politeness file, so every run of the
//! user that shares Politeness, in any engine, obeys it together. A run
//! admits a request to a domain key only while the count in flight there,
//! across every sharing run, is below its own `politeness.domain.concurrency`
//! and below the key's ceiling a 429 lowered; once its own
//! `politeness.domain.delay` passed since the last request start there by
//! any sharing run; and once a `Retry-After` or a backoff any sharing run
//! met has ended. A backoff waits 2 s, doubling at each one in a row up to
//! 60 s. A 429 halves the key's ceiling (from the count in flight when it
//! had none) and backs it off; the ceiling then grows back by one with each
//! request that succeeds after a quiet [`RECOVERY`] since it last changed.
//! Each rule is a pure function of the wall time it is given (ADR 0059).
//!
//! The per-domain state lives in the `domain_limits` table Foxhound adds to
//! the file; a Collie engine adds none and touches none (ADR 0098).

use std::time::{Duration, SystemTime};

use goodfetch_engine::politeness::{Ending, PermitRule, PolitenessTables, Verdict, milliseconds};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_DOMAIN_CONCURRENCY, POLITENESS_DOMAIN_DELAY,
};
use rusqlite::{Connection, OptionalExtension as _};

/// The first backoff of a domain.
const BACKOFF_INITIAL: Duration = Duration::from_secs(2);

/// The longest backoff of a domain.
const BACKOFF_MAX: Duration = Duration::from_mins(1);

/// How long a lowered ceiling stays quiet before it grows back by one.
pub(crate) const RECOVERY: Duration = Duration::from_secs(10);

/// The table the rule keeps each domain key's shared state in.
pub(crate) const TABLES: PolitenessTables = PolitenessTables {
    name: "goodfetch-frontier",
    statements: &["CREATE TABLE IF NOT EXISTS domain_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoffs INTEGER NOT NULL,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER
        ) STRICT"],
};

/// How long a domain waits after `backoffs` backoffs in a row, the first
/// one included: 2 s, then doubling up to 60 s.
pub(crate) fn backoff(backoffs: u32) -> Duration {
    let doublings = backoffs.saturating_sub(1).min(31);
    BACKOFF_INITIAL
        .saturating_mul(1 << doublings)
        .min(BACKOFF_MAX)
}

/// One domain key's shared state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Limits {
    /// No request before this moment, in milliseconds.
    ready_at: Option<i64>,
    /// Backoffs in a row.
    backoffs: u32,
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
                "SELECT ready_at, backoffs, ceiling, ceiling_changed_at, last_start
                 FROM domain_limits WHERE key = ?1",
                [key],
                |row| {
                    Ok(Self {
                        ready_at: row.get(0)?,
                        backoffs: row.get(1)?,
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
            "INSERT INTO domain_limits (key, ready_at, backoffs, ceiling, ceiling_changed_at, last_start)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (key) DO UPDATE SET ready_at = ?2, backoffs = ?3, ceiling = ?4,
                 ceiling_changed_at = ?5, last_start = ?6",
            (
                key,
                self.ready_at,
                self.backoffs,
                self.ceiling
                    .map(|ceiling| i64::try_from(ceiling).unwrap_or(i64::MAX)),
                self.ceiling_changed_at,
                self.last_start,
            ),
        )?;
        Ok(())
    }

    /// Whether one more request may start at `now` with `in_flight` in
    /// flight, under a run's own `concurrency` and `delay`.
    fn admit(
        &self,
        in_flight: u64,
        now: i64,
        concurrency: Option<u64>,
        delay: Option<Duration>,
    ) -> Result<(), Option<i64>> {
        if let Some(ready_at) = self.ready_at.filter(|ready_at| *ready_at > now) {
            return Err(Some(ready_at));
        }
        if let (Some(delay), Some(last_start)) = (delay, self.last_start) {
            let next = last_start.saturating_add(duration_milliseconds(delay));
            if next > now {
                return Err(Some(next));
            }
        }
        let cap = match (concurrency, self.ceiling) {
            (Some(own), Some(ceiling)) => Some(own.min(ceiling)),
            (own, ceiling) => own.or(ceiling),
        };
        if cap.is_some_and(|cap| in_flight >= cap) {
            return Err(None);
        }
        Ok(())
    }

    /// The state after a request ended at `now` as `ending` says, with
    /// `others` still in flight.
    fn ended(mut self, others: u64, ending: Ending, now: i64) -> Self {
        let wait_until = |limits: &mut Self, until: i64| {
            limits.ready_at = Some(
                limits
                    .ready_at
                    .map_or(until, |ready_at| ready_at.max(until)),
            );
        };
        match ending {
            Ending::Done => {
                self.backoffs = 0;
                if let (Some(ceiling), Some(changed_at)) = (self.ceiling, self.ceiling_changed_at)
                    && now >= changed_at.saturating_add(duration_milliseconds(RECOVERY))
                {
                    self.ceiling = Some(ceiling.saturating_add(1));
                    self.ceiling_changed_at = Some(now);
                }
            }
            Ending::BackOff => {
                self.backoffs = self.backoffs.saturating_add(1);
                let until = now.saturating_add(duration_milliseconds(backoff(self.backoffs)));
                wait_until(&mut self, until);
            }
            Ending::Until(until) => wait_until(&mut self, milliseconds(until)),
            Ending::TooManyRequests(until) => {
                let base = self.ceiling.unwrap_or(others.saturating_add(1));
                self.ceiling = Some((base / 2).max(1));
                self.ceiling_changed_at = Some(now);
                self.backoffs = self.backoffs.saturating_add(1);
                let until = until.map_or_else(
                    || now.saturating_add(duration_milliseconds(backoff(self.backoffs))),
                    milliseconds,
                );
                wait_until(&mut self, until);
            }
        }
        self
    }
}

fn duration_milliseconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// A crawl's per-domain rule under its own settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DomainRule {
    concurrency: Option<u64>,
    delay: Option<Duration>,
}

impl DomainRule {
    /// The rule `settings` give a crawl.
    pub(crate) fn of(settings: &Resolved) -> Self {
        Self {
            concurrency: settings.count(POLITENESS_DOMAIN_CONCURRENCY.name),
            delay: settings
                .duration(POLITENESS_DOMAIN_DELAY.name)
                .filter(|delay| !delay.is_zero()),
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
        let limits = Limits::read(connection, key)?;
        Ok(
            match limits.admit(in_flight, milliseconds(now), self.concurrency, self.delay) {
                Ok(()) => Verdict::Go,
                Err(Some(until)) => Verdict::Until(goodfetch_engine::politeness::moment(until)),
                Err(None) => Verdict::Busy,
            },
        )
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
        let limits = Limits::read(connection, key)?.ended(in_flight, ending, now);
        limits.write(connection, key)?;
        Ok(limits
            .ready_at
            .filter(|ready_at| *ready_at > now)
            .map(goodfetch_engine::politeness::moment))
    }
}

#[cfg(test)]
mod tests;
