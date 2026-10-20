//! The test `tracing` subscriber (ADR 0060): it writes to the test's output
//! with a timer that reads simulated time from a closure the caller passes,
//! such as the paused `Clock`'s wall reading, so traces carry no real
//! timestamps. Under `just test`, it also writes `rpc`'s dispatch records
//! as JSON lines into the running test's exercise record, which ADR 0072's
//! audit reads for an in-process test over the in-memory transport.

use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;

use tracing_subscriber::Layer as _;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::exercise::{DISPATCH_TARGET, ExerciseRecord};
use crate::variables::Variables;

/// Writes each event's time as RFC 3339, read from the closure.
struct SimulatedTime<F>(F);

impl<F: Fn() -> SystemTime> FormatTime for SimulatedTime<F> {
    fn format_time(&self, writer: &mut Writer<'_>) -> fmt::Result {
        let now = jiff::Timestamp::try_from((self.0)()).map_err(|_| fmt::Error)?;
        write!(writer, "{now}")
    }
}

/// The subscriber a test installs: the test's output, timed by `now`, and
/// the dispatch records in the test's exercise record.
///
/// # Panics
///
/// Under `just test`, if the test's exercise record cannot be opened.
pub fn subscriber(
    now: impl Fn() -> SystemTime + Send + Sync + Clone + 'static,
) -> impl tracing::Subscriber + Send + Sync {
    subscriber_writing_to(now, tracing_subscriber::fmt::TestWriter::new)
}

/// The same subscriber writing to `writer` instead of the test's output.
///
/// # Panics
///
/// Under `just test`, if the test's exercise record cannot be opened.
pub fn subscriber_writing_to<W>(
    now: impl Fn() -> SystemTime + Send + Sync + Clone + 'static,
    writer: W,
) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let record = ExerciseRecord::of(&Variables::from_process()).map(|record| {
        let file = record
            .open()
            .unwrap_or_else(|error| panic!("the test's exercise record cannot be opened: {error}"));
        tracing_subscriber::fmt::layer()
            .json()
            .with_timer(SimulatedTime(now.clone()))
            .with_writer(Arc::new(file))
            .with_filter(Targets::new().with_target(DISPATCH_TARGET, LevelFilter::TRACE))
    });
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_timer(SimulatedTime(now))
                .with_writer(writer)
                .with_filter(LevelFilter::INFO),
        )
        .with(record)
}

#[cfg(test)]
mod tests;
