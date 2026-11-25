//! The Open Source Security Foundation's Scorecard as a nightly Live check that blocks nothing (ADRs 0032
//! and 0064): `xtask results scorecard` reads Scorecard's JSON results and
//! holds each check to its floor in `ci/scorecard.toml`. A check under its
//! floor fails the Live check; one Scorecard could not run (score -1) above
//! a floor of -1 makes it inconclusive (exit 75), as do missing results; and
//! a check with no floor only reports.
//! Covers: story:1-75

use rstest::rstest;

use crate::workspace::Workspace;

/// The floors: a check scored -1 by design (no packaging workflow) is
/// accepted at -1.
const FLOORS: &str = "[floor]\n\
     Binary-Artifacts = 9\n\
     Token-Permissions = 10\n\
     Packaging = -1\n\
     Fuzzing = 10\n";

fn results(scores: &[(&str, i8)]) -> String {
    let checks: Vec<String> = scores
        .iter()
        .map(|(name, score)| {
            format!(
                r#"{{ "details": null, "score": {score}, "reason": "{name} reason", "name": "{name}",
                     "documentation": {{ "url": "https://example.com", "short": "" }} }}"#
            )
        })
        .collect();
    format!(
        r#"{{ "date": "2026-11-25T00:00:00Z",
             "repo": {{ "name": "github.com/goodfetch/goodfetch", "commit": "c1" }},
             "scorecard": {{ "version": "v5.6.0", "commit": "s1" }},
             "score": 7.1, "checks": [{}], "metadata": null }}"#,
        checks.join(", ")
    )
}

#[rstest]
#[case::every_check_at_its_floor(
    &[("Binary-Artifacts", 9), ("Token-Permissions", 10), ("Packaging", -1), ("Fuzzing", 10)],
    0,
    None
)]
#[case::a_check_without_a_floor_only_reports(
    &[("Binary-Artifacts", 10), ("Token-Permissions", 10), ("Packaging", 10), ("Fuzzing", 10), ("Vulnerabilities", 0)],
    0,
    None
)]
#[case::a_check_under_its_floor(
    &[("Binary-Artifacts", 9), ("Token-Permissions", 8), ("Packaging", -1), ("Fuzzing", 10)],
    1,
    Some("Token-Permissions")
)]
#[case::a_check_that_could_not_run(
    &[("Binary-Artifacts", 9), ("Token-Permissions", 10), ("Packaging", -1), ("Fuzzing", -1)],
    75,
    Some("Fuzzing")
)]
#[case::a_check_under_its_floor_beside_one_that_could_not_run(
    &[("Binary-Artifacts", 2), ("Token-Permissions", 10), ("Packaging", -1), ("Fuzzing", -1)],
    1,
    Some("Binary-Artifacts")
)]
fn each_check_is_held_to_its_floor(
    #[case] scores: &[(&str, i8)],
    #[case] code: i32,
    #[case] named: Option<&str>,
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("ci/scorecard.toml", FLOORS.as_bytes());
    workspace.write("scorecard.json", results(scores).as_bytes());

    let outcome = workspace.xtask(&["results", "scorecard", "--results", "scorecard.json"]);

    assert_eq!(outcome.code, code, "{}{}", outcome.stdout, outcome.stderr);
    if let Some(check) = named {
        assert!(outcome.stdout.contains(check), "{}", outcome.stdout);
    }
}

/// Scorecard's step failed before it wrote its results, as when GitHub's API
/// was unavailable.
#[test]
fn missing_results_are_inconclusive() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("ci/scorecard.toml", FLOORS.as_bytes());

    let outcome = workspace.xtask(&["results", "scorecard", "--results", "scorecard.json"]);

    assert_eq!(outcome.code, 75, "{}", outcome.stderr);
}
