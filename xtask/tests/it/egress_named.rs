//! The jobs ADR 0043 names, each reaching its own listed endpoints under the
//! deny, run only on a schedule, by hand or from another such workflow,
//! never for a pull request's code; a fixture's licence endpoint, which a
//! fixture of any tier may need, is the one exception.
//! Covers: story:1-70

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[rstest]
#[case::saas_sinks_for_a_pull_request(
    "saas-sinks",
    "pull_request:",
    vec![finding("named-job-on-pull-request", "pull-request.yml#quick")]
)]
#[case::vendor_conformance_in_the_merge_queue(
    "vendor-conformance",
    "merge_group:",
    vec![finding("named-job-on-pull-request", "pull-request.yml#quick")]
)]
#[case::saas_sinks_nightly("saas-sinks", "schedule:\n    - cron: \"0 3 * * *\"", vec![])]
#[case::fixture_licence_for_a_pull_request("fixture-licence", "pull_request:", vec![])]
fn named_job_runs_for_no_pull_request(
    #[case] exception: &str,
    #[case] trigger: &str,
    #[case] expected: Vec<crate::workspace::Finding>,
) {
    let workspace = Workspace::from_fixture("egress");
    workspace.edit(
        "ci/egress/endpoints.toml",
        "[[deny]]\njob = \"pull-request.yml#quick\"\n",
        &format!(
            "[[named]]\njob = \"pull-request.yml#quick\"\nexception = \"{exception}\"\nendpoints = [\"db.example.com:5432\"]\n"
        ),
    );
    workspace.edit(
        ".github/workflows/pull-request.yml",
        "on:\n  pull_request:\n",
        &format!("on:\n  {trigger}\n"),
    );

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(
        outcome.code,
        i32::from(!expected.is_empty()),
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.findings(), expected);
}
