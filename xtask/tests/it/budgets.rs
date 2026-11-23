//! The budget check (ADR 0061; TESTING.md, Time budgets): `xtask budgets`
//! reads each job's last 20 runs in its own tier from the results store,
//! compares their median with the Time budgets table and opens or updates one
//! `needs-triage` issue per breached suite or job, with the 20 durations. It
//! never fails a run.

use rstest::rstest;
use serde_json::{Value, json};

use crate::github::{GitHub, Request};
use crate::store::{Attempt, STORE, job, with, write_store};
use crate::workspace::{Outcome, Workspace, finding};

const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";
const NEEDS_TRIAGE: &str =
    "/repos/goodfetch/goodfetch/issues?labels=needs-triage&state=all&per_page=100&page=1";

const PULL_REQUEST: &str = ".github/workflows/pull-request.yml";
const RELEASE_SUITE: &str = ".github/workflows/release-suite.yml";

/// The file of TESTING.md's section that holds the table of time budgets.
const BUDGETS_FILE: &str = "docs/testing/suites-commands-and-time-budgets.md";

/// The repository's own table, so a reworded table of time budgets fails
/// these tests rather than leaving the check with nothing to compare.
const TABLE: &str = include_str!("../../../docs/testing/suites-commands-and-time-budgets.md");

const HOSTED: &[&str] = &["ubuntu-24.04"];
const SELF_HOSTED: &[&str] = &["self-hosted", "macOS"];

/// A finished, successful job named `name` that ran `minutes` from
/// `started_at`.
fn named_job(name: &str, labels: &[&str], minutes: u64, started_at: &str) -> Value {
    let started: jiff::Timestamp = started_at.parse().unwrap();
    let completed = started + jiff::SignedDuration::from_secs(i64::try_from(minutes * 60).unwrap());
    with(
        with(job(labels, minutes * 60, started_at), "job", json!(name)),
        "completed_at",
        json!(completed.to_string()),
    )
}

/// The start of run `run`: one a day from 2026-09-01, 08:00 UTC.
fn started_at(run: u64) -> &'static str {
    let day = jiff::civil::date(2026, 9, 1)
        .checked_add(jiff::Span::new().days(i64::try_from(run).unwrap()))
        .unwrap();
    Box::leak(format!("{day}T08:00:00Z").into_boxed_str())
}

/// Run `run` of `workflow`, started by `event`, with `jobs`.
fn run_of(workflow: &'static str, event: &'static str, run: u64, jobs: Vec<Value>) -> Attempt {
    let mut attempt = Attempt::new(run, started_at(run));
    attempt.workflow = workflow;
    attempt.event = event;
    attempt.jobs = jobs;
    attempt
}

/// Runs 1 to `count` of the Release suite, each with one hosted cell
/// `linux-x64-gnu` that ran `minutes(run)`.
fn release_runs(count: u64, minutes: impl Fn(u64) -> u64) -> Vec<Attempt> {
    (1..=count)
        .map(|run| {
            run_of(
                RELEASE_SUITE,
                "workflow_dispatch",
                run,
                vec![named_job(
                    "linux-x64-gnu",
                    HOSTED,
                    minutes(run),
                    started_at(run),
                )],
            )
        })
        .collect()
}

/// GitHub with `issues` labelled `needs-triage`.
fn github(issues: &str) -> GitHub {
    GitHub::serving(&[
        (NEEDS_TRIAGE, issues),
        (&format!("POST {ISSUES}"), r#"{ "number": 41 }"#),
        (&format!("PATCH {ISSUES}/7"), r#"{ "number": 7 }"#),
    ])
}

fn budgets(workspace: &Workspace, github: &GitHub) -> Outcome {
    workspace.xtask(&[
        "budgets",
        "--store",
        STORE,
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
    ])
}

fn workspace_with(attempts: &[Attempt]) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(BUDGETS_FILE, TABLE.as_bytes());
    write_store(&workspace, attempts);
    workspace
}

fn json_body(request: &Request) -> Value {
    serde_json::from_str(&request.body).unwrap()
}

/// The marker line of each issue body the check wrote.
fn markers(writes: &[Request]) -> Vec<String> {
    writes
        .iter()
        .map(|request| {
            json_body(request)["body"]
                .as_str()
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

/// The runs an issue body lists, from the first cell of each row of its
/// table of durations.
fn listed_runs(body: &str) -> Vec<u64> {
    body.lines()
        .filter_map(|line| line.strip_prefix("| ")?.split(' ').next()?.parse().ok())
        .collect()
}

#[test]
fn job_over_its_budget_opens_one_needs_triage_issue_with_its_last_20_durations() {
    let workspace = workspace_with(&release_runs(21, |_| 50));
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let writes = github.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].method, "POST");
    assert_eq!(writes[0].path, ISSUES);
    let body = json_body(&writes[0]);
    assert_eq!(body["labels"], json!(["needs-triage"]));
    assert_eq!(
        markers(&writes),
        vec![
            "<!-- goodfetch-results: needs-triage time-budget job release-suite.yml linux-x64-gnu -->"
        ]
    );
    assert_eq!(
        listed_runs(body["body"].as_str().unwrap()),
        (2..=21).rev().collect::<Vec<u64>>()
    );
}

#[test]
fn second_breach_updates_its_open_issue() {
    let workspace = workspace_with(&release_runs(20, |_| 50));
    let github = github(
        r#"[ { "number": 7, "state": "open", "closed_at": null,
               "body": "<!-- goodfetch-results: needs-triage time-budget job release-suite.yml linux-x64-gnu -->\nan earlier breach" } ]"#,
    );

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let writes = github.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].method, "PATCH");
    assert_eq!(writes[0].path, format!("{ISSUES}/7"));
    assert_eq!(
        listed_runs(json_body(&writes[0])["body"].as_str().unwrap()).len(),
        20
    );
}

/// The budget is a median: half the runs may pass it.
#[rstest]
#[case::every_run_within(|_| 40)]
#[case::median_at_the_budget(|run| if run % 2 == 0 { 44 } else { 46 })]
#[case::only_the_runs_before_the_last_20_over(|run| if run <= 10 { 90 } else { 40 })]
fn job_within_its_budget_opens_nothing(#[case] minutes: fn(u64) -> u64) {
    let workspace = workspace_with(&release_runs(30, minutes));
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(github.writes(), vec![]);
}

#[test]
fn median_past_the_budget_opens_an_issue() {
    let workspace = workspace_with(&release_runs(20, |run| if run <= 11 { 46 } else { 44 }));
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(github.writes().len(), 1);
}

/// Cells on the owner's machine keep the 90-minute job cap and stay out of
/// the Release budgets (TESTING.md, Time budgets).
#[test]
fn release_cell_on_the_owners_machine_is_kept_out_of_the_budgets() {
    let attempts: Vec<Attempt> = (1..=20)
        .map(|run| {
            run_of(
                RELEASE_SUITE,
                "workflow_dispatch",
                run,
                vec![named_job("macos-arm64", SELF_HOSTED, 80, started_at(run))],
            )
        })
        .collect();
    let workspace = workspace_with(&attempts);
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(github.writes(), vec![]);
}

/// The pull-request tier's hosted jobs run on GitHub's standard runners
/// (ADR 0135's amendment of 2026-11-22), where its budgets are measured.
fn pull_request_runs(jobs: impl Fn(&'static str) -> Vec<Value>) -> Vec<Attempt> {
    (1..=20)
        .map(|run| run_of(PULL_REQUEST, "pull_request", run, jobs(started_at(run))))
        .collect()
}

#[rstest]
#[case::quick_suite_job_past_10_minutes(
    |start| vec![named_job("quick (quick)", HOSTED, 11, start)],
    "time-budget job pull-request.yml quick (quick)",
)]
#[case::pre_merge_shard_past_15_minutes(
    |start| vec![named_job("pre-merge shard (1)", HOSTED, 16, start)],
    "time-budget job pull-request.yml pre-merge shard (1)",
)]
#[case::all_green_past_20_minutes_from_the_first_jobs_start(
    |start| vec![
        named_job("pre-merge build", HOSTED, 14, start),
        named_job("all green", HOSTED, 7, &later(start, 14)),
    ],
    "time-budget suite Pre-merge",
)]
fn pull_request_tier_over_its_budget_opens_its_issue(
    #[case] jobs: fn(&'static str) -> Vec<Value>,
    #[case] key: &str,
) {
    let workspace = workspace_with(&pull_request_runs(jobs));
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        markers(&github.writes()),
        vec![format!("<!-- goodfetch-results: needs-triage {key} -->")]
    );
}

#[test]
fn release_group_past_60_minutes_opens_its_issue() {
    let attempts: Vec<Attempt> = (1..=20)
        .map(|run| {
            let start = started_at(run);
            run_of(
                RELEASE_SUITE,
                "workflow_dispatch",
                run,
                vec![
                    named_job("plan", HOSTED, 1, start),
                    named_job("linux-x64-gnu", HOSTED, 40, &later(start, 1)),
                    named_job("windows-x64", HOSTED, 40, &later(start, 25)),
                ],
            )
        })
        .collect();
    let workspace = workspace_with(&attempts);
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        markers(&github.writes()),
        vec!["<!-- goodfetch-results: needs-triage time-budget suite Release -->"]
    );
}

/// `start` plus `minutes`, RFC 3339.
fn later(start: &str, minutes: i64) -> String {
    let start: jiff::Timestamp = start.parse().unwrap();
    (start + jiff::SignedDuration::from_mins(minutes)).to_string()
}

#[test]
fn tier_without_a_budget_row_is_reported_and_fails_nothing() {
    let workspace = workspace_with(&release_runs(20, |_| 50));
    workspace.edit(
        BUDGETS_FILE,
        "| Release | Merge queue | ≤ 60 min per group, each job ≤ 45 min | 90 min per job, 120 min for the queue |\n",
        "",
    );
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.notes(),
        vec![finding("missing-budget", "Release, Merge queue")]
    );
    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(github.writes(), vec![]);
}

/// The repository's table names every budget the tiers need, so the check
/// reports none missing.
#[test]
fn the_table_in_testing_md_has_every_budget_the_tiers_need() {
    let workspace = workspace_with(&[]);
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
}

/// A run whose last job ends before its first starts has no group time: it
/// is left out with a note, never counted as 0 s, which would pull the
/// median under its budget.
#[test]
fn run_ending_before_it_starts_is_left_out_of_the_group_median() {
    let attempts: Vec<Attempt> = (1..=20)
        .map(|run| {
            let start = started_at(run);
            let jobs = if run <= 10 {
                vec![
                    named_job("plan", HOSTED, 1, start),
                    named_job("linux-x64-gnu", HOSTED, 40, &later(start, 1)),
                    named_job("windows-x64", HOSTED, 40, &later(start, 30)),
                ]
            } else {
                vec![with(
                    named_job("linux-x64-gnu", HOSTED, 40, start),
                    "completed_at",
                    json!(later(start, -5)),
                )]
            };
            run_of(RELEASE_SUITE, "workflow_dispatch", run, jobs)
        })
        .collect();
    let workspace = workspace_with(&attempts);
    let github = github("[]");

    let outcome = budgets(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.notes(),
        (11..=20)
            .map(|run| finding("negative-span", &format!("Release, run {run}")))
            .collect::<Vec<_>>()
    );
    let writes = github.writes();
    assert_eq!(
        markers(&writes),
        vec!["<!-- goodfetch-results: needs-triage time-budget suite Release -->"]
    );
    assert_eq!(
        listed_runs(json_body(&writes[0])["body"].as_str().unwrap()),
        (1..=10).rev().collect::<Vec<u64>>()
    );
}
