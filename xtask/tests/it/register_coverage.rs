//! The weekly coverage job's comparison (ADR 0072; TESTING.md, Quality
//! register): `xtask register coverage` adds up the job's three lcov reports,
//! compares the project's line coverage with the `coverage.project` row within
//! 0.5 %, and opens or updates one `needs-triage` issue when it falls, as
//! `xtask budgets` does. It never fails a run over a figure.

use rstest::rstest;
use serde_json::Value;

use crate::github::{GitHub, Request};
use crate::workspace::{Outcome, Workspace, finding};

const TESTING: &str = "TESTING.md";
const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";
const NEEDS_TRIAGE: &str =
    "/repos/goodfetch/goodfetch/issues?labels=needs-triage&state=all&per_page=100&page=1";

/// A register whose `coverage.project` row holds `today`.
fn register(today: &str) -> String {
    format!(
        "\
# Testing

## Quality register

### Enforced

| Dimension | Rule | Checked by | Runs at |
|---|---|---|---|
| Rust lints | 0 warnings | `cargo clippy` | merge gate |

### Measured, not yet enforced

| Row | Metric, per unit | Contract | Today | Benchmark | Direction |
|---|---|---|---|---|---|
| `F1.parses` | HTML parses per page | ≤ 1 | after #1583 | B1 | must not grow |
| `coverage.project` | project line coverage | none: a ratchet | {today} | `just coverage all` | must not fall (0.5 %) |

### Exceptions

| ID | Rule | Path | Reason | Owner | Expires | Removed by |
|---|---|---|---|---|---|---|
| X1 | `F1.parses` | F1 | a reason | The owner | 2027-01-30 | #1577 |

### Rules
"
    )
}

/// An lcov report of two source files, whose counts add up to `hit` of `lines`.
fn lcov(hit: u32, lines: u32) -> String {
    format!(
        "TN:\nSF:a.rs\nDA:1,1\nLF:1\nLH:1\nend_of_record\n\
         TN:\nSF:b.rs\nDA:1,0\nLF:{}\nLH:{}\nend_of_record\n",
        lines - 1,
        hit - 1
    )
}

/// A workspace whose row holds `today` and whose three reports hold `hit` of
/// `lines` lines between them, so a line counted in only one report changes
/// the figure.
fn workspace(today: &str, hit: u32, lines: u32) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(TESTING, register(today).as_bytes());
    workspace.write(
        "target/coverage/rust/lcov.info",
        lcov(hit - 2, lines - 2).as_bytes(),
    );
    workspace.write("target/coverage/vitest/lcov.info", lcov(1, 1).as_bytes());
    workspace.write("target/coverage/python/lcov.info", lcov(1, 1).as_bytes());
    workspace
}

/// GitHub with `issues` labelled `needs-triage`.
fn github(issues: &str) -> GitHub {
    GitHub::serving(&[
        (NEEDS_TRIAGE, issues),
        (&format!("POST {ISSUES}"), r#"{ "number": 41 }"#),
        (&format!("PATCH {ISSUES}/7"), r#"{ "number": 7 }"#),
    ])
}

fn coverage(workspace: &Workspace, github: &GitHub, more: &[&str]) -> Outcome {
    let mut arguments = vec![
        "register",
        "coverage",
        "--reports",
        "target/coverage",
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
    ];
    arguments.extend_from_slice(more);
    workspace.xtask(&arguments)
}

fn json_body(request: &Request) -> Value {
    serde_json::from_str(&request.body).unwrap()
}

#[test]
fn a_figure_below_the_row_opens_one_needs_triage_issue() {
    let workspace = workspace("80.00 %", 70, 100);
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let writes = github.writes();
    assert_eq!(writes.len(), 1, "{}", outcome.stdout);
    assert_eq!(writes[0].method, "POST");
    let body = json_body(&writes[0]);
    assert_eq!(body["labels"], serde_json::json!(["needs-triage"]));
    let text = body["body"].as_str().unwrap();
    assert!(text.contains("70.00 %"), "{text}");
    assert!(text.contains("80.00 %"), "{text}");
}

/// 0.5 % of 80 is 0.4 points: 79.7 % stays within it, 79.5 % does not.
#[rstest]
#[case::within_the_tolerance(797, false)]
#[case::past_the_tolerance(795, true)]
fn a_fall_counts_only_past_the_tolerance(#[case] hit: u32, #[case] opens: bool) {
    let workspace = workspace("80.00 %", hit, 1000);
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(!github.writes().is_empty(), opens, "{}", outcome.stdout);
}

#[test]
fn a_figure_above_the_row_opens_nothing() {
    let workspace = workspace("80.00 %", 90, 100);
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), vec![]);
    assert!(github.writes().is_empty(), "{}", outcome.stdout);
}

#[test]
fn a_second_fall_updates_the_open_issue() {
    let workspace = workspace("80.00 %", 70, 100);
    let github = github(
        r#"[ { "number": 7, "state": "open", "closed_at": null, "pull_request": null,
               "body": "<!-- goodfetch-results: needs-triage coverage.project -->\nan older fall" } ]"#,
    );

    let outcome = coverage(&workspace, &github, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let writes = github.writes();
    assert_eq!(writes.len(), 1, "{}", outcome.stdout);
    assert_eq!(writes[0].method, "PATCH");
    assert_eq!(writes[0].path, format!("{ISSUES}/7"));
}

#[test]
fn a_row_with_no_figure_opens_one_issue_with_the_first_figure_and_writes_nothing() {
    let workspace = workspace("the next weekly coverage job", 70, 100);
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let writes = github.writes();
    assert_eq!(writes.len(), 1, "{}", outcome.stdout);
    let body = json_body(&writes[0]);
    assert_eq!(body["labels"], serde_json::json!(["needs-triage"]));
    let text = body["body"].as_str().unwrap();
    assert!(text.contains("70.00 %"), "{text}");
    assert!(text.contains("--write"), "{text}");
    assert_eq!(
        workspace.read_text(TESTING),
        register("the next weekly coverage job")
    );
}

#[rstest]
#[case::the_first_figure("the next weekly coverage job", 70, 100, "70.00 %")]
#[case::a_rise("80.00 %", 90, 100, "90.00 %")]
#[case::a_small_rise("80.00 %", 801, 1000, "80.10 %")]
fn write_puts_the_figure_in_the_row_and_opens_nothing(
    #[case] today: &str,
    #[case] hit: u32,
    #[case] lines: u32,
    #[case] written: &str,
) {
    let workspace = workspace(today, hit, lines);
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &["--write"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(github.writes().is_empty(), "{}", outcome.stdout);
    assert_eq!(workspace.read_text(TESTING), register(written));
}

#[rstest]
#[case::a_fall(70, 100, true)]
#[case::a_fall_within_the_tolerance(797, 1000, false)]
fn write_never_lowers_the_row(#[case] hit: u32, #[case] lines: u32, #[case] opens: bool) {
    let workspace = workspace("80.00 %", hit, lines);
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &["--write"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(!github.writes().is_empty(), opens, "{}", outcome.stdout);
    assert_eq!(workspace.read_text(TESTING), register("80.00 %"));
}

#[test]
fn a_written_figure_is_what_the_next_run_compares_with() {
    let workspace = workspace("the next weekly coverage job", 70, 100);
    let github = github("[]");
    let written = coverage(&workspace, &github, &["--write"]);
    assert_eq!(written.code, 0, "{}", written.stderr);
    assert!(github.writes().is_empty(), "{}", written.stdout);

    // 60 of 100 lines between the three reports.
    workspace.write("target/coverage/rust/lcov.info", lcov(58, 98).as_bytes());
    let outcome = coverage(&workspace, &github, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(github.writes().len(), 1, "{}", outcome.stdout);
}

#[test]
fn a_report_that_is_missing_fails_with_no_issue_and_no_figure() {
    let workspace = workspace("80.00 %", 90, 100);
    workspace.remove("target/coverage/python/lcov.info");
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &["--write"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("coverage-report-missing", "Python")]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(github.writes().is_empty(), "{}", outcome.stdout);
    assert_eq!(workspace.read_text(TESTING), register("80.00 %"));
}

#[rstest]
#[case::a_count_that_is_no_number("LF:many\n")]
#[case::no_lines_at_all("TN:\n")]
fn a_report_it_cannot_read_fails_with_no_issue(#[case] report: &str) {
    let workspace = workspace("80.00 %", 90, 100);
    workspace.write("target/coverage/vitest/lcov.info", report.as_bytes());
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &[]);

    assert_eq!(
        outcome.findings(),
        vec![finding("coverage-report-unreadable", "TypeScript")]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(github.writes().is_empty(), "{}", outcome.stdout);
}

#[test]
fn a_register_with_no_row_fails_with_no_issue() {
    let workspace = workspace("80.00 %", 70, 100);
    workspace.edit(TESTING, "`coverage.project`", "`coverage.other`");
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &[]);

    assert_eq!(
        outcome.findings(),
        vec![finding("coverage-row-missing", "coverage.project")]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(github.writes().is_empty(), "{}", outcome.stdout);
}

/// The repository's own register, so a reworded row fails here rather than
/// on the job's next run. Every line ran, so no recorded figure can be a fall.
#[test]
fn the_row_of_this_repository_can_be_read_and_written() {
    let workspace = workspace("80.00 %", 100, 100);
    workspace.write(TESTING, include_str!("../../../TESTING.md").as_bytes());
    let github = github("[]");

    let outcome = coverage(&workspace, &github, &["--write"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        workspace.read_text(TESTING).lines().any(
            |line| line.starts_with("| `coverage.project` |") && line.contains("| 100.00 % |")
        ),
        "{}",
        outcome.stdout
    );
}
