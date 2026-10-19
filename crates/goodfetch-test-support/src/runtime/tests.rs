use std::time::Duration;

use proptest::test_runner::RngSeed;
use rstest::rstest;

use crate::variables::Suite;

use super::{proptest_config_for, runtime};

#[rstest]
#[case::quick(Suite::Quick, 16)]
#[case::pre_merge(Suite::PreMerge, 256)]
#[case::release(Suite::Release, 256)]
fn proptest_config_takes_the_suites_case_count(#[case] suite: Suite, #[case] cases: u32) {
    let config = proptest_config_for(suite, 9);

    assert_eq!(config.cases, cases);
}

#[test]
fn proptest_config_runs_on_the_tests_seed() {
    let config = proptest_config_for(Suite::Quick, 9);

    assert_eq!(config.rng_seed, RngSeed::Fixed(9));
}

#[test]
fn runtime_runs_on_a_paused_clock() {
    let runtime = runtime();

    #[expect(
        clippy::disallowed_methods,
        reason = "reads the paused clock the runtime advances"
    )]
    let waited = runtime.block_on(async {
        let start = tokio::time::Instant::now();
        tokio::time::sleep(Duration::from_secs(3600)).await;
        start.elapsed()
    });

    assert_eq!(waited, Duration::from_secs(3600));
}
