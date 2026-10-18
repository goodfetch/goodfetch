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
#[case::vitest_retry_in_the_config(
    "vitest/retry-in-the-config.ts",
    "vitest.config.ts",
    "retry-setting"
)]
#[case::vitest_retry_on_a_test(
    "vitest/retry-on-a-test.test.ts",
    "src/sample.test.ts",
    "retry-setting"
)]
#[case::playwright_retries_in_the_config(
    "playwright/retries-in-the-config.ts",
    "playwright.config.ts",
    "retry-setting"
)]
#[case::playwright_describe_configure_retries(
    "playwright/describe-configure-retries.spec.ts",
    "e2e/sample.spec.ts",
    "retry-setting"
)]
#[case::pytest_rerunfailures_in_the_lockfile(
    "pytest/pytest-rerunfailures-in-the-lockfile.lock",
    "uv.lock",
    "retry-plugin"
)]
#[case::flaky_in_the_lockfile("pytest/flaky-in-the-lockfile.lock", "uv.lock", "retry-plugin")]
#[case::pytest_retry_in_the_lockfile(
    "pytest/pytest-retry-in-the-lockfile.lock",
    "uv.lock",
    "retry-plugin"
)]
#[case::pytest_flaky_marker("pytest/flaky-marker.py", "tests/test_sample.py", "flaky-marker")]
#[case::flaky_decorator("pytest/flaky-decorator.py", "tests/test_sample.py", "flaky-marker")]
#[case::pytest_reruns_in_addopts("pytest/reruns-in-addopts.toml", "pyproject.toml", "retry-flag")]
fn refused_retry_fails(#[case] case: &str, #[case] path: &str, #[case] kind: &str) {
    let workspace = Workspace::from_fixture("no-retry");
    workspace.add_fixture_file(&format!("no-retry-cases/{case}"), path);

    let outcome = workspace.xtask(&["no-retry"]);

    assert_eq!(outcome.findings(), vec![finding(kind, path)]);
    assert_eq!(outcome.code, 1);
}
