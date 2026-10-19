//! The Proving ground's clock, the one place it reads time (ADR 0059): the
//! oracle's own time, which the benchmark keeps apart from a task's time, and
//! the system time a TOTP code is checked at. Neither ever changes a page or
//! an expected result, and neither is asserted by a test (ADR 0064).

use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Monotonic time since it started.
pub(crate) struct Stopwatch(Instant);

impl Stopwatch {
    pub(crate) fn start() -> Self {
        #[expect(
            clippy::disallowed_methods,
            reason = "the clock module is the Proving ground's one reader of time (ADR 0059)"
        )]
        Self(Instant::now())
    }

    /// Whole microseconds since it started.
    pub(crate) fn microseconds(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_micros()).unwrap_or(u64::MAX)
    }
}

/// Whole seconds since the Unix epoch by the system's clock, 0 before it.
pub(crate) fn unix_seconds() -> u64 {
    #[expect(
        clippy::disallowed_methods,
        reason = "the clock module is the Proving ground's one reader of time (ADR 0059)"
    )]
    let now = SystemTime::now();
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}
