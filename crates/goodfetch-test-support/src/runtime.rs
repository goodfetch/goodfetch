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
