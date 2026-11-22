//! What tests and bench targets run on (ADRs 0060 and 0071): a paused
//! current-thread Tokio runtime, one whose blocking pool a test can hold,
//! and proptest's configuration for the running suite on the test's seed.

use std::time::Duration;

use proptest::test_runner::{Config, RngSeed};

use crate::variables::{Suite, Variables, current_test};

/// A current-thread Tokio runtime whose clock starts paused, so code that
/// waits takes no real time.
///
/// # Panics
///
/// If Tokio cannot build it.
pub fn runtime() -> tokio::runtime::Runtime {
    started(&mut paused_current_thread())
}

/// A runtime like [`runtime`] whose blocking pool has a single thread, for a
/// test that holds it with [`with_the_blocking_pool_held`].
///
/// # Panics
///
/// If Tokio cannot build it.
pub fn runtime_with_one_blocking_thread() -> tokio::runtime::Runtime {
    started(paused_current_thread().max_blocking_threads(1))
}

/// A current-thread Tokio runtime on real time, as a program's own may be,
/// for a test that drives an SDK the way a program does.
///
/// # Panics
///
/// If Tokio cannot build it.
pub fn real_time_runtime() -> tokio::runtime::Runtime {
    started(goodfetch_runtime::current_thread().enable_all())
}

/// A current-thread runtime's builder, its clock paused.
fn paused_current_thread() -> tokio::runtime::Builder {
    let mut builder = goodfetch_runtime::current_thread();
    builder.enable_all().start_paused(true);
    builder
}

/// The runtime `builder` builds.
fn started(builder: &mut tokio::runtime::Builder) -> tokio::runtime::Runtime {
    builder
        .build()
        .unwrap_or_else(|error| panic!("the test runtime cannot start: {error}"))
}

/// Awaits `work` while a job holds a thread of the current runtime's
/// blocking pool, the whole pool on [`runtime_with_one_blocking_thread`],
/// and returns what `work` returned and whether the job still held its
/// thread when `work` ended: `false` when `work` waited for a thread of the
/// pool. The job lets its thread go after 5 seconds of real time in any
/// case, so work that waits for it still ends, even on a paused clock,
/// which does not advance while the job runs.
///
/// # Panics
///
/// If the job cannot run on the pool.
pub async fn with_the_blocking_pool_held<F: Future>(work: F) -> (F::Output, bool) {
    let (release, released) = std::sync::mpsc::channel::<()>();
    let (holding, holds) = std::sync::mpsc::channel::<()>();
    #[expect(
        clippy::disallowed_methods,
        reason = "holding the runtime's blocking pool is the job's whole job, and test support reaches no owner's tracker (TESTING.md, Shared helpers)"
    )]
    let job = tokio::runtime::Handle::current().spawn_blocking(move || {
        let _ = holding.send(());
        released.recv_timeout(Duration::from_secs(5)).is_ok()
    });
    holds
        .recv()
        .unwrap_or_else(|error| panic!("the job that holds the blocking pool ended: {error}"));
    let output = work.await;
    let _ = release.send(());
    let held_throughout = job
        .await
        .unwrap_or_else(|error| panic!("the job that holds the blocking pool failed: {error}"));
    (output, held_throughout)
}

/// Proptest's configuration for the running test: [`proptest_config_for`]
/// its suite and seed. `PROPTEST_CASES` is never set (ADR 0060).
///
/// # Panics
///
/// If `GOODFETCH_TEST_SEED` or `GOODFETCH_TEST_SUITE` cannot be read.
pub fn proptest_config() -> Config {
    let variables = Variables::from_process();
    let read = |error: &dyn std::fmt::Display| -> ! {
        panic!("proptest's configuration cannot be read: {error}")
    };
    let seed = variables
        .seed(&current_test())
        .unwrap_or_else(|error| read(&error));
    let suite = variables.suite().unwrap_or_else(|error| read(&error));
    proptest_config_for(suite, seed)
}

/// 16 cases in the Quick suite and 256 above it, on this seed. A test that
/// needs another count sets it itself.
pub fn proptest_config_for(suite: Suite, seed: u64) -> Config {
    let cases = match suite {
        Suite::Quick => 16,
        Suite::PreMerge | Suite::Release => 256,
    };
    Config {
        cases,
        rng_seed: RngSeed::Fixed(seed),
        ..Config::default()
    }
}

#[cfg(test)]
mod tests;
