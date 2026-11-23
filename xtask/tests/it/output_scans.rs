//! The output-scan check: every upload of a job's logs, test reports,
//! snapshots and artifacts follows the job's `gitleaks` scan of what it
//! uploads, and runs only when that scan passed (ADR 0067).
//! Covers: story:1-112

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn uploads_behind_a_passing_scan_of_their_paths_pass() {
    let workspace = Workspace::from_fixture("output-scans");

    let outcome = workspace.xtask(&["output-scans"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::no_scan("no-scan.yml", "unscanned-upload")]
#[case::scan_after_the_upload("scan-after-upload.yml", "unscanned-upload")]
#[case::path_the_scan_misses("path-not-scanned.yml", "unscanned-upload")]
#[case::step_after_the_scan("step-after-scan.yml", "unscanned-upload")]
#[case::upload_ignores_the_scan("upload-ignores-scan.yml", "upload-ignores-scan")]
#[case::upload_also_runs_without_the_scan("upload-or-always.yml", "upload-ignores-scan")]
#[case::scan_skipped_when_a_test_fails("scan-skipped-on-failure.yml", "scan-skipped-on-failure")]
fn upload_not_guarded_by_its_scan_fails(#[case] workflow: &str, #[case] kind: &str) {
    let workspace = Workspace::from_fixture("output-scans");
    workspace.add_fixture_file(
        &format!("output-scans-cases/{workflow}"),
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["output-scans"]);

    assert_eq!(outcome.findings(), vec![finding(kind, "extra.yml#upload")]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_job_that_starts_after_its_run_is_cancelled_fails() {
    let workspace = Workspace::from_fixture("output-scans");
    workspace.add_fixture_file(
        "output-scans-cases/job-runs-when-cancelled.yml",
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["output-scans"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("job-runs-when-cancelled", "extra.yml#record")]
    );
    assert_eq!(outcome.code, 1);
}
