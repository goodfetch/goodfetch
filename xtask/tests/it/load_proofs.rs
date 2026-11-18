//! An issue's acceptance criteria ask no proof under artificial load
//! (TESTING.md, A flaky test found after merge): no `stress-ng`, no busy
//! loops, no count of runs in a row. Issues come from a local stand-in for
//! GitHub.

use rstest::rstest;
use serde_json::{Value, json};

use crate::github::GitHub;
use crate::workspace::{Outcome, Workspace, finding};

const ISSUE: &str = "/repos/goodfetch/goodfetch/issues/7";
const OPEN: &str = "/repos/goodfetch/goodfetch/issues?state=open&per_page=100&page=1";

fn issue(number: u64, body: &str) -> Value {
    json!({ "number": number, "title": "A flaky test", "body": body })
}

/// A body whose acceptance criteria hold `criterion`.
fn asking(criterion: &str) -> String {
    format!(
        "## What happens\n\nA test fails under load.\n\n## Acceptance criteria\n\n- [ ] The cause is named.\n- [ ] {criterion}\n"
    )
}

fn check(body: &str) -> Outcome {
    let issue = issue(7, body).to_string();
    let github = GitHub::serving(&[(ISSUE, &issue)]);
    Workspace::from_fixture("empty").xtask_with_env(
        &["load-proofs", "check", "7", "--api-url", github.url()],
        &[("GITHUB_TOKEN", "a-read-token".as_ref())],
    )
}

#[rstest]
#[case::stress_ng("It passes 50 runs under `stress-ng --cpu 32`.")]
#[case::busy_loops("It passes with 32 busy loops running beside it.")]
#[case::runs_in_a_row("It passes 50 runs in a row.")]
#[case::passing_runs_in_a_row("It shows 20 passing runs in a row.")]
#[case::times_in_a_row("The test passes fifty times in a row.")]
#[case::consecutive_runs("It passes 100 consecutive runs.")]
#[case::repeat("It passes `just repeat 50 --test nextest=crate::it a_test`.")]
fn an_acceptance_criterion_asking_a_proof_under_load_fails(#[case] criterion: &str) {
    let outcome = check(&asking(criterion));

    assert_eq!(
        outcome.findings(),
        vec![finding("load-proof", "#7 line 8")],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

/// What TESTING.md asks, and words that only look alike: a product's own
/// count of failed runs or a count under ten, and a proof under load named outside the
/// acceptance criteria, as the description of what happened.
#[rstest]
#[case::the_named_cause(
    "It passes one normal run, and the pull request names the cause it removed."
)]
#[case::a_product_count("Three failed runs in a row auto-pause a Schedule.")]
#[case::a_small_count(
    "The fetcher retries three times in a row, then pauses after 3 runs in a row that fail."
)]
#[case::one_repeat(
    "`just repeat 1 --test nextest=crate::it a_test` run twice compiles nothing the second time."
)]
#[case::consecutive_redirects("More than five consecutive redirects make robots.txt unavailable.")]
fn an_acceptance_criterion_asking_what_testing_md_asks_passes(#[case] criterion: &str) {
    let body = format!(
        "## What happened\n\nWith `stress-ng` and 32 busy loops, 50 runs in a row failed 25 tests.\n\n{}",
        asking(criterion)
    );

    let outcome = check(&body);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// A task-list item asks for something wherever it stands, and a section
/// headed in bold is a section.
#[test]
fn a_task_list_item_outside_the_acceptance_criteria_is_checked() {
    let body = "**Desired behavior**\n\nIt passes.\n\n**Notes**\n\n- [ ] Show 50 runs in a row.\n";

    let outcome = check(body);

    assert_eq!(
        outcome.findings(),
        vec![finding("load-proof", "#7 line 7")],
        "{}",
        outcome.stdout
    );
}

#[test]
fn the_sweep_names_each_open_issue_that_asks_and_skips_pull_requests() {
    let asks = asking("It passes 50 runs in a row.");
    let issues = json!([
        issue(7, &asks),
        issue(8, &asking("It passes one normal run.")),
        issue(9, ""),
        { "number": 10, "title": "A pull request", "body": asks, "pull_request": {} },
    ])
    .to_string();
    let github = GitHub::serving(&[(OPEN, &issues)]);

    let outcome = Workspace::from_fixture("empty").xtask_with_env(
        &["load-proofs", "sweep", "--api-url", github.url()],
        &[("GITHUB_TOKEN", "a-read-token".as_ref())],
    );

    assert_eq!(
        outcome.findings(),
        vec![finding("load-proof", "#7 line 8")],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[test]
fn a_pull_request_is_refused() {
    let pull_request =
        json!({ "number": 7, "title": "A pull request", "body": "", "pull_request": {} })
            .to_string();
    let github = GitHub::serving(&[(ISSUE, &pull_request)]);

    let outcome = Workspace::from_fixture("empty").xtask_with_env(
        &["load-proofs", "check", "7", "--api-url", github.url()],
        &[("GITHUB_TOKEN", "a-read-token".as_ref())],
    );

    assert_ne!(outcome.code, 0, "{}", outcome.stdout);
    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
}
