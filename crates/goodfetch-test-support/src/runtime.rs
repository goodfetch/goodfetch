//! What tests and bench targets run on (ADRs 0060 and 0071): a paused
//! current-thread Tokio runtime, and proptest's configuration for the
//! running suite on the test's seed.

use proptest::test_runner::{Config, RngSeed};

use crate::variables::{Suite, Variables, current_test};

/// A current-thread Tokio runtime whose clock starts paused, so code that
/// waits takes no real time.
///
/// # Panics
///
/// If Tokio cannot build it.
pub fn runtime() -> tokio::runtime::Runtime {
    #[expect(
        clippy::disallowed_methods,
        reason = "test helpers build the tests' runtime (ADR 0060)"
    )]
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap_or_else(|error| panic!("the test runtime cannot start: {error}"))
}

/// How long [`with_the_blocking_pool_held`] lets its work run.
const HELD_POOL_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// What the future `work` makes answers on a current-thread Tokio runtime
/// whose only blocking thread another job holds, or `None` when it has not
/// answered within 5 seconds of the real clock: work that waits on the
/// runtime's own blocking pool, such as a `tokio::fs` call, never answers,
/// while work run inline, as the engine's inline blocking policy runs it,
/// does (ADR 0060). The clock is the real one, since a held blocking thread
/// keeps a paused clock from advancing.
///
/// # Panics
///
/// If Tokio cannot build the runtime, or the job holding its blocking
/// thread panics.
pub fn with_the_blocking_pool_held<W, F>(work: W) -> Option<F::Output>
where
    W: FnOnce() -> F,
    F: std::future::Future,
{
    #[expect(
        clippy::disallowed_methods,
        reason = "test helpers build the tests' runtime (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap_or_else(|error| panic!("the test runtime cannot start: {error}"));
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (holding, holds) = std::sync::mpsc::channel::<()>();
    runtime.block_on(async {
        #[expect(
            clippy::disallowed_methods,
            reason = "the helper's one job holds the runtime's only blocking thread, outside any owner"
        )]
        let pool = tokio::runtime::Handle::current().spawn_blocking(move || {
            let _ = holding.send(());
            let _ = held.recv();
        });
        holds
            .recv()
            .unwrap_or_else(|error| panic!("the blocking thread never started: {error}"));
        let answered = tokio::time::timeout(HELD_POOL_WAIT, work()).await.ok();
        let _ = release.send(());
        pool.await
            .unwrap_or_else(|error| panic!("the job holding the blocking thread failed: {error}"));
        answered
    })
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
