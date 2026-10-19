//! The drift check (ADR 0071), run with the daily test-health job: `main`'s
//! latest counted figures in the results store against the last release's.
//! A figure up more than 10% since that release, or a resident-memory peak
//! within 10% of its ceiling, opens or updates one `needs-triage` issue per
//! figure; before the first release, nothing is compared with a release.
//! The figures reach the store in each job's JUnit artifact, as
//! `xtask perf compare --save` writes them.

use serde_json::{Value, json};

use crate::github::{GitHub, Request};
use crate::store::{Attempt, STORE, write_store};
use crate::workspace::{Outcome, Workspace};

const RELEASE_COMMIT: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const MAIN_COMMIT: &str = "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";
const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";
const FRONTIER: &str = "goodfetch-frontier/instructions::frontier::push";

/// `xtask perf compare --save`'s file for a head that counted `figures`,
/// `(Callgrind or DHAT key, value)`, of the frontier's benchmark.
fn saved_figures(figures: &[(&str, u64)]) -> Vec<u8> {
    let workspace = Workspace::from_fixture("empty");
    let mut profiles: Vec<Value> = Vec::new();
    for (key, value) in figures {
        let tool = if *key == "Ir" { "Callgrind" } else { "DHAT" };
        profiles.push(json!({
            "data": { "parts": [], "total": { "metrics": { *key: { "values": { "new": value } } }, "regressions": [] } },
            "duration_ns": 1, "process_ns": 1, "started_at": "2026-10-19T08:00:00Z", "tool": tool,
        }));
    }
    let line = json!({
        "baselines": [null, null], "benchmark_exe": "x", "benchmark_file": "x", "duration_ns": 1,
        "function_name": "push", "group": "frontier", "kind": "LibraryBenchmark",
        "module_path": "instructions::frontier::push", "output_dir": "x",
        "package_dir": "crates/foxhound/goodfetch-frontier", "profiles": profiles,
        "project_root": "/work", "started_at": "2026-10-19T08:00:00Z", "version": "7",
    });
    workspace.write("head.jsonl", line.to_string().as_bytes());
    let outcome = workspace.xtask(&[
        "perf",
        "compare",
        "--gungraun",
        "head.jsonl",
        "--save",
        "perf-figures.json",
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    workspace.read("perf-figures.json").unwrap()
}

/// A Release suite run of `commit` on `main`, whose linux-x64-gnu cell
/// saved `figures`.
fn release_suite_run(
    run: u64,
    started_at: &'static str,
    commit: &'static str,
    figures: Vec<u8>,
) -> Attempt {
    let mut attempt = Attempt::new(run, started_at);
    attempt.workflow = ".github/workflows/release-suite.yml";
    attempt.event = "workflow_dispatch";
    attempt.commit = commit;
    attempt.kept = vec![(
        "junit-release-dispatch-linux-x64-gnu-1/perf-figures.json".to_owned(),
        figures,
    )];
    attempt
}

/// GitHub with `release` as the latest release, at `RELEASE_COMMIT`, and no
/// `needs-triage` issue yet.
fn github(release: Option<&str>) -> GitHub {
    let mut routes: Vec<(String, String)> = vec![
        (
            format!("{ISSUES}?labels=needs-triage&state=all&per_page=100&page=1"),
            "[]".to_owned(),
        ),
        (
            "/repos/goodfetch/goodfetch/milestones?state=open&per_page=100&page=1".to_owned(),
            r#"[ { "number": 1, "open_issues": 3, "due_on": null } ]"#.to_owned(),
        ),
        (format!("POST {ISSUES}"), r#"{ "number": 41 }"#.to_owned()),
    ];
    if let Some(tag) = release {
        routes.push((
            "/repos/goodfetch/goodfetch/releases/latest".to_owned(),
            format!(r#"{{ "tag_name": "{tag}" }}"#),
        ));
        routes.push((
            format!("/repos/goodfetch/goodfetch/commits/{tag}"),
            format!(r#"{{ "sha": "{RELEASE_COMMIT}" }}"#),
        ));
    }
    let routes: Vec<(&str, &str)> = routes
        .iter()
        .map(|(path, body)| (path.as_str(), body.as_str()))
        .collect();
    GitHub::serving(&routes)
}

fn drift(workspace: &Workspace, github: &GitHub) -> Outcome {
    workspace.xtask(&[
        "perf",
        "drift",
        "--store",
        STORE,
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
    ])
}

/// The marker line of each issue the check opened.
fn opened(writes: &[Request]) -> Vec<String> {
    writes
        .iter()
        .filter(|request| request.method == "POST" && request.path == ISSUES)
        .map(|request| {
            let body: Value = serde_json::from_str(&request.body).unwrap();
            assert_eq!(body["labels"], json!(["needs-triage"]));
            body["body"]
                .as_str()
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn figure_up_more_than_10_percent_since_the_last_release_opens_one_issue() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            release_suite_run(
                1,
                "2026-10-01T08:00:00Z",
                RELEASE_COMMIT,
                saved_figures(&[("Ir", 1_000_000), ("TotalBytes", 50_000)]),
            ),
            release_suite_run(
                2,
                "2026-10-10T08:00:00Z",
                MAIN_COMMIT,
                saved_figures(&[("Ir", 1_200_000), ("TotalBytes", 51_000)]),
            ),
            release_suite_run(
                3,
                "2026-10-18T08:00:00Z",
                MAIN_COMMIT,
                saved_figures(&[("Ir", 1_101_000), ("TotalBytes", 55_000)]),
            ),
        ],
    );
    let github = github(Some("v0.1.0"));

    let outcome = drift(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        opened(&github.writes()),
        vec![format!(
            "<!-- goodfetch-results: needs-triage perf-drift linux-x64 {FRONTIER} instructions -->"
        )]
    );
}

/// A run records only what `xtask affected` selected for its commit, so
/// each figure's latest value comes from the last run that measured it.
fn instructions(subject: &str, value: u64) -> Vec<u8> {
    json!({ "figures": [{ "subject": subject, "figure": "instructions", "value": value }] })
        .to_string()
        .into_bytes()
}

const PARSER: &str = "goodfetch-parser/instructions::parse";

#[test]
fn figure_drifted_before_a_run_that_measured_only_others_still_opens_one_issue() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            release_suite_run(
                1,
                "2026-10-01T08:00:00Z",
                RELEASE_COMMIT,
                instructions(FRONTIER, 1_000_000),
            ),
            release_suite_run(
                2,
                "2026-10-10T08:00:00Z",
                MAIN_COMMIT,
                instructions(FRONTIER, 1_200_000),
            ),
            release_suite_run(3, "2026-10-18T08:00:00Z", "c3c3", instructions(PARSER, 500)),
        ],
    );
    let github = github(Some("v0.1.0"));

    let outcome = drift(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        opened(&github.writes()),
        vec![format!(
            "<!-- goodfetch-results: needs-triage perf-drift linux-x64 {FRONTIER} instructions -->"
        )]
    );
}

/// The release's figure of a benchmark its commit did not measure is the
/// last one `main` recorded before it.
#[test]
fn release_figure_its_commit_did_not_measure_is_mains_last_before_it() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            release_suite_run(
                1,
                "2026-09-20T08:00:00Z",
                "c0c0",
                instructions(FRONTIER, 1_000_000),
            ),
            release_suite_run(
                2,
                "2026-10-01T08:00:00Z",
                RELEASE_COMMIT,
                instructions(PARSER, 500),
            ),
            release_suite_run(
                3,
                "2026-10-18T08:00:00Z",
                MAIN_COMMIT,
                instructions(FRONTIER, 1_200_000),
            ),
        ],
    );
    let github = github(Some("v0.1.0"));

    let outcome = drift(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        opened(&github.writes()),
        vec![format!(
            "<!-- goodfetch-results: needs-triage perf-drift linux-x64 {FRONTIER} instructions -->"
        )]
    );
}

/// A Release run on `main` may test an older commit than its head: the
/// release's figures are those of the run that tested the release's commit.
#[test]
fn release_commit_is_the_one_a_run_tested_not_its_head() {
    let workspace = Workspace::from_fixture("empty");
    let mut release = release_suite_run(
        1,
        "2026-10-05T08:00:00Z",
        RELEASE_COMMIT,
        instructions(FRONTIER, 1_000_000),
    );
    release.head = Some(MAIN_COMMIT);
    write_store(
        &workspace,
        &[
            release,
            release_suite_run(
                2,
                "2026-10-18T08:00:00Z",
                MAIN_COMMIT,
                instructions(FRONTIER, 1_200_000),
            ),
        ],
    );
    let github = github(Some("v0.1.0"));

    let outcome = drift(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(opened(&github.writes()).len(), 1);
}

/// The issue's change rounds away from zero, as the comparison table's does,
/// so it is never under the measured one.
#[test]
fn drift_issue_states_its_change_rounded_away_from_zero() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            release_suite_run(
                1,
                "2026-10-01T08:00:00Z",
                RELEASE_COMMIT,
                instructions(FRONTIER, 1_000_000),
            ),
            release_suite_run(
                2,
                "2026-10-18T08:00:00Z",
                MAIN_COMMIT,
                instructions(FRONTIER, 1_100_050),
            ),
        ],
    );
    let github = github(Some("v0.1.0"));

    let outcome = drift(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let writes = github.writes();
    let body: Value = serde_json::from_str(&writes[0].body).unwrap();
    assert!(
        body["body"].as_str().unwrap().contains("| +10.01% |"),
        "{}",
        body["body"]
    );
}

#[test]
fn before_the_first_release_nothing_is_compared() {
    let workspace = Workspace::from_fixture("empty");
    write_store(
        &workspace,
        &[
            release_suite_run(
                1,
                "2026-10-01T08:00:00Z",
                RELEASE_COMMIT,
                saved_figures(&[("Ir", 1_000_000)]),
            ),
            release_suite_run(
                2,
                "2026-10-18T08:00:00Z",
                MAIN_COMMIT,
                saved_figures(&[("Ir", 2_000_000)]),
            ),
        ],
    );
    let github = github(None);

    let outcome = drift(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(github.writes(), vec![]);
}

#[test]
fn resident_memory_peak_within_10_percent_of_its_ceiling_opens_one_issue() {
    let workspace = Workspace::from_fixture("empty");
    let figures = json!({ "figures": [
        { "subject": "collie/idle-attached-engine", "figure": "resident-memory", "value": 7_600_000, "ceiling": 8_388_608 },
        { "subject": "collie/idle-daemon", "figure": "resident-memory", "value": 12_000_000, "ceiling": 16_777_216 },
    ] });
    write_store(
        &workspace,
        &[release_suite_run(
            1,
            "2026-10-18T08:00:00Z",
            MAIN_COMMIT,
            figures.to_string().into_bytes(),
        )],
    );
    let github = github(None);

    let outcome = drift(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        opened(&github.writes()),
        vec![
            "<!-- goodfetch-results: needs-triage perf-ceiling linux-x64 collie/idle-attached-engine resident-memory -->"
                .to_owned()
        ]
    );
}
