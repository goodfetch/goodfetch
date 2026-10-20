//! The `Clock` handle (ADR 0059): the one way the engine reads wall time,
//! reads monotonic time and sleeps until a monotonic moment. Its system
//! version reads the OS and `tokio::time`; its test version,
//! [`Clock::paused`], runs on Tokio's paused clock, wall time being a fixed
//! start plus the time the paused clock advanced, so the two never
//! disagree. Rules that depend on time take a reading as an argument.

use std::time::{Duration, SystemTime};

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

fn monotonic() -> Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Clock is the one reader of monotonic time (ADR 0059)"
    )]
    Instant::now()
}

#[cfg(test)]
mod tests;
