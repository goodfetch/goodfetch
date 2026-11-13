//! The test `tracing` subscriber (ADR 0060): it writes to the test's output
//! with a timer that reads simulated time from a closure the caller passes,
//! such as the paused `Clock`'s wall reading, so traces carry no real
//! timestamps. Under `just test`, it also writes `rpc`'s dispatch records
//! and the engine's outcome records (each Error code a Run recorded and no
//! reply carries) as JSON lines into the running test's exercise record,
//! which ADR 0072's audit reads for an in-process test over the in-memory
//! transport. `logged` captures what `tracing` wrote while a closure ran,
//! and [`CapturedLog`] is the writer it captures into, which a test hands
//! to [`subscriber_writing_to`] to read what that subscriber wrote.
#![expect(
    clippy::disallowed_types,
    reason = "a `tracing` writer cannot await: one Mutex guards the bytes a `CapturedLog` captured, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use tracing_subscriber::Layer;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry::LookupSpan;

use crate::exercise::ExerciseRecord;
use crate::facts::{DISPATCH_TARGET, OUTCOME_TARGET};
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
/// the dispatch and outcome records in the test's exercise record.
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
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_timer(SimulatedTime(now.clone()))
                .with_writer(writer)
                .with_filter(LevelFilter::INFO),
        )
        .with(exercise_layer(now))
}

/// The layer of [`subscriber`] that writes the dispatch and outcome records,
/// timed by `now`, into the test's exercise record under `just test`, and
/// is none otherwise: for a test that installs a subscriber of its own,
/// whose record must still show what it reached (ADR 0072).
///
/// # Panics
///
/// Under `just test`, if the test's exercise record cannot be opened.
pub fn exercise_layer<S>(
    now: impl Fn() -> SystemTime + Send + Sync + 'static,
) -> Option<impl Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    ExerciseRecord::of(&Variables::from_process()).map(|record| {
        let file = record
            .open()
            .unwrap_or_else(|error| panic!("the test's exercise record cannot be opened: {error}"));
        tracing_subscriber::fmt::layer()
            .json()
            .with_timer(SimulatedTime(now))
            .with_writer(Arc::new(file))
            .with_filter(
                Targets::new()
                    .with_target(DISPATCH_TARGET, LevelFilter::TRACE)
                    .with_target(OUTCOME_TARGET, LevelFilter::TRACE),
            )
    })
}

/// What `tracing` wrote while `work` ran, at every level and without
/// colour, beside `work`'s answer: the bytes a test searches for what no
/// log line may hold, such as a party's own text.
pub fn logged<T>(work: impl FnOnce() -> T) -> (T, Vec<u8>) {
    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    let answer = tracing::subscriber::with_default(subscriber, work);
    (answer, captured.written())
}

/// A `tracing` writer that keeps the bytes a subscriber wrote, shared by
/// its clones: [`logged`]'s, and one a test hands to
/// [`subscriber_writing_to`] as `move || writer.clone()`.
#[derive(Clone, Default)]
pub struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl CapturedLog {
    /// What was written so far.
    pub fn written(&self) -> Vec<u8> {
        self.bytes().clone()
    }

    fn bytes(&self) -> MutexGuard<'_, Vec<u8>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl io::Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
