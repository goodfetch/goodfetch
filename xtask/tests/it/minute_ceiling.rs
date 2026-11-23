//! The minute ceiling of test health while the repository is private (ADRs
//! 0064, 0072 and 0108, as amended by ADR 0135): the month's hosted minutes
//! of the test-health, diagnostic and coverage jobs, counted from the store
//! in Linux-equivalent minutes, skip the constrained runs, then the
//! diagnostic runs, as they near 1,000, and the coverage job from 550; a
//! skip goes to the job summary. The in-diff mutation job runs on the
//! owner's machine and counts nothing.
//! Covers: story:1-113

use rstest::rstest;

use crate::store::{Attempt, STORE, job, write_store};
use crate::workspace::{Outcome, Workspace};

fn ceiling(workspace: &Workspace, run: &str) -> Outcome {
    ceiling_for_jobs(workspace, run, "1")
}

/// The ceiling's answer for a run of `jobs` hosted jobs.
fn ceiling_for_jobs(workspace: &Workspace, run: &str, jobs: &str) -> Outcome {
    workspace.xtask(&[
        "results",
        "ceiling",
        "--store",
        STORE,
        "--run",
        run,
        "--jobs",
        jobs,
        "--today",
        "2026-10-18",
        "--summary",
        "summary.md",
    ])
}

/// The value of `key` in the command's `key=value` lines.
fn value(outcome: &Outcome, key: &str) -> Option<String> {
    outcome
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('=').map(str::to_owned))
}

fn attempt(
    workflow: &'static str,
    run: u64,
    started_at: &'static str,
    jobs: Vec<serde_json::Value>,
) -> Attempt {
    let mut attempt = Attempt::new(run, started_at);
    attempt.workflow = workflow;
    attempt.jobs = jobs;
    attempt
}

/// A month whose counted jobs took `minutes` Linux minutes, in one job.
fn month_of(workspace: &Workspace, minutes: u64) {
    write_store(
        workspace,
        &[attempt(
            ".github/workflows/test-health.yml",
            10,
            "2026-10-02T03:00:00Z",
            vec![job(&["ubuntu-24.04"], minutes * 60, "2026-10-02T03:00:00Z")],
        )],
    );
}

#[test]
fn month_counts_hosted_minutes_of_test_health_jobs_rounded_up_and_multiplied() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            attempt(
                ".github/workflows/test-health.yml",
                10,
                "2026-10-02T03:00:00Z",
                vec![
                    // 2 minutes 5 seconds: 3 Linux minutes.
                    job(&["ubuntu-24.04"], 125, "2026-10-02T03:00:00Z"),
                    // 1 minute 1 second on Linux arm64: 2.
                    job(&["ubuntu-24.04-arm"], 61, "2026-10-02T03:00:00Z"),
                    // The owner's machine counts nothing (ADR 0108).
                    job(&["self-hosted", "macOS"], 600, "2026-10-02T03:00:00Z"),
                ],
            ),
            attempt(
                ".github/workflows/diagnostic.yml",
                11,
                "2026-10-03T03:00:00Z",
                vec![
                    // 30 seconds on Windows: 1 minute times 2.
                    job(&["windows-2025"], 30, "2026-10-03T03:00:00Z"),
                    // 61 seconds on macOS: 2 minutes times 11.
                    job(&["macos-15"], 61, "2026-10-03T03:00:00Z"),
                ],
            ),
            // Another workflow's minutes are another ceiling's.
            attempt(
                ".github/workflows/pull-request.yml",
                12,
                "2026-10-04T03:00:00Z",
                vec![job(&["ubuntu-24.04"], 600, "2026-10-04T03:00:00Z")],
            ),
            // Last month's minutes are spent.
            attempt(
                ".github/workflows/test-health.yml",
                9,
                "2026-09-30T03:00:00Z",
                vec![job(&["ubuntu-24.04"], 600, "2026-09-30T03:00:00Z")],
            ),
        ],
    );

    let outcome = ceiling(&workspace, "constrained");

    assert_eq!(outcome.code, 0);
    assert_eq!(value(&outcome, "minutes").as_deref(), Some("29"));
}

#[test]
fn month_counts_the_coverage_jobs_hosted_minutes() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[attempt(
            ".github/workflows/coverage.yml",
            13,
            "2026-10-10T05:00:00Z",
            vec![
                job(&["ubuntu-24.04"], 2_400, "2026-10-10T05:00:00Z"),
                job(&["ubuntu-24.04"], 30, "2026-10-10T05:00:00Z"),
            ],
        )],
    );

    let outcome = ceiling(&workspace, "coverage");

    assert_eq!(outcome.code, 0);
    assert_eq!(value(&outcome, "minutes").as_deref(), Some("41"));
}

#[test]
fn skipped_coverage_job_names_its_threshold_in_the_job_summary() {
    let workspace = Workspace::from_fixture("empty");
    month_of(&workspace, 550);

    let outcome = ceiling(&workspace, "coverage");

    assert_eq!(outcome.code, 0);
    let summary = String::from_utf8(workspace.read("summary.md").unwrap()).unwrap();
    assert!(
        summary.contains("used 550 included minutes, and the coverage job starts only below 550"),
        "{summary}"
    );
}

#[rstest]
#[case::constrained_while_its_120_minutes_fit(880, "constrained", true)]
#[case::constrained_once_its_120_minutes_do_not_fit(881, "constrained", false)]
#[case::diagnostic_while_its_10_minutes_fit(990, "diagnostic", true)]
#[case::diagnostic_once_its_10_minutes_do_not_fit(991, "diagnostic", false)]
#[case::coverage_below_550(549, "coverage", true)]
#[case::coverage_from_550(550, "coverage", false)]
fn run_starts_only_while_the_month_leaves_it_room(
    #[case] minutes: u64,
    #[case] run: &str,
    #[case] admitted: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    month_of(&workspace, minutes);

    let outcome = ceiling(&workspace, run);

    assert_eq!(outcome.code, 0);
    assert_eq!(
        value(&outcome, "admitted").as_deref(),
        Some(if admitted { "true" } else { "false" })
    );
}

/// A failed run gets up to 20 hosted diagnostic jobs, each with a 10-minute
/// timeout: the month must have room for all of them.
#[rstest]
#[case::while_every_jobs_10_minutes_fit(950, "5", true)]
#[case::once_they_do_not_all_fit(951, "5", false)]
fn diagnostic_run_starts_only_while_the_month_leaves_room_for_every_job(
    #[case] minutes: u64,
    #[case] jobs: &str,
    #[case] admitted: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    month_of(&workspace, minutes);

    let outcome = ceiling_for_jobs(&workspace, "diagnostic", jobs);

    assert_eq!(outcome.code, 0);
    assert_eq!(
        value(&outcome, "admitted").as_deref(),
        Some(if admitted { "true" } else { "false" })
    );
}

#[rstest]
#[case::skipped(881, true)]
#[case::admitted(880, false)]
fn skipped_run_writes_the_skip_to_the_job_summary(#[case] minutes: u64, #[case] summarized: bool) {
    let workspace = Workspace::from_fixture("empty");
    month_of(&workspace, minutes);

    let outcome = ceiling(&workspace, "constrained");

    assert_eq!(outcome.code, 0);
    assert_eq!(
        workspace
            .read("summary.md")
            .is_some_and(|summary| !summary.is_empty()),
        summarized
    );
}
