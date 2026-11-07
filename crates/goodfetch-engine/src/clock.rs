//! The `Clock` handle (ADR 0059): the one way the engine reads wall time,
//! reads monotonic time and sleeps until a monotonic moment. Its system
//! version reads the OS and `tokio::time`; its test version,
//! [`Clock::paused`], runs on Tokio's paused clock, wall time being a fixed
//! start plus the time the paused clock advanced, so the two never
//! disagree. Rules that depend on time take a reading as an argument.
//! Wall time is written in two forms: a timestamp at the clock's own
//! precision, as the Audit log, the Usage ledger and the known certificates
//! record it, and a point in time to the millisecond, as a Run and "Needs
//! you" write theirs; both are RFC 3339 in UTC.

use std::time::{Duration, SystemTime};

use goodfetch_protocol::run::PointInTime;
use jiff::Timestamp;
use tokio::time::Instant;

/// Where the engine reads time.
#[derive(Clone, Copy, Debug)]
pub struct Clock(Source);

#[derive(Clone, Copy, Debug)]
enum Source {
    System,
    Paused { wall: SystemTime, since: Instant },
}

impl Clock {
    /// The OS's clock, which only `main` builds.
    pub fn system() -> Self {
        Self(Source::System)
    }

    /// The test version: wall time starts at `start` and moves with Tokio's
    /// paused clock. `disallowed-methods` refuses it outside tests and bench
    /// targets.
    ///
    /// # Panics
    ///
    /// Off a Tokio runtime: made before the runtime it runs on, it would
    /// count the real time until that runtime is built as wall time.
    pub fn paused(start: SystemTime) -> Self {
        tokio::runtime::Handle::try_current()
            .expect("invariant: a paused Clock is made on the runtime it runs on");
        Self(Source::Paused {
            wall: start,
            since: monotonic(),
        })
    }

    /// Wall time, in UTC.
    pub fn wall(&self) -> SystemTime {
        match self.0 {
            #[expect(
                clippy::disallowed_methods,
                reason = "the Clock is the one reader of wall time (ADR 0059)"
            )]
            Source::System => SystemTime::now(),
            Source::Paused { wall, since } => wall + monotonic().duration_since(since),
        }
    }

    /// Wall time as a timestamp, at the clock's own precision.
    ///
    /// # Panics
    ///
    /// When the wall time is outside the years 1 to 9999, which no OS
    /// clock reads.
    pub fn timestamp(&self) -> Timestamp {
        timestamp(self.wall())
    }

    /// Wall time as a point in time, to the millisecond (ADR 0102).
    ///
    /// # Panics
    ///
    /// As [`Self::timestamp`].
    pub fn point_in_time(&self) -> PointInTime {
        to_the_millisecond(self.timestamp())
    }

    /// The wall time `wait` from now as a point in time, to the
    /// millisecond, such as when a domain is next admitted.
    ///
    /// # Panics
    ///
    /// As [`Self::timestamp`].
    pub fn point_in_time_after(&self, wait: Duration) -> PointInTime {
        to_the_millisecond(timestamp(self.wall() + wait))
    }

    /// Monotonic time, which only goes up.
    pub fn now(&self) -> Instant {
        monotonic()
    }

    /// Waits until `moment`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn sleep_until(&self, moment: Instant) {
        tokio::time::sleep_until(moment).await;
    }

    /// Waits for `duration`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn sleep(&self, duration: Duration) {
        self.sleep_until(self.now() + duration).await;
    }
}

fn timestamp(wall: SystemTime) -> Timestamp {
    Timestamp::try_from(wall).expect("invariant: the clock reads a time jiff can hold")
}

fn to_the_millisecond(at: Timestamp) -> PointInTime {
    PointInTime::from_rfc3339(at.round(jiff::Unit::Millisecond).unwrap_or(at).to_string())
}

fn monotonic() -> Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Clock is the one reader of monotonic time (ADR 0059)"
    )]
    Instant::now()
}

#[cfg(test)]
mod tests;
