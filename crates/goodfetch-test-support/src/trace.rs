//! The test `tracing` subscriber (ADR 0060): it writes to the test's output
//! with a timer that reads simulated time from a closure the caller passes,
//! such as the paused `Clock`'s wall reading, so traces carry no real
//! timestamps.

use std::fmt;
use std::time::SystemTime;

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

/// Writes each event's time as RFC 3339, read from the closure.
struct SimulatedTime<F>(F);

impl<F: Fn() -> SystemTime> FormatTime for SimulatedTime<F> {
    fn format_time(&self, writer: &mut Writer<'_>) -> fmt::Result {
        let now = jiff::Timestamp::try_from((self.0)()).map_err(|_| fmt::Error)?;
        write!(writer, "{now}")
    }
}

/// The subscriber a test installs: the test's output, timed by `now`.
pub fn subscriber(
    now: impl Fn() -> SystemTime + Send + Sync + 'static,
) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::fmt()
        .with_timer(SimulatedTime(now))
        .with_test_writer()
        .finish()
}

/// The same subscriber writing to `writer` instead of the test's output.
pub fn subscriber_writing_to<W>(
    now: impl Fn() -> SystemTime + Send + Sync + 'static,
    writer: W,
) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .with_timer(SimulatedTime(now))
        .with_writer(writer)
        .finish()
}

#[cfg(test)]
mod tests;
