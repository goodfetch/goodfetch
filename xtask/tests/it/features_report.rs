//! `xtask features report` (ADRs 0072, 0075 and 0094): each Feature's
//! status at a milestone's gate, from the inventory, the declarations, the
//! quarantine list and `main`'s latest Release run in the results store:
//! tested, untested with its cause, excepted with its evidence kind, or
//! planned; each Product's view; and the milestone a flagged story is due.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::store::{Attempt, STORE, in_every_release_cell, test_run, with, write_store};
use crate::workspace::{Outcome, Workspace};

/// Where the fixture's runner listings sit, as `xtask journeys listings`
/// writes them.
const LISTINGS: &str = "listings";

/// The folder the report is written to.
const OUT: &str = "target/features-report";

/// One test run of `main`'s Release suite, of the commit `c<tree>`.
fn release_run(test: &str, outcome: &str, tree: &str) -> Value {
    let run = with(test_run(test, outcome, tree), "suite", json!("release"));
    with(run, "tier", json!("dispatch"))
}

/// A Release run on `main`, started at `started_at`, that tested `c<tree>`.
fn release_attempt(run: u64, started_at: &'static str, commit: &'static str) -> Attempt {
    Attempt {
        workflow: ".github/workflows/release-suite.yml",
        event: "workflow_dispatch",
        commit,
        ..Attempt::new(run, started_at)
    }
}

/// The Linux artifact of a Release run's cell.
const LINUX: &str = "junit-release-dispatch-linux-x64-gnu-1";

/// J-001@cli's result document, proved, as its Release cell keeps it.
fn j001_cli_kept() -> (String, Vec<u8>) {
    let document = json!({
        "kind": "journey",
        "journey_id": "J-001",
        "surface": "cli",
        "executable": { "product": "foxhound", "version": "0.1.0" },
        "platform": { "os": "linux", "arch": "x86_64" },
        "runtime": "rust",
        "steps": [{ "text": "Reach the goal", "outcome": "passed" }],
        "checks": [{ "text": "The goal is reached", "outcome": "passed" }],
        "tasks": [],
        "outcome": "proved",
    });
    (
        format!("{LINUX}/result-documents/J-001@cli.json"),
        serde_json::to_vec(&document).unwrap(),
    )
}

/// The fixture's store: an older Release run, a newer test-health run, and
/// `main`'s latest Release run, which the report reads.
fn fixture_store(workspace: &Workspace) {
    fixture_store_keeping(workspace, vec![j001_cli_kept()]);
}

/// The fixture's store, `main`'s latest Release run keeping `kept`.
fn fixture_store_keeping(workspace: &Workspace, kept: Vec<(String, Vec<u8>)>) {
    let older = Attempt {
        test_runs: vec![
            release_run("sample tests::fails", "passed", "0"),
            release_run("sample tests::never_ran", "passed", "0"),
        ],
        ..release_attempt(9, "2026-10-17T03:00:00Z", "c0")
    };
    // Every cell of the latest commit ran (ADR 0108's whole-cell rule).
    let mut runs = in_every_release_cell(&release_run("sample tests::passes", "passed", "1"));
    runs.extend([
        release_run("sample tests::fails", "failed", "1"),
        release_run("sample tests::crawls", "passed", "1"),
        release_run("sample::it reaches_the_goal", "passed", "1"),
        in_every_release_cell(&release_run(
            "sample::it journeys::j001::cli",
            "passed",
            "1",
        ))
        .swap_remove(0),
    ]);
    let latest = Attempt {
        test_runs: runs,
        kept,
        ..release_attempt(10, "2026-10-18T03:00:00Z", "c1")
    };
    let health = Attempt {
        test_runs: vec![test_run("sample tests::passes", "failed", "2")],
        ..Attempt::new(11, "2026-10-19T03:00:00Z")
    };
    write_store(workspace, &[older, latest, health]);
}

fn report(workspace: &Workspace, milestone: &str) -> (Outcome, Value) {
    let outcome = workspace.xtask(&[
        "features",
        "report",
        "--milestone",
        milestone,
        "--store",
        STORE,
        "--listings",
        LISTINGS,
        "--out",
        OUT,
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let text = workspace.read_text(&format!("{OUT}/features.json"));
    (outcome, serde_json::from_str(&text).unwrap())
}

/// Each Feature's ID with its status and, when it has one, its cause or
/// evidence kind.
fn statuses(report: &Value) -> BTreeMap<String, String> {
    report["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|feature| {
            let status = feature["status"].as_str().unwrap();
            let detail = feature["cause"]
                .as_str()
                .or_else(|| feature["evidence"].as_str());
            let status = match detail {
                Some(detail) => format!("{status} {detail}"),
                None => status.to_owned(),
            };
            (feature["id"].as_str().unwrap().to_owned(), status)
        })
        .collect()
}

fn feature<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|feature| feature["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not in the report"))
}

#[test]
fn each_feature_gets_its_status_and_cause_at_the_milestone() {
    let workspace = Workspace::from_fixture("features-report");
    fixture_store(&workspace);

    let (_, report) = report(&workspace, "M1");

    let expected: BTreeMap<String, String> = [
        ("error:sample_excepted", "excepted manual"),
        ("error:sample_unreachable", "excepted none"),
        ("operation:sample.crawl", "tested"),
        ("operation:sample.failed", "untested failed"),
        ("operation:sample.quarantined", "untested quarantined"),
        ("operation:sample.tested", "tested"),
        ("operation:sample.unrun", "untested not-run"),
        ("operation:sample.untested", "untested no-test"),
        ("story:1-1", "tested"),
        ("story:1-2", "untested no-test-above-unit-level"),
        ("story:1-3", "untested no-covering-journey"),
        ("story:1-4", "planned"),
        ("story:1-5", "planned"),
    ]
    .into_iter()
    .map(|(id, status)| (id.to_owned(), status.to_owned()))
    .collect();
    assert_eq!(statuses(&report), expected);
}

#[test]
fn each_products_view_lists_only_the_features_it_carries() {
    let workspace = Workspace::from_fixture("features-report");
    fixture_store(&workspace);

    let (outcome, report) = report(&workspace, "M1");

    let views: BTreeMap<&str, Vec<&str>> = report["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|view| {
            let features = view["features"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap())
                .collect();
            (view["product"].as_str().unwrap(), features)
        })
        .collect();
    assert!(!views["collie"].contains(&"operation:sample.crawl"));
    assert!(views["foxhound"].contains(&"operation:sample.crawl"));
    assert_eq!(views["collie"].len() + 1, views["foxhound"].len());
    let collie = &report["views"][0];
    assert_eq!(collie["product"], "collie");
    assert_eq!(collie["summary"]["tested"], 2);
    assert_eq!(collie["summary"]["excepted"]["manual"], 1);
    assert_eq!(collie["summary"]["excepted"]["none"], 1);
    assert_eq!(collie["summary"]["planned"], 2);
    assert_eq!(
        outcome.stdout.lines().collect::<Vec<_>>(),
        vec![
            "collie: 2 tested, 6 untested, 2 excepted, 2 planned at the M1 gate",
            "foxhound: 3 tested, 6 untested, 2 excepted, 2 planned at the M1 gate",
            "M1 gate: not ready, 6 Features untested on main's latest Release run, of c1",
        ]
    );
}

#[test]
fn a_flagged_story_is_due_with_its_earliest_covering_variant() {
    let workspace = Workspace::from_fixture("features-report");
    fixture_store(&workspace);

    let (_, at_m1) = report(&workspace, "M1");
    let (_, at_m2) = report(&workspace, "M2");

    // Story 1-4 is an M1 story whose only covering variant, J-002's
    // `sdk-rust`, is due at M2.
    let story = feature(&at_m1, "story:1-4");
    assert_eq!(story["milestone"], "M1 Core");
    assert_eq!(story["due"], "M2");
    assert_eq!(story["status"], "planned");
    assert_eq!(feature(&at_m1, "story:1-1")["due"], "M1");
    let story = feature(&at_m2, "story:1-4");
    assert_eq!(story["status"], "untested");
    assert_eq!(story["cause"], "no-covering-journey");
    assert_eq!(
        story["tests"][1],
        json!({ "test": "J-002@sdk-rust", "source": "journey", "level": "journey", "outcome": "not-run" })
    );
}

#[test]
fn an_untested_feature_lists_its_declaring_tests_with_their_outcomes() {
    let workspace = Workspace::from_fixture("features-report");
    fixture_store(&workspace);

    let (_, report) = report(&workspace, "M1");

    assert_eq!(
        feature(&report, "operation:sample.failed")["tests"],
        json!([{ "test": "sample tests::fails", "source": "nextest", "level": "unit", "outcome": "failed" }])
    );
    assert_eq!(
        feature(&report, "story:1-1")["tests"],
        json!([
            { "test": "sample::it reaches_the_goal", "source": "nextest", "level": "integration", "outcome": "passed" },
            { "test": "J-001@cli", "source": "journey", "level": "journey", "outcome": "passed" },
        ])
    );
    let excepted = feature(&report, "error:sample_excepted");
    assert_eq!(excepted["reason"], "A person checks it in the owner test.");
    assert_eq!(excepted["link"], "https://example.invalid/owner-test");
}

#[test]
fn without_the_store_no_declaring_test_has_run() {
    let workspace = Workspace::from_fixture("features-report");

    let outcome = workspace.xtask(&["features", "report", "--milestone", "M1", "--out", OUT]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let report: Value =
        serde_json::from_str(&workspace.read_text(&format!("{OUT}/features.json"))).unwrap();
    assert_eq!(report["release_commit"], Value::Null);
    assert_eq!(report["listings"], false);
    let statuses = statuses(&report);
    assert_eq!(statuses["operation:sample.tested"], "untested not-run");
    assert_eq!(
        statuses["operation:sample.quarantined"],
        "untested quarantined"
    );
    assert_eq!(statuses["story:1-1"], "untested not-run");
}

#[test]
fn the_page_lists_each_untested_feature_with_its_cause() {
    let workspace = Workspace::from_fixture("features-report");
    fixture_store(&workspace);
    workspace.edit(
        "features/exceptions.toml",
        "A person checks it in the owner test.",
        "A person checks <b>it</b> in the owner test.",
    );

    report(&workspace, "M1");

    let page = workspace.read_text(&format!("{OUT}/features.html"));
    assert!(page.starts_with("<!doctype html>"));
    for row in [
        "<tr><td><code>operation:sample.failed</code></td><td>failed</td>",
        "<tr><td><code>operation:sample.quarantined</code></td><td>quarantined</td>",
        "<tr><td><code>operation:sample.unrun</code></td><td>not-run</td>",
        "<tr><td><code>operation:sample.untested</code></td><td>no-test</td>",
        "<tr><td><code>story:1-2</code></td><td>no-test-above-unit-level</td>",
        "<tr><td><code>story:1-3</code></td><td>no-covering-journey</td>",
    ] {
        assert!(page.contains(row), "{row} is not on the page");
    }
    assert!(page.contains("&lt;b&gt;it&lt;/b&gt;"));
    assert!(!page.contains("<b>it</b>"));
}

#[test]
fn the_gate_is_not_ready_while_a_due_feature_is_untested() {
    let workspace = Workspace::from_fixture("features-report");
    fixture_store(&workspace);

    let (_, report) = report(&workspace, "M1");

    assert_eq!(
        report["gate"],
        json!({
            "ready": false,
            "untested": [
                "operation:sample.failed",
                "operation:sample.quarantined",
                "operation:sample.unrun",
                "operation:sample.untested",
                "story:1-2",
                "story:1-3",
            ],
        })
    );
}

#[test]
fn the_gate_is_ready_once_every_due_feature_is_tested_or_excepted() {
    let workspace = Workspace::from_fixture("features-report");
    fixture_store(&workspace);
    for feature in [
        "operation:sample.failed",
        "operation:sample.quarantined",
        "operation:sample.unrun",
        "operation:sample.untested",
        "story:1-2",
        "story:1-3",
    ] {
        workspace.append(
            "features/exceptions.toml",
            &format!(
                "\n[[exception]]\nfeature = \"{feature}\"\nevidence = \"manual\"\nreason = \"A person checks it.\"\nlink = \"https://example.invalid/owner-test\"\n"
            ),
        );
    }

    let (outcome, report) = report(&workspace, "M1");

    assert_eq!(report["gate"], json!({ "ready": true, "untested": [] }));
    assert_eq!(
        outcome.stdout.lines().last(),
        Some("M1 gate: ready on main's latest Release run, of c1")
    );
}

#[test]
fn without_a_release_run_the_gate_is_not_ready() {
    let workspace = Workspace::from_fixture("features-report");
    workspace.write("features/exceptions.toml", b"");
    workspace.write("features/inventory.json", br#"{"features": []}"#);

    let outcome = workspace.xtask(&["features", "report", "--milestone", "M1", "--out", OUT]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let report: Value =
        serde_json::from_str(&workspace.read_text(&format!("{OUT}/features.json"))).unwrap();
    assert_eq!(report["gate"], json!({ "ready": false, "untested": [] }));
    assert_eq!(
        outcome.stdout.lines().last(),
        Some("M1 gate: not ready, since the store holds no Release run of main")
    );
}

#[test]
fn an_unknown_milestone_is_refused() {
    let workspace = Workspace::from_fixture("features-report");

    let outcome = workspace.xtask(&["features", "report", "--milestone", "M5", "--out", OUT]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
}

/// A variant counts as the Journey report judges it, from the result
/// documents its cells kept: one whose test passed in a cell that kept no
/// document of it is failed, never proved (ADR 0072's rules 3 and 4; #349).
#[test]
fn a_variant_whose_test_passed_without_keeping_a_document_is_failed() {
    let workspace = Workspace::from_fixture("features-report");
    fixture_store_keeping(&workspace, Vec::new());

    let (_, report) = report(&workspace, "M1");

    let story = feature(&report, "story:1-1");
    assert_eq!(
        story["tests"][1],
        json!({ "test": "J-001@cli", "source": "journey", "level": "journey", "outcome": "failed" })
    );
    assert_eq!(story["status"], "untested");
}

/// A conformance case's declaration counts as the conformance document its
/// Release cell kept gives it.
#[test]
fn a_conformance_case_counts_as_its_kept_document_gives_it() {
    let workspace = Workspace::from_fixture("features-report");
    workspace.write(
        "conformance/sample/cases.toml",
        b"product = \"foxhound\"\n\n[suite]\nname = \"sample\"\nversion = \"1\"\n\n[[capability]]\nname = \"echo\"\n\n[[capability.case]]\nid = \"echo-repeats\"\ncovers = [\"operation:sample.untested\"]\n",
    );
    let document = json!({
        "kind": "conformance",
        "suite": { "name": "sample", "version": "1" },
        "target": { "name": "goodfetch-foxhound", "version": "0.1.0" },
        "platform": { "os": "linux", "arch": "x86_64" },
        "runtime": "rust",
        "proving_ground": { "version": "0.1.0", "seed": 7 },
        "online": false,
        "capabilities": [{
            "name": "echo",
            "outcome": "proved",
            "cases": [{
                "id": "echo-repeats",
                "covers": ["operation:sample.untested"],
                "outcome": "passed",
                "entries": [{ "checks": [{ "name": "the echo repeats", "passed": true }], "evidence": {} }],
            }],
        }],
    });
    fixture_store_keeping(
        &workspace,
        vec![
            j001_cli_kept(),
            (
                format!("{LINUX}/result-documents/conformance-sample.json"),
                serde_json::to_vec(&document).unwrap(),
            ),
        ],
    );

    let (_, report) = report(&workspace, "M1");

    let entry = feature(&report, "operation:sample.untested");
    assert_eq!(
        entry["tests"],
        json!([{ "test": "sample echo-repeats", "source": "conformance", "level": "conformance", "outcome": "passed" }])
    );
    assert_eq!(entry["status"], "tested");
}

/// ADR 0108's whole-cell rule: a commit some of whose Release cells have not
/// run is not read, so a newer commit with one cell run leaves the report on
/// the last commit every cell ran.
#[test]
fn a_commit_some_of_whose_cells_have_not_run_is_not_read() {
    let workspace = Workspace::from_fixture("features-report");
    let complete = Attempt {
        test_runs: in_every_release_cell(&release_run("sample tests::passes", "passed", "1")),
        ..release_attempt(10, "2026-10-18T03:00:00Z", "c1")
    };
    let partial = Attempt {
        test_runs: in_every_release_cell(&release_run("sample tests::passes", "failed", "2"))
            .into_iter()
            .take(1)
            .collect(),
        ..release_attempt(11, "2026-10-19T03:00:00Z", "c2")
    };
    write_store(&workspace, &[complete, partial]);

    let (_, report) = report(&workspace, "M1");

    assert_eq!(report["release_commit"], "c1");
    assert_eq!(
        feature(&report, "operation:sample.tested")["tests"][0]["outcome"],
        "passed"
    );
}
