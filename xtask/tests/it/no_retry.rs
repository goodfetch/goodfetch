//! The no-retry check: no runner configuration, test source, lockfile,
//! workflow or suite command lets a test run again after it failed (ADRs 0064
//! and 0074). Each case copies one file over a clean fixture workspace.
//! Covers: story:1-65

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn workspace_whose_runners_never_retry_passes() {
    let workspace = Workspace::from_fixture("no-retry");

    let outcome = workspace.xtask(&["no-retry"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::nextest_retries_in_a_profile(
    "nextest/retries-in-a-profile.toml",
    ".config/nextest.toml",
    "retry-setting"
)]
#[case::nextest_retries_with_backoff(
    "nextest/retries-with-backoff.toml",
    ".config/nextest.toml",
    "retry-setting"
)]
#[case::nextest_retries_in_a_per_test_override(
    "nextest/retries-in-an-override.toml",
    ".config/nextest.toml",
    "retry-setting"
)]
#[case::nextest_flaky_result_pass_in_a_profile(
    "nextest/flaky-result-pass-in-a-profile.toml",
    ".config/nextest.toml",
    "flaky-result-not-fail"
)]
#[case::nextest_flaky_result_in_a_per_test_override(
    "nextest/flaky-result-in-an-override.toml",
    ".config/nextest.toml",
    "flaky-result-not-fail"
)]
#[case::nextest_default_profile_without_flaky_result(
    "nextest/default-profile-without-flaky-result.toml",
    ".config/nextest.toml",
    "flaky-result-not-fail"
)]
fn refused_retry_fails(#[case] case: &str, #[case] path: &str, #[case] kind: &str) {
    let workspace = Workspace::from_fixture("no-retry");
    workspace.add_fixture_file(&format!("no-retry-cases/{case}"), path);

    let outcome = workspace.xtask(&["no-retry"]);

    assert_eq!(outcome.findings(), vec![finding(kind, path)]);
    assert_eq!(outcome.code, 1);
}
