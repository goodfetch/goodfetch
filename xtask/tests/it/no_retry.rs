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
#[case::nextest_retries_flag("just/nextest-retries-flag.justfile", "justfile", "retry-flag")]
#[case::nextest_flaky_result_flag(
    "just/nextest-flaky-result-flag.justfile",
    "justfile",
    "retry-flag"
)]
#[case::nextest_retries_variable(
    "just/nextest-retries-variable.justfile",
    "justfile",
    "retry-flag"
)]
#[case::nextest_flaky_result_variable(
    "just/nextest-flaky-result-variable.justfile",
    "justfile",
    "retry-flag"
)]
#[case::nextest_retries_variable_in_a_workflow(
    "workflows/nextest-retries-variable.yml",
    ".github/workflows/pull-request.yml",
    "retry-flag"
)]
#[case::nextest_retries_variable_in_cargo_config(
    "cargo/nextest-retries-variable.toml",
    ".cargo/config.toml",
    "retry-flag"
)]
#[case::vitest_retry_flag("just/vitest-retry-flag.justfile", "justfile", "retry-flag")]
#[case::vitest_retry_flag_in_a_package_script(
    "package/vitest-retry-flag.json",
    "package.json",
    "retry-flag"
)]
#[case::playwright_retries_flag("just/playwright-retries-flag.justfile", "justfile", "retry-flag")]
#[case::playwright_retries_flag_in_a_package_script(
    "package/playwright-retries-flag.json",
    "package.json",
    "retry-flag"
)]
#[case::retry_action(
    "workflows/retry-action.yml",
    ".github/workflows/pull-request.yml",
    "retry-action"
)]
#[case::shell_until_loop_around_a_test("just/until-loop.justfile", "justfile", "retry-loop")]
#[case::shell_while_loop_around_a_test("just/while-loop.justfile", "justfile", "retry-loop")]
#[case::shell_until_loop_around_just_repeat(
    "just/until-loop-around-repeat.justfile",
    "justfile",
    "retry-loop"
)]
#[case::shell_until_loop_around_a_recipes_own_just(
    "just/until-loop-around-its-own-just.justfile",
    "justfile",
    "retry-loop"
)]
#[case::shell_for_loop_breaking_on_a_passing_test(
    "workflows/for-loop-with-break.yml",
    ".github/workflows/pull-request.yml",
    "retry-loop"
)]
#[case::continue_on_error_on_a_test_step(
    "workflows/continue-on-error-on-a-test-step.yml",
    ".github/workflows/pull-request.yml",
    "continue-on-error"
)]
#[case::continue_on_error_on_a_job_with_a_test_step(
    "workflows/continue-on-error-on-a-test-job.yml",
    ".github/workflows/pull-request.yml",
    "continue-on-error"
)]
#[case::chromium_test_launcher_retry_limit(
    "chromium/test-launcher-retry-limit.sh",
    "chromium/gate.sh",
    "retry-flag"
)]
#[case::chromium_gtest_run_without_a_retry_limit(
    "chromium/gtest-without-retry-limit.sh",
    "chromium/gate.sh",
    "gtest-without-retry-limit"
)]
#[case::chromium_num_retries("chromium/num-retries.sh", "chromium/gate.sh", "retry-flag")]
#[case::wptrunner_retry_unexpected(
    "chromium/retry-unexpected.sh",
    "chromium/gate.sh",
    "retry-flag"
)]
#[case::wptrunner_rerun_above_one("chromium/rerun.sh", "chromium/gate.sh", "retry-flag")]
#[case::wptrunner_repeat_until_unexpected(
    "chromium/repeat-until-unexpected.sh",
    "chromium/gate.sh",
    "retry-flag"
)]
#[case::typ_retry_limit("chromium/typ-retry-limit.sh", "chromium/gate.sh", "retry-flag")]
#[case::typ_retry_limit_in_a_python_argument_list(
    "chromium/typ-retry-limit-in-python.py",
    "chromium/run_gpu_tests.py",
    "retry-flag"
)]
#[case::typ_isolated_script_retry_limit(
    "chromium/isolated-script-retry-limit.sh",
    "chromium/gate.sh",
    "retry-flag"
)]
#[case::typ_retry_only_retry_on_failure_tests(
    "chromium/retry-only-retry-on-failure-tests.sh",
    "chromium/gate.sh",
    "retry-flag"
)]
#[case::retry_on_failure_expectation(
    "chromium/retry-on-failure-expectation.txt",
    "chromium/expectations.txt",
    "retry-expectation"
)]
// "all green" refuses a re-run only after a failed step named `Test…`
// (ADR 0064), so a step that runs a test runner must be named so.
#[case::runner_step_without_a_test_name(
    "workflows/runner-step-without-a-test-name.yml",
    ".github/workflows/pull-request.yml",
    "unnamed-test-step"
)]
#[case::miri_step_without_a_test_name(
    "workflows/miri-step-without-a-test-name.yml",
    ".github/workflows/pull-request.yml",
    "unnamed-test-step"
)]
#[case::stress_step_without_a_test_name(
    "workflows/stress-step-without-a-test-name.yml",
    ".github/workflows/pull-request.yml",
    "unnamed-test-step"
)]
fn refused_retry_fails(#[case] case: &str, #[case] path: &str, #[case] kind: &str) {
    let workspace = Workspace::from_fixture("no-retry");
    workspace.add_fixture_file(&format!("no-retry-cases/{case}"), path);

    let outcome = workspace.xtask(&["no-retry"]);

    assert_eq!(outcome.findings(), vec![finding(kind, path)]);
    assert_eq!(outcome.code, 1);
}
