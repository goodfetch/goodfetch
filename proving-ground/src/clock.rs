//! The Proving ground's clock, the one place it reads time (ADR 0059): the
//! oracle's own time, which the benchmark keeps apart from a task's time,
//! the time of each request it observes, and the date its leaf certificates
//! start from, and the latency proxy's delays and calibration. Times are
//! measurements, never asserted by a test (ADR 0064).

use std::time::{Instant, SystemTime};

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

/// Tokio's monotonic time now, which the latency proxy schedules each delay
/// from: a test that pauses Tokio's clock (`start_paused`, `TESTING.md`'s
/// test `Clock`) moves it, so no delay takes real time there.
pub(crate) fn now() -> tokio::time::Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "the clock module is the Proving ground's one reader of time (ADR 0059)"
    )]
    tokio::time::Instant::now()
}

/// The wall clock's time now, which dates the leaf certificates.
pub(crate) fn wall_now() -> SystemTime {
    #[expect(
        clippy::disallowed_methods,
        reason = "the clock module is the Proving ground's one reader of time (ADR 0059)"
    )]
    SystemTime::now()
}
