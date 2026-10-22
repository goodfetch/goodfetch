//! `xtask journeys report` (ADRs 0058, 0063 and 0094): each variant of an
//! active Journey with its outcome on `main`'s latest Release run, from the
//! result documents that run kept in the results store, loaded through
//! `goodfetch-protocol`'s reader, by milestone, area and Product; and, with
//! `--owner-test`, the owner test's "Journeys to try" from the catalog and
//! the milestone's `manual` exceptions.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::store::{Attempt, STORE, in_every_release_cell, test_run, with, write_store};
use crate::workspace::{Outcome, Workspace};

/// The folder the report is written to.
const OUT: &str = "target/journeys-report";

/// The Linux and macOS artifacts of a Release run's cells.
const LINUX: &str = "junit-release-dispatch-linux-x64-gnu-1";
const MACOS: &str = "junit-release-dispatch-macos-arm64-1";

/// A `journey` result document of `variant`, `J-NNN@<surface>`, holding
/// each step's and check's text with its outcome, and the outcome it says
/// they give, written as `JourneyRun` writes one.
fn document(
    variant: &str,
    steps: &[(&str, &str)],
    checks: &[(&str, &str)],
    outcome: &str,
) -> Vec<u8> {
    let (journey, surface) = variant.split_once('@').unwrap();
    let pairs = |items: &[(&str, &str)]| -> Vec<Value> {
        items
            .iter()
            .map(|(text, outcome)| json!({ "text": text, "outcome": outcome }))
            .collect()
    };
    let document = json!({
        "kind": "journey",
        "journey_id": journey,
        "surface": surface,
        "executable": { "product": "foxhound", "version": "0.1.0" },
        "platform": { "os": "linux", "arch": "x86_64" },
        "runtime": "rust",
        "steps": pairs(steps),
        "checks": pairs(checks),
        "tasks": [],
        "outcome": outcome,
    });
    serde_json::to_vec_pretty(&document).unwrap()
}

/// J-001's three steps and its check, each with the outcome given.
fn j001(variant: &str, open: &str, check: &str, outcome: &str) -> Vec<u8> {
    document(
        variant,
        &[
            ("Start the Daemon", "passed"),
            (
                "Ask for the Console, which the system browser opens",
                "manual",
            ),
            ("Open the Console", open),
        ],
        &[("The Console opens with no login", check)],
        outcome,
    )
}

/// `document` kept by `artifact` as the Release cells keep it.
fn kept(artifact: &str, variant: &str, document: Vec<u8>) -> (String, Vec<u8>) {
    (
        format!("{artifact}/result-documents/{variant}.json"),
        document,
    )
}

/// A Release run on `main`, started at `started_at`, that tested `commit`,
/// whose one test run names it.
fn release_attempt(run: u64, started_at: &'static str, commit: &'static str) -> Attempt {
    let tree = commit.trim_start_matches('c');
    let test_run = with(
        test_run("sample tests::passes", "passed", tree),
        "suite",
        json!("release"),
    );
    Attempt {
        workflow: ".github/workflows/release-suite.yml",
        event: "workflow_dispatch",
        commit,
        // Every cell ran (ADR 0108's whole-cell rule).
        test_runs: in_every_release_cell(&with(test_run, "tier", json!("dispatch"))),
        ..Attempt::new(run, started_at)
    }
}

/// The fixture's store: an older Release run, `main`'s latest Release run
/// across two cells, and a later test-health run, which is no Release run.
fn fixture_store(workspace: &Workspace) {
    let j003 = document(
        "J-003@cli",
        &[
            ("Start a crawl", "passed"),
            ("Kill the Daemon and start it again", "passed"),
        ],
        &[("No Record is delivered twice", "passed")],
        "proved",
    );
    let older = Attempt {
        kept: vec![kept(LINUX, "J-003@cli", j003.clone())],
        ..release_attempt(9, "2026-10-17T03:00:00Z", "c0")
    };
    // J-001@console leaves out its last step; J-001@foxhound-cli says
    // `proved` with a failed check. The reader refuses both.
    let console = document(
        "J-001@console",
        &[
            ("Start the Daemon", "passed"),
            (
                "Ask for the Console, which the system browser opens",
                "manual",
            ),
        ],
        &[("The Console opens with no login", "passed")],
        "proved",
    );
    let failed_crawl = document(
        "J-002@cli",
        &[("Crawl the site", "failed")],
        &[("Each Record holds its page's Markdown", "not_reached")],
        "failed",
    );
    let crawl = document(
        "J-002@cli",
        &[("Crawl the site", "passed")],
        &[("Each Record holds its page's Markdown", "passed")],
        "proved",
    );
    let latest = Attempt {
        kept: vec![
            kept(
                LINUX,
                "J-001@cli",
                j001("J-001@cli", "passed", "passed", "proved"),
            ),
            kept(LINUX, "J-001@console", console),
            kept(
                LINUX,
                "J-001@foxhound-cli",
                j001("J-001@foxhound-cli", "passed", "failed", "proved"),
            ),
            kept(LINUX, "J-002@cli", failed_crawl),
            kept(
                MACOS,
                "J-001@cli",
                j001("J-001@cli", "passed", "passed", "proved"),
            ),
            kept(MACOS, "J-002@cli", crawl),
        ],
        ..release_attempt(10, "2026-10-18T03:00:00Z", "c1")
    };
    let health = Attempt {
        test_runs: vec![test_run("sample tests::passes", "passed", "1")],
        commit: "c1",
        kept: vec![kept(
            "junit-pre-merge-test-health-linux-x64-1",
            "J-003@cli",
            j003,
        )],
        ..Attempt::new(11, "2026-10-19T03:00:00Z")
    };
    write_store(workspace, &[older, latest, health]);
}

fn report(workspace: &Workspace, arguments: &[&str]) -> (Outcome, Value) {
    let mut all = vec!["journeys", "report", "--out", OUT];
    all.extend_from_slice(arguments);
    let outcome = workspace.xtask(&all);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let text = workspace.read_text(&format!("{OUT}/journeys.json"));
    (outcome, serde_json::from_str(&text).unwrap())
}

/// Each variant's ID with its outcome, across the report's groups.
fn outcomes(report: &Value) -> BTreeMap<String, String> {
    report["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["variants"].as_array().unwrap())
        .map(|variant| {
            (
                variant["id"].as_str().unwrap().to_owned(),
                variant["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn each_variant_gets_its_outcome_from_the_documents_of_mains_latest_release_run() {
    let workspace = Workspace::from_fixture("journeys-report");
    fixture_store(&workspace);

    let (_, report) = report(&workspace, &["--store", STORE]);

    assert_eq!(report["release_commit"], "c1");
    let expected: BTreeMap<String, String> = [
        ("J-001@cli", "proved"),
        ("J-001@console", "rejected"),
        ("J-001@foxhound-cli", "rejected"),
        ("J-002@cli", "failed"),
        ("J-002@sdk-rust", "unproved"),
        ("J-003@cli", "unproved"),
        ("J-005@sdk-python", "unproved"),
    ]
    .into_iter()
    .map(|(id, outcome)| (id.to_owned(), outcome.to_owned()))
    .collect();
    assert_eq!(outcomes(&report), expected);
}

/// The runners' listings, as `xtask journeys listings` writes them, in
/// which `sample::it journeys::j001::cli` carries J-001@cli.
fn write_listings(workspace: &Workspace) {
    workspace.write(
        "listings/nextest.json",
        br#"{ "rust-suites": { "sample::it": { "testcases": { "journeys::j001::cli": {} } } } }"#,
    );
    workspace.write("listings/vitest.json", b"[]");
    workspace.write("listings/pytest.json", br#"{ "tests": [] }"#);
    workspace.write(
        "listings/playwright.json",
        br#"{ "config": { "rootDir": "." }, "suites": [] }"#,
    );
}

/// A Release test run of `c1` of J-001@cli's test, in the cell of `os`,
/// `arch` and `runtime`.
fn j001_cli_run(os: &str, arch: &str, runtime: Option<&str>) -> Value {
    let run = with(
        test_run("sample::it journeys::j001::cli", "passed", "1"),
        "suite",
        json!("release"),
    );
    let run = with(run, "tier", json!("dispatch"));
    let run = with(with(run, "os", json!(os)), "arch", json!(arch));
    with(run, "runtime", json!(runtime))
}

/// A cell that ran a variant's test but kept no document, as when the test
/// panics before `finish`, leaves the variant `failed`, even when another
/// cell's document proves it (ADR 0063: every automated variant `proved`).
#[test]
fn a_variant_whose_test_ran_in_a_cell_that_kept_no_document_is_failed() {
    let workspace = Workspace::from_fixture("journeys-report");
    write_listings(&workspace);
    let latest = Attempt {
        test_runs: [
            j001_cli_run("linux", "x64", Some("gnu")),
            j001_cli_run("macos", "arm64", None),
        ]
        .into_iter()
        .chain(release_attempt(10, "2026-10-18T03:00:00Z", "c1").test_runs)
        .collect(),
        kept: vec![kept(
            LINUX,
            "J-001@cli",
            j001("J-001@cli", "passed", "passed", "proved"),
        )],
        ..release_attempt(10, "2026-10-18T03:00:00Z", "c1")
    };
    write_store(&workspace, &[latest]);

    let (outcome, report) = report(&workspace, &["--store", STORE, "--listings", "listings"]);

    assert_eq!(outcomes(&report)["J-001@cli"], "failed");
    assert!(outcome.stdout.contains("macos arm64"), "{}", outcome.stdout);
}

/// With every cell that ran the variant's test keeping its document, the
/// documents decide.
#[test]
fn a_variant_every_cell_that_ran_it_kept_a_proving_document_for_is_proved() {
    let workspace = Workspace::from_fixture("journeys-report");
    write_listings(&workspace);
    let latest = Attempt {
        test_runs: [
            j001_cli_run("linux", "x64", Some("gnu")),
            j001_cli_run("macos", "arm64", None),
        ]
        .into_iter()
        .chain(release_attempt(10, "2026-10-18T03:00:00Z", "c1").test_runs)
        .collect(),
        kept: vec![
            kept(
                LINUX,
                "J-001@cli",
                j001("J-001@cli", "passed", "passed", "proved"),
            ),
            kept(
                MACOS,
                "J-001@cli",
                j001("J-001@cli", "passed", "passed", "proved"),
            ),
        ],
        ..release_attempt(10, "2026-10-18T03:00:00Z", "c1")
    };
    write_store(&workspace, &[latest]);

    let (_, report) = report(&workspace, &["--store", STORE, "--listings", "listings"]);

    assert_eq!(outcomes(&report)["J-001@cli"], "proved");
}

#[test]
fn a_document_the_reader_refuses_is_rejected_never_proved() {
    let workspace = Workspace::from_fixture("journeys-report");
    let incomplete = document(
        "J-003@cli",
        &[("Start a crawl", "passed")],
        &[("No Record is delivered twice", "passed")],
        "proved",
    );
    let complete = document(
        "J-002@cli",
        &[("Crawl the site", "passed")],
        &[("Each Record holds its page's Markdown", "passed")],
        "proved",
    );
    let latest = Attempt {
        kept: vec![
            kept(LINUX, "J-003@cli", incomplete),
            kept(LINUX, "J-002@cli", complete),
        ],
        ..release_attempt(10, "2026-10-18T03:00:00Z", "c1")
    };
    write_store(&workspace, &[latest]);

    let (_, report) = report(&workspace, &["--store", STORE]);

    let outcomes = outcomes(&report);
    assert_eq!(outcomes["J-002@cli"], "proved");
    assert_eq!(outcomes["J-003@cli"], "rejected");
}

#[test]
fn outcomes_group_by_milestone_area_and_product() {
    let workspace = Workspace::from_fixture("journeys-report");
    fixture_store(&workspace);

    let (outcome, report) = report(&workspace, &["--store", STORE]);

    let groups: Vec<(String, u64, String, Vec<String>)> = report["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| {
            let variants = group["variants"]
                .as_array()
                .unwrap()
                .iter()
                .map(|variant| variant["id"].as_str().unwrap().to_owned())
                .collect();
            (
                group["milestone"].as_str().unwrap().to_owned(),
                group["area"].as_u64().unwrap(),
                group["product"].as_str().unwrap().to_owned(),
                variants,
            )
        })
        .collect();
    let group = |milestone: &str, area, product: &str, variants: &[&str]| {
        (
            milestone.to_owned(),
            area,
            product.to_owned(),
            variants
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        groups,
        vec![
            group("M1", 3, "collie", &["J-001@cli", "J-001@console"]),
            group("M1", 3, "foxhound", &["J-001@foxhound-cli", "J-003@cli"]),
            group("M1", 6, "foxhound", &["J-002@cli"]),
            group("M2", 6, "foxhound", &["J-002@sdk-rust", "J-005@sdk-python"]),
        ]
    );
    let headings: Vec<&str> = outcome
        .stdout
        .lines()
        .filter(|line| line.starts_with("## "))
        .collect();
    assert_eq!(
        headings,
        vec![
            "## M1, area 3, Collie",
            "## M1, area 3, Foxhound",
            "## M1, area 6, Foxhound",
            "## M2, area 6, Foxhound",
        ]
    );
    assert!(
        outcome
            .stdout
            .lines()
            .any(|line| line == "- `J-003@cli` unproved: Resume a killed crawl"),
        "{}",
        outcome.stdout
    );
    assert_eq!(
        workspace.read_text(&format!("{OUT}/journeys.md")),
        outcome.stdout
    );
}

#[test]
fn without_a_release_run_of_main_every_variant_is_unproved() {
    let workspace = Workspace::from_fixture("journeys-report");

    let (_, report) = report(&workspace, &[]);

    assert_eq!(report["release_commit"], Value::Null);
    assert!(
        outcomes(&report)
            .values()
            .all(|outcome| outcome == "unproved"),
        "{report}"
    );
}

#[test]
fn the_owner_test_lists_both_products_journeys_due_with_their_manual_steps_and_exceptions() {
    let workspace = Workspace::from_fixture("journeys-report");

    let outcome = workspace.xtask(&["journeys", "report", "--owner-test", "M1"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stdout,
        "\
- [ ] **J-001** (Collie) Start the Daemon and open its Console (`J-001@cli`, `J-001@console`, `J-001@foxhound-cli`). Manual step: Ask for the Console, which the system browser opens (only a person sees the browser open).
- [ ] **J-002** (Foxhound) Crawl a site to Markdown (`J-002@cli`)
- [ ] **J-003** (Foxhound) Resume a killed crawl (`J-003@cli`)

**Manual exceptions** (`features/exceptions.toml`, evidence `manual`):

- [ ] `error:sample_by_hand`: A person reads the error's wording. (https://example.invalid/owner-test#wording)
- [ ] `story:3-1`: A person sees the Daemon's tray icon. (https://example.invalid/owner-test#tray)
"
    );
}

#[test]
fn a_later_owner_test_lists_only_the_journeys_due_at_its_milestone() {
    let workspace = Workspace::from_fixture("journeys-report");

    let outcome = workspace.xtask(&["journeys", "report", "--owner-test", "M2"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stdout,
        "\
- [ ] **J-002** (Foxhound) Crawl a site to Markdown (`J-002@sdk-rust`)
- [ ] **J-005** (Foxhound) Crawl from Python (`J-005@sdk-python`)

**Manual exceptions** (`features/exceptions.toml`, evidence `manual`):

- [ ] `error:sample_by_hand`: A person reads the error's wording. (https://example.invalid/owner-test#wording)
- [ ] `story:3-2`: A person checks it at the next milestone. (https://example.invalid/owner-test#later)
"
    );
}
