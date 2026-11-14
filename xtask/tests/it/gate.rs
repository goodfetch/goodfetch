//! The incremental local gate (`xtask gate`, ADR 0135): a gate run records
//! each step's result by head, and a later run on a descendant head reruns
//! the steps and members `xtask affected` finds the change since then
//! reaches, those the evidence of what each member read reaches, every check
//! that judges the whole change, and every red or missing result; it reuses
//! the rest. The local gate is repository tooling, so the module declares no
//! Feature.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::affected::{commit_all, repository, value};
use crate::github::GitHub;
use rstest::rstest;

use crate::workspace::{Outcome, Workspace, write_executable};

/// Now, in seconds since the Unix epoch, for every run.
const NOW: &str = "1800000000";

/// Each step's `(action, members)` and reason from the plan's
/// `<step>.action`, `<step>.members` and `<step>.why` lines, and the
/// `previous`, `head` and `stress.base` lines' heads.
struct Plan {
    previous: String,
    head: String,
    stress_base: String,
    steps: BTreeMap<String, (String, String)>,
    why: BTreeMap<String, String>,
}

/// Plans with the records in `records` and the dep-info in the `target`
/// folder beside them.
fn plan(workspace: &Workspace, records: &Path) -> Plan {
    plan_in(workspace, records, "CI=")
}

/// Plans as [`plan`] does, in the run's environment `environment`.
fn plan_in(workspace: &Workspace, records: &Path, environment: &str) -> Plan {
    plan_with(workspace, records, environment, &[])
}

/// Plans as [`plan_in`] does, with these arguments added.
fn plan_with(workspace: &Workspace, records: &Path, environment: &str, extra: &[&str]) -> Plan {
    parse(&planned(workspace, records, environment, extra))
}

/// The plan for the head checked out, as `xtask gate plan` prints it and
/// `ci/pre-merge.sh` reads it.
pub(crate) fn plan_text(workspace: &Workspace, records: &Path) -> String {
    planned(workspace, records, "CI=", &[]).stdout
}

/// Runs `xtask gate plan` as [`plan_with`] does: its outcome, which passed.
fn planned(workspace: &Workspace, records: &Path, environment: &str, extra: &[&str]) -> Outcome {
    planned_at(workspace, records, NOW, environment, extra, &[])
}

/// Plans as [`plan`] does, at `now`.
fn plan_at(workspace: &Workspace, records: &Path, now: &str) -> Plan {
    parse(&planned_at(workspace, records, now, "CI=", &[], &[]))
}

/// Plans as [`plan`] does, reading the quarantined tests' issues from
/// `github` with a token.
fn plan_reading_issues(workspace: &Workspace, records: &Path, github: &GitHub) -> Plan {
    parse(&planned_at(
        workspace,
        records,
        NOW,
        "CI=",
        &["--api-url", github.url()],
        &[("GITHUB_TOKEN", "a-read-token".as_ref())],
    ))
}

/// Runs `xtask gate plan` as [`planned`] does, at `now`, with these
/// variables added.
fn planned_at(
    workspace: &Workspace,
    records: &Path,
    now: &str,
    environment: &str,
    extra: &[&str],
    variables: &[(&str, &std::ffi::OsStr)],
) -> Outcome {
    let target = target(records);
    let records = records.to_string_lossy();
    let target = target.to_string_lossy();
    let mut args = vec![
        "gate",
        "plan",
        "--records",
        &records,
        "--now",
        now,
        "--target-dir",
        &target,
        "--environment",
        environment,
    ];
    args.extend(extra);
    let outcome = workspace.xtask_with_env(&args, variables);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    outcome
}

fn parse(outcome: &Outcome) -> Plan {
    let mut plan = Plan {
        previous: String::new(),
        head: String::new(),
        stress_base: String::new(),
        steps: BTreeMap::new(),
        why: BTreeMap::new(),
    };
    for line in outcome.stdout.lines() {
        let (key, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("a plan line is `<key>=<value>`, not {line:?}"));
        match key.split_once('.') {
            None => match key {
                "previous" => value.clone_into(&mut plan.previous),
                "head" => value.clone_into(&mut plan.head),
                "members" | "unowned" => {}
                _ => panic!("an unexpected plan line: {line:?}"),
            },
            Some(("stress", "base")) => value.clone_into(&mut plan.stress_base),
            Some((step, "action")) => {
                value.clone_into(&mut plan.steps.entry(step.to_owned()).or_default().0);
            }
            Some((step, "members")) => {
                value.clone_into(&mut plan.steps.entry(step.to_owned()).or_default().1);
            }
            Some((step, "why")) => {
                plan.why.insert(step.to_owned(), value.to_owned());
            }
            Some((_, "earned")) => {}
            Some(_) => panic!("an unexpected plan line: {line:?}"),
        }
    }
    plan
}

fn record_outcome(workspace: &Workspace, records: &Path, head: &str, step: &str) -> Outcome {
    workspace.xtask(&[
        "gate",
        "record",
        "--records",
        &records.to_string_lossy(),
        "--head",
        head,
        "--now",
        NOW,
        "--step",
        step,
        "--status",
        "pass",
    ])
}

fn record(workspace: &Workspace, records: &Path, head: &str, step: &str, status: &str) {
    let outcome = workspace.xtask(&[
        "gate",
        "record",
        "--records",
        &records.to_string_lossy(),
        "--head",
        head,
        "--now",
        NOW,
        "--step",
        step,
        "--status",
        status,
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

fn action<'a>(plan: &'a Plan, step: &str) -> (&'a str, &'a str) {
    let (action, members) = &plan.steps[step];
    (action, members)
}

fn members(plan: &Plan, step: &str) -> BTreeSet<String> {
    action(plan, step).1.split(' ').map(str::to_owned).collect()
}

/// The Cargo target folder beside the records.
fn target(records: &Path) -> PathBuf {
    records.with_file_name("target")
}

/// The records' folder, outside the repository so the tree stays clean.
fn records() -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let records = folder.path().join("gate-runs");
    (folder, records)
}

/// The fixture's crates, each with the folder of its source.
const CRATES: [(&str, &str); 5] = [
    ("goodfetch-engine", "crates/goodfetch-engine/src/lib.rs"),
    (
        "goodfetch-conformance",
        "crates/goodfetch-conformance/src/lib.rs",
    ),
    (
        "goodfetch-frontier",
        "crates/foxhound/goodfetch-frontier/src/lib.rs",
    ),
    (
        "goodfetch-sink-s3",
        "crates/foxhound/sinks/goodfetch-sink-s3/src/main.rs",
    ),
    ("goodfetch-proving-ground", "proving-ground/src/main.rs"),
];

/// Writes Cargo's dep-info for each fixture crate's last build into the
/// target folder beside `records`: its root source, and the extra files
/// `extra` gives a crate.
fn dep_info(workspace: &Workspace, records: &Path, extra: &[(&str, &str)]) {
    let deps = target(records).join("debug/deps");
    std::fs::create_dir_all(&deps).unwrap();
    for (krate, source) in CRATES {
        let mut prerequisites = vec![source.to_owned()];
        prerequisites.extend(
            extra
                .iter()
                .filter(|(name, _)| *name == krate)
                .map(|(_, path)| (*path).to_owned()),
        );
        let file = deps.join(format!("{}-0.d", krate.replace('-', "_")));
        let sources: Vec<String> = prerequisites
            .iter()
            .map(|path| workspace.path().join(path).display().to_string())
            .collect();
        std::fs::write(
            &file,
            format!("{}: {}\n", file.display(), sources.join(" ")),
        )
        .unwrap();
    }
}

/// The fixture as a repository, with its records' folder and dep-info.
fn gated_repository(extra: &[(&str, &str)]) -> (Workspace, tempfile::TempDir, PathBuf) {
    let workspace = repository();
    let (folder, records) = records();
    dep_info(&workspace, &records, extra);
    (workspace, folder, records)
}

/// A first run on the fixture's `change` head, with perf, TLC and the
/// Pre-merge suite green and the Quick suite red.
fn first_run(workspace: &Workspace, records: &Path) -> String {
    let first = plan(workspace, records);
    assert_eq!(first.previous, "none");
    assert!(first.steps.values().all(|(action, _)| action == "run"));
    for (step, status) in [
        ("perf", "pass"),
        ("tlc", "pass"),
        ("visual", "pass"),
        ("pre-merge", "pass"),
        ("quick", "fail"),
    ] {
        record(workspace, records, &first.head, step, status);
    }
    first.head
}

/// The members `xtask affected` selects for the change from `base` to HEAD.
fn affected_members(workspace: &Workspace, base: &str) -> BTreeSet<String> {
    let affected = workspace.xtask(&["affected", "--base", base, "--head", "HEAD"]);
    ["crates", "vitest-packages", "pytest-packages"]
        .iter()
        .flat_map(|key| {
            value(&affected, key)
                .split(' ')
                .filter(|member| !member.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn a_later_run_reruns_what_the_change_since_the_first_reaches_and_every_red_result() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = first_run(&workspace, &records);
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"\"\"\"Changed.\"\"\"\n",
    );
    commit_all(&workspace, "Change the Python client");

    let second = plan(&workspace, &records);

    assert_eq!(second.previous, first);
    // What `xtask affected` selects, plus the pnpm packages, which
    // Vitest and Playwright rerun in the head's order in every run.
    let mut expected = affected_members(&workspace, &first);
    assert_eq!(
        expected,
        BTreeSet::from([
            "sdks/python/collie-client".to_owned(),
            "sdks/python/foxhound".to_owned()
        ])
    );
    expected.extend([
        "sdks/typescript/collie-client".to_owned(),
        "sdks/typescript/foxhound".to_owned(),
    ]);
    assert_eq!(action(&second, "pre-merge").0, "part");
    assert_eq!(members(&second, "pre-merge"), expected);
    assert!(
        second.why["pre-merge"].contains("in the head's test order"),
        "{}",
        second.why["pre-merge"]
    );
    // Green, and reached by nothing that changed.
    assert_eq!(action(&second, "perf").0, "reuse");
    assert_eq!(action(&second, "tlc").0, "reuse");
    // Red on the first head: never reused.
    assert_eq!(action(&second, "quick"), ("run", "-"));
    // Never recorded on the first head.
    assert_eq!(action(&second, "msrv"), ("run", "-"));
    for step in [
        "dco",
        "commit-messages",
        "floor-guard",
        "lint",
        "audit",
        "commit-leaks",
        "notices",
    ] {
        assert_eq!(action(&second, step).0, "run", "{step}");
    }
}

/// A crate whose build read a changed file outside its folder, as
/// `include_str!("../../TESTING.md")` does, reruns, with its dependants
/// (the review's F5); a crate with no dep-info reruns too.
#[test]
fn the_evidence_of_what_a_crate_read_reruns_it_and_missing_evidence_fails_closed() {
    let (workspace, _folder, records) = gated_repository(&[("goodfetch-engine", "TESTING.md")]);
    workspace.write("TESTING.md", b"# Testing\n");
    commit_all(&workspace, "Add the testing guide");
    first_run(&workspace, &records);
    std::fs::remove_file(target(&records).join("debug/deps/goodfetch_sink_s3-0.d")).unwrap();
    workspace.write("TESTING.md", b"# Testing, reworded\n");
    commit_all(&workspace, "Reword the testing guide");

    let second = plan(&workspace, &records);

    let rerun = members(&second, "pre-merge");
    for krate in [
        "goodfetch-engine",
        "goodfetch-frontier",
        "goodfetch-sink-s3",
    ] {
        assert!(rerun.contains(krate), "{krate} not in {rerun:?}");
    }
    assert!(!rerun.contains("goodfetch-proving-ground"), "{rerun:?}");
    let why = &second.why["pre-merge"];
    assert!(why.contains("its build read TESTING.md"), "{why}");
    assert!(why.contains("no dep-info"), "{why}");
}

/// A test that reads a file no member owns at run time names it in a string
/// literal: its member reruns when the file changes.
#[test]
fn a_member_whose_files_name_a_changed_file_in_a_literal_reruns() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write(
        "crates/goodfetch-conformance/src/lib.rs",
        b"//! Reads the docs.\npub const INDEX: &str = \"docs/index.md\";\n",
    );
    commit_all(&workspace, "Read the docs' index");
    first_run(&workspace, &records);
    workspace.write("docs/index.md", b"# Changed\n");
    commit_all(&workspace, "Change the docs' index");

    let second = plan(&workspace, &records);

    let rerun = members(&second, "pre-merge");
    assert!(rerun.contains("goodfetch-conformance"), "{rerun:?}");
    let why = &second.why["pre-merge"];
    assert!(!rerun.contains("goodfetch-engine"), "{rerun:?} {why}");
    assert!(
        second.why["pre-merge"].contains("names docs/index.md"),
        "{}",
        second.why["pre-merge"]
    );
}

/// The fixture with an `xtask` crate of its own, as the repository has, and
/// its dep-info: a first run green on every step.
fn green_run_with_xtask(workspace: &Workspace, records: &Path) {
    add_xtask(workspace, records);
    let first = plan(workspace, records);
    record_plan(workspace, records, &first, &[]);
}

/// Adds an `xtask` crate to the fixture, as the repository has, with the
/// dep-info of its executable and its tests.
fn add_xtask(workspace: &Workspace, records: &Path) {
    workspace.edit(
        "Cargo.toml",
        "\"proving-ground\",",
        "\"proving-ground\",\n  \"xtask\",",
    );
    workspace.write(
        "xtask/Cargo.toml",
        b"[package]\nname = \"xtask\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    );
    workspace.write("xtask/src/main.rs", b"fn main() {}\n");
    workspace.write("xtask/tests/it/main.rs", b"//! Its tests.\n");
    commit_all(workspace, "Add xtask");
    let deps = target(records).join("debug/deps");
    for (name, source) in [
        ("xtask", "xtask/src/main.rs"),
        ("it", "xtask/tests/it/main.rs"),
    ] {
        let file = deps.join(format!("{name}-0.d"));
        let source = workspace.path().join(source);
        std::fs::write(&file, format!("{}: {}\n", file.display(), source.display())).unwrap();
    }
}

/// The steps of `plan` that run whatever a change reaches.
const EVERY_RUN: [&str; 12] = [
    "dco",
    "commit-messages",
    "floor-guard",
    "commit-leaks",
    "audit",
    "notices",
    "lint",
    "feature-combinations",
    "fuzz-check",
    "proofs",
    "journeys-tasks",
    "features-tests",
];

/// The members Vitest, Playwright and pytest test in the fixture: every run
/// reruns them in the head's order.
const SHUFFLED: [&str; 4] = [
    "sdks/python/collie-client",
    "sdks/python/foxhound",
    "sdks/typescript/collie-client",
    "sdks/typescript/foxhound",
];

/// #2090's F4: a file of the gate's inputs reaches the steps that read it and
/// no other. An xtask test file reaches xtask's own tests and, since the
/// Linux-only tests are partly xtask's, `just linux`; a `ci/` file its
/// steps; one no gate step runs, or that only a step every run reruns reads,
/// reaches nothing more.
#[rstest::rstest]
#[case::xtask_test_file("xtask/tests/it/stress.rs", &["linux"], true)]
#[case::xtask_fixture("xtask/tests/it/fixtures/empty/README.md", &["linux"], true)]
#[case::ci_no_gate_step_runs("ci/coverage.sh", &[], false)]
#[case::ci_backstop("ci/gate-backstop.sh", &[], false)]
#[case::ci_backstop_switch("ci/gate-backstop.state", &[], false)]
#[case::ci_read_by_a_step_every_run_reruns("ci/gitleaks.toml", &[], false)]
#[case::ci_proof("ci/proofs/msrv/Cargo.toml", &[], false)]
#[case::ci_perf("ci/perf.sh", &["perf"], false)]
#[case::ci_linux("ci/linux.sh", &["linux"], false)]
#[case::ci_visual("ci/visual/run.sh", &["visual"], false)]
#[case::ci_container("ci/container.sh", &["linux", "perf", "visual"], false)]
#[case::ci_msrv("ci/msrv.sh", &["msrv"], false)]
#[case::ci_miri("ci/miri.toml", &["miri"], false)]
fn a_gate_input_the_map_names_reaches_only_the_steps_that_read_it(
    #[case] path: &str,
    #[case] steps: &[&str],
    #[case] reaches_xtask: bool,
) {
    let (workspace, _folder, records) = gated_repository(&[]);
    green_run_with_xtask(&workspace, &records);
    workspace.write(path, b"# Changed.\n");
    commit_all(&workspace, "Change one input");

    let second = plan(&workspace, &records);

    let ran: BTreeSet<&str> = second
        .steps
        .iter()
        .filter(|(_, (action, _))| action == "run")
        .map(|(step, _)| step.as_str())
        .collect();
    let expected: BTreeSet<&str> = EVERY_RUN.iter().chain(steps).copied().collect();
    assert_eq!(ran, expected, "{:?}", second.why);
    let mut rerun: BTreeSet<String> = SHUFFLED.iter().map(|member| (*member).to_owned()).collect();
    if reaches_xtask {
        rerun.insert("xtask".to_owned());
    }
    for step in ["quick", "pre-merge", "stress"] {
        assert_eq!(action(&second, step).0, "part", "{step}: {:?}", second.why);
        assert_eq!(members(&second, step), rerun, "{step}: {:?}", second.why);
    }
}

/// A `ci/` file a member's test names still reaches that member, by the
/// string-literal evidence: a test may run it.
#[test]
fn a_ci_file_a_test_names_reaches_that_tests_member() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write(
        "xtask/tests/it/coverage.rs",
        b"const SCRIPT: &str = \"../ci/coverage.sh\";\n",
    );
    workspace.write("ci/coverage.sh", b"#!/bin/sh\n");
    green_run_with_xtask(&workspace, &records);
    workspace.write("ci/coverage.sh", b"#!/bin/sh\nexit 0\n");
    commit_all(&workspace, "Change the coverage script");

    let second = plan(&workspace, &records);

    assert!(
        members(&second, "pre-merge").contains("xtask"),
        "{:?}",
        second.why
    );
    assert!(
        second.why["pre-merge"].contains("names ci/coverage.sh"),
        "{}",
        second.why["pre-merge"]
    );
    // Its evidence reaches xtask's tests alone, which no crate depends on.
    for step in ["perf", "msrv", "miri", "tlc", "visual", "kennel"] {
        assert_eq!(action(&second, step).0, "reuse", "{step}: {:?}", second.why);
    }
}

/// Any path of the gate's inputs the map does not name still reruns every
/// step, as does `xtask/src/`, which decides the plan. The reason names the
/// changed input first.
#[rstest::rstest]
#[case::justfile("justfile")]
#[case::a_script_every_step_needs("ci/pre-merge.sh")]
#[case::a_new_ci_file("ci/a-new-script.sh")]
#[case::a_new_ci_folder("ci/a-new-folder/settings.toml")]
#[case::xtask_source("xtask/src/gate/plan.rs")]
#[case::xtask_manifest("xtask/Cargo.toml")]
#[case::xtask_modules("xtask/modules.toml")]
fn a_gate_input_the_map_does_not_name_reruns_every_step(#[case] path: &str) {
    let (workspace, _folder, records) = gated_repository(&[]);
    green_run_with_xtask(&workspace, &records);
    let mut contents = workspace.read(path).unwrap_or_default();
    contents.extend_from_slice(b"\n# Changed.\n");
    workspace.write(path, &contents);
    commit_all(&workspace, "Change one input");

    let second = plan(&workspace, &records);

    assert!(
        second.steps.values().all(|(action, _)| action == "run"),
        "{:?}",
        second.steps
    );
    assert!(
        second.why["perf"].starts_with(&format!("{path} changed since ")),
        "{}",
        second.why["perf"]
    );
}

#[test]
fn a_tree_with_uncommitted_changes_reuses_and_records_nothing() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = first_run(&workspace, &records);
    let record_file = records.join(format!("{first}.tsv"));
    let recorded = std::fs::read_to_string(&record_file).unwrap();
    workspace.write("crates/goodfetch-engine/src/lib.rs", b"//! Changed.\n");

    let dirty = plan(&workspace, &records);
    let late = record_outcome(&workspace, &records, &first, "perf");

    assert_eq!(dirty.previous, "none");
    assert_eq!(dirty.head, "none");
    assert!(!dirty.steps.is_empty());
    assert!(
        dirty.steps.values().all(|(action, _)| action == "run"),
        "{:?}",
        dirty.steps
    );
    assert_eq!(late.code, 0, "{}", late.stderr);
    assert_eq!(std::fs::read_to_string(&record_file).unwrap(), recorded);
}

/// A result is recorded only for the head the plan was made for, on a clean
/// tree: a checkout or an edit during the run refuses it and drops the plan
/// (the review's F6).
#[test]
fn a_result_for_another_head_or_a_changed_tree_is_refused() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = plan(&workspace, &records).head;
    workspace.write("crates/goodfetch-engine/src/lib.rs", b"//! Changed.\n");
    commit_all(&workspace, "Move HEAD during the run");

    let moved = record_outcome(&workspace, &records, &first, "perf");

    assert_ne!(moved.code, 0, "{}", moved.stdout);
    assert!(!records.join(format!("{first}.plan")).exists());
    let second = plan(&workspace, &records).head;
    workspace.write(
        "crates/goodfetch-engine/src/lib.rs",
        b"//! Edited mid-run.\n",
    );

    let edited = record_outcome(&workspace, &records, &second, "perf");

    assert_ne!(edited.code, 0, "{}", edited.stdout);
    assert!(!records.join(format!("{second}.plan")).exists());
    for head in [&first, &second] {
        let recorded = std::fs::read_to_string(records.join(format!("{head}.tsv"))).unwrap();
        assert!(!recorded.contains("\nperf\t"), "{recorded}");
    }
}

/// The checks a run on the tree of their green result reuses.
const TREE_ONLY: [&str; 4] = ["lint", "feature-combinations", "fuzz-check", "proofs"];

/// The fixture as a repository whose head holds what lint reads again: a
/// Quality register with one Exception, which expires on `expires`, and a
/// quarantine list with no test. A first run on it at [`NOW`] (2027-01-15,
/// 08:00 UTC) recorded each of [`TREE_ONLY`] green.
fn gated_with_a_green_lint(expires: &str) -> (Workspace, tempfile::TempDir, PathBuf, Plan) {
    let (workspace, folder, records) = gated_repository(&[]);
    workspace.write(
        "TESTING.md",
        crate::register::CLEAN_TESTING
            .replace("2027-01-30", expires)
            .as_bytes(),
    );
    workspace.write("quarantine.toml", b"# No test is quarantined.\n");
    commit_all(
        &workspace,
        "Add the Quality register and the quarantine list",
    );
    let first = plan(&workspace, &records);
    for step in TREE_ONLY {
        record(&workspace, &records, &first.head, step, "pass");
    }
    (workspace, folder, records, first)
}

/// A run on the tree of a green run reuses the checks whose result follows
/// from the tree and the host (#2090's F7).
#[test]
fn a_run_on_the_same_tree_reuses_the_checks_that_follow_from_the_tree_and_the_host() {
    let (workspace, _folder, records, first) = gated_with_a_green_lint("2027-01-30");

    let again = plan(&workspace, &records);

    assert_eq!(again.previous, first.head);
    for step in TREE_ONLY {
        assert_eq!(action(&again, step).0, "reuse", "{step}");
    }
}

/// Lint's register check reads the day: an Exception past its expiry fails
/// it (ADR 0140). A green lint from the Exception's last day is reused until
/// midnight UTC, and lint runs again from the day after.
#[test]
fn lint_reruns_on_the_same_tree_once_an_exception_of_the_register_has_expired() {
    let (workspace, _folder, records, _first) = gated_with_a_green_lint("2027-01-15");

    let at_the_days_last_second = plan_at(&workspace, &records, "1800057599");
    let at_midnight = plan_at(&workspace, &records, "1800057600");

    assert_eq!(action(&at_the_days_last_second, "lint").0, "reuse");
    assert_eq!(action(&at_midnight, "lint").0, "run");
    assert_eq!(
        at_midnight.why["lint"],
        "lint's register check: X1: it expired on 2027-01-15: remove the gap, or ask the owner to extend it"
    );
}

/// With a GitHub token in the run's environment, lint's zizmor also runs
/// its online audits, which read GitHub: the gate's plan is told so
/// (`--zizmor-online`), and a green lint is not reused.
#[test]
fn lint_reruns_on_the_same_tree_when_its_zizmor_audits_online() {
    let (workspace, _folder, records, _first) = gated_with_a_green_lint("2027-01-30");

    let online = plan_with(&workspace, &records, "CI=", &["--zizmor-online"]);

    assert_eq!(action(&online, "lint").0, "run");
    assert_eq!(
        online.why["lint"],
        "lint's zizmor audits online with the run's GitHub token, so lint reads more than the tree"
    );
}

const ISSUE: &str = "/repos/goodfetch/goodfetch/issues/1601";

/// Lint's quarantine check reads each quarantined test's issue from GitHub
/// (ADR 0064): a green lint is not reused once that issue is closed.
#[test]
fn lint_reruns_on_the_same_tree_once_a_quarantined_tests_issue_is_closed() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write("TESTING.md", crate::register::CLEAN_TESTING.as_bytes());
    workspace.write(
        "quarantine.toml",
        b"[[test]]\nrunner = \"nextest\"\nid = \"goodfetch-engine tests::flaky\"\nissue = 1601\ndate = 2026-11-13\n",
    );
    commit_all(&workspace, "Quarantine a test");
    let open = GitHub::serving(&[(
        ISSUE,
        r#"{ "number": 1601, "state": "open", "labels": [ { "name": "flaky-test" } ] }"#,
    )]);
    let closed = GitHub::serving(&[(
        ISSUE,
        r#"{ "number": 1601, "state": "closed", "labels": [ { "name": "flaky-test" } ] }"#,
    )]);
    let first = plan_reading_issues(&workspace, &records, &open);
    record(&workspace, &records, &first.head, "lint", "pass");

    let while_open = plan_reading_issues(&workspace, &records, &open);
    let after_the_close = plan_reading_issues(&workspace, &records, &closed);

    assert_eq!(action(&while_open, "lint").0, "reuse");
    assert_eq!(action(&after_the_close, "lint").0, "run");
    assert_eq!(
        after_the_close.why["lint"],
        "lint's quarantine check: goodfetch-engine tests::flaky: issue #1601 is closed; a quarantined test needs its open `flaky-test` issue (ADR 0064)"
    );
}

/// The plan reads the quarantined tests' issues only for a green lint it
/// would otherwise reuse: a run on another tree, which reruns lint whatever
/// they read, asks GitHub nothing.
#[test]
fn a_run_that_reruns_lint_reads_no_quarantined_tests_issue() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write("TESTING.md", crate::register::CLEAN_TESTING.as_bytes());
    workspace.write(
        "quarantine.toml",
        b"[[test]]\nrunner = \"nextest\"\nid = \"goodfetch-engine tests::flaky\"\nissue = 1601\ndate = 2026-11-13\n",
    );
    commit_all(&workspace, "Quarantine a test");
    let github = GitHub::serving(&[(
        ISSUE,
        r#"{ "number": 1601, "state": "open", "labels": [ { "name": "flaky-test" } ] }"#,
    )]);
    let first = plan_reading_issues(&workspace, &records, &github);
    record(&workspace, &records, &first.head, "lint", "pass");
    workspace.write("docs/index.md", b"# Changed\n");
    commit_all(&workspace, "Change the docs");

    let second = plan_reading_issues(&workspace, &records, &github);

    assert_eq!(action(&second, "lint").0, "run");
    assert_eq!(github.requests().len(), 0, "{:?}", github.requests());
}

/// A run whose environment differs (another test profile, selection or
/// engine) reuses nothing (the re-review's N1).
#[test]
fn a_run_in_another_environment_reuses_nothing() {
    let (workspace, _folder, records) = gated_repository(&[]);
    first_run(&workspace, &records);

    let second = plan_in(&workspace, &records, "CI=1");

    assert!(
        second.steps.values().all(|(action, _)| action == "run"),
        "{:?}",
        second.steps
    );
}

/// The host fingerprint covers the tools the gate's steps run that no
/// package of the system holds: a result recorded with one of them at one
/// version is not reused with it at another, whichever step ran it. A
/// stand-in first on `PATH` (and before Cargo's own folder, where Cargo
/// looks for its subcommands) gives the other version.
#[rstest]
#[case::just("just")]
#[case::rustups_toolchains("rustup")]
#[case::typos("typos")]
#[case::zizmor("zizmor")]
#[case::actionlint("actionlint")]
#[case::gitleaks("gitleaks")]
#[case::osv_scanner("osv-scanner")]
#[case::cargo_deny("cargo-deny")]
#[case::cargo_shear("cargo-shear")]
#[case::cargo_hakari("cargo-hakari")]
#[case::cargo_hack("cargo-hack")]
#[case::cargo_fuzz("cargo-fuzz")]
#[case::cargo_audit("cargo-audit")]
#[case::cargo_insta("cargo-insta")]
fn a_run_with_a_tool_at_another_version_reuses_nothing(#[case] tool: &str) {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = first_run(&workspace, &records);
    let stand_in = tempfile::tempdir().unwrap();
    write_executable(
        &stand_in.path().join(tool),
        "#!/bin/sh\necho 'another version'\n",
    );
    let path = path_starting_with(stand_in.path());

    let second = parse(&planned_at(
        &workspace,
        &records,
        NOW,
        "CI=",
        &[],
        &[("PATH", &path)],
    ));

    assert_eq!(second.previous, first);
    assert!(
        second.steps.values().all(|(action, _)| action == "run"),
        "{:?}",
        second.steps
    );
    assert_eq!(second.why["perf"], host_changed_since(&first));
}

/// `PATH` with `folder` first, then Cargo's own folder, where Cargo looks
/// for its subcommands before `PATH`, then the host's.
fn path_starting_with(folder: &Path) -> std::ffi::OsString {
    let cargo_home = std::env::var_os("CARGO_HOME").map_or_else(
        || Path::new(&std::env::var_os("HOME").unwrap()).join(".cargo"),
        PathBuf::from,
    );
    std::env::join_paths(
        [folder.to_owned(), cargo_home.join("bin")]
            .into_iter()
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap()
}

/// The reason every step gives after the host changed since the run on
/// `head`.
fn host_changed_since(head: &str) -> String {
    format!(
        "the host or its tools changed since the run on {}",
        &head[..10]
    )
}

/// The stand-ins above change the host by what they answer, never by
/// standing first on `PATH`: a command no step runs, put there the same way,
/// leaves the gate's results reused.
#[test]
fn a_command_no_step_runs_first_on_the_path_keeps_the_gates_results() {
    let (workspace, _folder, records) = gated_repository(&[]);
    first_run(&workspace, &records);
    let stand_in = tempfile::tempdir().unwrap();
    write_executable(
        &stand_in.path().join("no-tool-of-the-gate"),
        "#!/bin/sh\necho 'another version'\n",
    );
    let path = path_starting_with(stand_in.path());

    let second = parse(&planned_at(
        &workspace,
        &records,
        NOW,
        "CI=",
        &[],
        &[("PATH", &path)],
    ));

    assert_eq!(action(&second, "perf").0, "reuse");
}

/// The proofs run Miri with no skip, so they fail on a toolchain without
/// its `miri` component: a result recorded with it is not reused once a
/// toolchain rustup holds has other components. `rustup` is a stand-in
/// that lists one toolchain and, as its components, the file beside it.
#[test]
fn a_run_after_a_toolchain_lost_a_component_reuses_nothing() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let stand_in = tempfile::tempdir().unwrap();
    let components = stand_in.path().join("components");
    write_executable(
        &stand_in.path().join("rustup"),
        &format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n'toolchain list') echo 'nightly-2026-10-05-x86_64-unknown-linux-gnu' ;;\n'component list') cat '{}' ;;\nesac\n",
            components.display()
        ),
    );
    std::fs::write(&components, "miri-x86_64-unknown-linux-gnu\nrust-src\n").unwrap();
    let path = path_starting_with(stand_in.path());
    let plan_now = || {
        parse(&planned_at(
            &workspace,
            &records,
            NOW,
            "CI=",
            &[],
            &[("PATH", &path)],
        ))
    };
    let first = plan_now();
    record(&workspace, &records, &first.head, "perf", "pass");
    assert_eq!(action(&plan_now(), "perf").0, "reuse");

    std::fs::write(&components, "rust-src\n").unwrap();
    let after = plan_now();

    assert_eq!(action(&after, "perf").0, "run");
    assert_eq!(after.why["perf"], host_changed_since(&first.head));
}

/// Lint's pyright and mypy, and the suites' pytest, run on the Python of
/// the project's environment, which no file of the tree pins and the
/// system's `python3` need not be: a result recorded with the environment
/// on one Python is not reused with it on another.
#[test]
fn a_run_with_the_python_environment_on_another_python_reuses_nothing() {
    let (workspace, _folder, records) = gated_repository(&[]);
    // The environment is no part of the tree, as in the repository.
    workspace.write(".git/info/exclude", b".venv/\n");
    workspace.write(".venv/pyvenv.cfg", b"version_info = 3.12.12\n");
    let first = first_run(&workspace, &records);
    assert_eq!(action(&plan(&workspace, &records), "perf").0, "reuse");

    workspace.write(".venv/pyvenv.cfg", b"version_info = 3.14.4\n");
    let second = plan(&workspace, &records);

    assert_eq!(action(&second, "perf").0, "run");
    assert_eq!(second.why["perf"], host_changed_since(&first));
}

/// Git also ignores what two files outside the tree name: the repository's
/// own `info/exclude` and the user's. Lint's check of `.gitignore`
/// (ci/ignored-sources.sh) and the tools that skip ignored files read them,
/// so a result recorded under one of them is not reused once it changed.
#[test]
fn a_run_after_the_ignore_file_of_the_repository_changed_reuses_nothing() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = first_run(&workspace, &records);

    workspace.write(".git/info/exclude", b"/target\n");
    let second = plan(&workspace, &records);

    assert_eq!(action(&second, "perf").0, "run");
    assert_eq!(second.why["perf"], host_changed_since(&first));
}

#[test]
fn a_run_after_the_ignore_file_of_the_user_changed_reuses_nothing() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let user = tempfile::tempdir().unwrap();
    let ignore = user.path().join("ignore");
    let config = user.path().join("gitconfig");
    std::fs::write(
        &config,
        format!("[core]\n\texcludesFile = {}\n", ignore.display()),
    )
    .unwrap();
    std::fs::write(&ignore, "*.orig\n").unwrap();
    let plan_now = || {
        parse(&planned_at(
            &workspace,
            &records,
            NOW,
            "CI=",
            &[],
            &[("GIT_CONFIG_GLOBAL", config.as_os_str())],
        ))
    };
    let first = plan_now();
    record(&workspace, &records, &first.head, "perf", "pass");
    assert_eq!(action(&plan_now(), "perf").0, "reuse");

    std::fs::write(&ignore, "*.orig\n/target\n").unwrap();
    let after = plan_now();

    assert_eq!(action(&after, "perf").0, "run");
    assert_eq!(after.why["perf"], host_changed_since(&first.head));
}

/// A crate is judged by the dep-info of each of its test targets: one with
/// none reruns, though another target's dep-info exists (the re-review's N3).
#[test]
fn a_crate_with_a_test_target_whose_dep_info_is_missing_reruns() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write("crates/goodfetch-engine/tests/it.rs", b"//! Its tests.\n");
    commit_all(&workspace, "Add the engine's integration tests");
    first_run(&workspace, &records);
    workspace.write("docs/index.md", b"# Changed\n");
    commit_all(&workspace, "Change the docs");

    let second = plan(&workspace, &records);

    let rerun = members(&second, "pre-merge");
    assert!(rerun.contains("goodfetch-engine"), "{rerun:?}");
    assert!(
        second.why["pre-merge"].contains("no dep-info of its test target `it`"),
        "{}",
        second.why["pre-merge"]
    );
}

/// A change to a crate's `tests/` folder after a recorded run reruns that
/// crate's tests alone: its test target's dep-info shows the target read
/// the file, and no library, executable or benchmark build did, so neither
/// its dependant nor perf reruns (#2238).
#[test]
fn a_change_to_a_crates_tests_folder_reruns_its_tests_alone() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write("crates/goodfetch-engine/tests/it.rs", b"//! Its tests.\n");
    commit_all(&workspace, "Add the engine's integration tests");
    let file = target(&records).join("debug/deps/it-0.d");
    let source = workspace.path().join("crates/goodfetch-engine/tests/it.rs");
    std::fs::write(&file, format!("{}: {}\n", file.display(), source.display())).unwrap();
    first_run(&workspace, &records);
    workspace.write(
        "crates/goodfetch-engine/tests/it.rs",
        b"//! Its tests, reworded.\n",
    );
    commit_all(&workspace, "Reword the engine's integration tests");

    let second = plan(&workspace, &records);

    let mut expected: BTreeSet<String> =
        SHUFFLED.iter().map(|member| (*member).to_owned()).collect();
    expected.insert("goodfetch-engine".to_owned());
    assert_eq!(
        (
            action(&second, "pre-merge").0,
            members(&second, "pre-merge")
        ),
        ("part", expected),
        "{}",
        second.why["pre-merge"]
    );
    assert_eq!(action(&second, "perf").0, "reuse", "{}", second.why["perf"]);
}

/// A changed file of a crate's `tests/` folder that a file outside it names
/// reruns the crate with its dependants, and the log gives that reason, not
/// the narrower one its integration tests' dep-info gives (#2238).
#[test]
fn a_tests_file_another_file_names_reruns_its_crate_with_its_dependants() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write("crates/goodfetch-engine/tests/it.rs", b"//! Its tests.\n");
    workspace.write(
        "shared/runners.toml",
        b"runner = \"crates/goodfetch-engine/tests/it.rs::every_case\"\n",
    );
    commit_all(
        &workspace,
        "Add the engine's integration tests and their runners",
    );
    let file = target(&records).join("debug/deps/it-0.d");
    let source = workspace.path().join("crates/goodfetch-engine/tests/it.rs");
    std::fs::write(&file, format!("{}: {}\n", file.display(), source.display())).unwrap();
    first_run(&workspace, &records);
    workspace.write(
        "crates/goodfetch-engine/tests/it.rs",
        b"//! Its tests, reworded.\n",
    );
    commit_all(&workspace, "Reword the engine's integration tests");

    let second = plan(&workspace, &records);

    let rerun = members(&second, "pre-merge");
    assert!(rerun.contains("goodfetch-frontier"), "{rerun:?}");
    let why = &second.why["pre-merge"];
    assert!(
        why.contains("shared/runners.toml names crates/goodfetch-engine/tests/it.rs"),
        "{why}"
    );
}

/// A nextest JUnit file of `cases`, each `(binary ID, test name, passed)`,
/// as nextest writes it, or, when not `whole`, one that declares a case
/// more than it holds, as a run stopped while writing it leaves it.
fn nextest_junit(cases: &[(&str, &str, bool)], whole: bool) -> String {
    use std::fmt::Write as _;
    let failures = cases.iter().filter(|(_, _, passed)| !passed).count();
    let declared = cases.len() + usize::from(!whole);
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"nextest-run\" tests=\"{declared}\" failures=\"{failures}\" errors=\"0\">\n"
    );
    for (binary, name, passed) in cases {
        let _ = writeln!(text, "<testsuite name=\"{binary}\" tests=\"1\">");
        if *passed {
            let _ = writeln!(text, "<testcase name=\"{name}\" classname=\"{binary}\"/>");
        } else {
            let _ = writeln!(
                text,
                "<testcase name=\"{name}\" classname=\"{binary}\"><failure message=\"failed\">failed</failure></testcase>"
            );
        }
        text.push_str("</testsuite>\n");
    }
    text.push_str("</testsuites>\n");
    text
}

/// Records a step's result with the evidence `arguments` add.
fn record_with(workspace: &Workspace, records: &Path, head: &str, step: &str, arguments: &[&str]) {
    let records = records.to_string_lossy();
    let mut command = vec![
        "gate",
        "record",
        "--records",
        &records,
        "--head",
        head,
        "--now",
        NOW,
        "--step",
        step,
        "--status",
        "fail",
    ];
    command.extend(arguments);
    let outcome = workspace.xtask(&command);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// Records the first run's Quick suite red with `junit` as its nextest
/// file, beside the records, and perf, TLC, visual and the Pre-merge suite
/// green; then commits a change that reaches no member.
fn red_quick_suite_then_a_docs_change(workspace: &Workspace, records: &Path, junit: &str) {
    red_suite_then_a_docs_change(workspace, records, "quick", junit);
}

/// Records the first run's `suite`, the Quick or the Pre-merge suite, red
/// with `junit` as its nextest file, beside the records, and the other
/// suite, perf, TLC and visual green; then commits a change that reaches no
/// member.
fn red_suite_then_a_docs_change(workspace: &Workspace, records: &Path, suite: &str, junit: &str) {
    let first = plan(workspace, records);
    for step in ["perf", "tlc", "visual", "pre-merge", "quick"] {
        if step != suite {
            record(workspace, records, &first.head, step, "pass");
        }
    }
    let file = records.with_file_name(format!("{suite}-junit.xml"));
    std::fs::write(&file, junit).unwrap();
    record_with(
        workspace,
        records,
        &first.head,
        suite,
        &["--junit", &file.to_string_lossy()],
    );
    workspace.write("docs/index.md", b"# Changed\n");
    commit_all(workspace, "Change the docs");
}

/// A red Quick suite records red only the crates with a failing test: the
/// others passed in full, so the next run reuses them where the change
/// since does not reach them (the test-gate audit's option 4).
#[test]
fn a_red_quick_suite_records_red_only_the_crates_with_a_failing_test() {
    let (workspace, _folder, records) = gated_repository(&[]);
    red_quick_suite_then_a_docs_change(
        &workspace,
        &records,
        &nextest_junit(
            &[
                ("goodfetch-engine", "dispatch::routes", false),
                ("goodfetch-frontier", "claim::slices", true),
            ],
            true,
        ),
    );

    let second = plan(&workspace, &records);

    assert_eq!(action(&second, "quick").0, "part");
    let rerun = members(&second, "quick");
    assert!(rerun.contains("goodfetch-engine"), "{rerun:?}");
    for passed in [
        "goodfetch-frontier",
        "goodfetch-conformance",
        "goodfetch-sink-s3",
        "goodfetch-proving-ground",
    ] {
        assert!(!rerun.contains(passed), "{passed} reran: {rerun:?}");
    }
}

/// A red Pre-merge suite, red in nextest alone, records red the crates with
/// a failing test and those holding insta snapshots, which cargo-insta
/// judges unreferenced only once every test passed, and green the others
/// (the test-gate audit's option 4b).
#[test]
fn a_red_pre_merge_suite_records_green_each_crate_that_passed_in_full_but_one_holding_snapshots() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write(
        "crates/goodfetch-conformance/tests/snapshots/it__a_case.snap",
        b"---\nsource: tests/it.rs\n---\na case\n",
    );
    commit_all(&workspace, "Add a snapshot");
    red_suite_then_a_docs_change(
        &workspace,
        &records,
        "pre-merge",
        &nextest_junit(
            &[
                ("goodfetch-engine", "dispatch::routes", false),
                ("goodfetch-frontier", "claim::slices", true),
                ("goodfetch-conformance::it", "a_case", true),
            ],
            true,
        ),
    );

    let second = plan(&workspace, &records);

    assert_eq!(action(&second, "pre-merge").0, "part");
    let rerun = members(&second, "pre-merge");
    for red in ["goodfetch-engine", "goodfetch-conformance"] {
        assert!(rerun.contains(red), "{red} reused: {rerun:?}");
    }
    for passed in [
        "goodfetch-frontier",
        "goodfetch-sink-s3",
        "goodfetch-proving-ground",
    ] {
        assert!(!rerun.contains(passed), "{passed} reran: {rerun:?}");
    }
}

/// When the step's nextest file does not account for its failure, every
/// member records red, as without the file: no test failed in it, so the
/// failure was elsewhere; it is unfinished; or a failing test is of a crate
/// the step did not run.
#[rstest::rstest]
#[case::no_failing_test(&[("goodfetch-engine", "dispatch::routes", true)], true)]
#[case::unfinished(&[("goodfetch-engine", "dispatch::routes", false)], false)]
#[case::a_crate_the_step_did_not_run(&[("goodfetch-elsewhere", "a::test", false)], true)]
fn a_red_quick_suite_its_file_does_not_account_for_records_every_member_red(
    #[case] cases: &[(&str, &str, bool)],
    #[case] whole: bool,
) {
    let (workspace, _folder, records) = gated_repository(&[]);
    red_quick_suite_then_a_docs_change(&workspace, &records, &nextest_junit(cases, whole));

    let second = plan(&workspace, &records);

    assert_eq!(action(&second, "quick"), ("run", "-"));
}

/// A red stress run records green each crate whose every selected test
/// passed its count of times, and red the crate of a test that failed or
/// ran fewer times, across the run's files and its iterations' suites.
#[test]
fn a_red_stress_run_records_green_each_crate_whose_selected_tests_all_passed_their_count() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = plan(&workspace, &records);
    for step in ["perf", "tlc", "visual", "pre-merge", "quick"] {
        record(&workspace, &records, &first.head, step, "pass");
    }
    let folder = records.with_file_name("repeat-3");
    std::fs::create_dir_all(&folder).unwrap();
    let red = records.with_file_name("stress-red-commands.tsv");
    std::fs::write(&red, NEXTEST_TEST_FAILED).unwrap();
    let selection = records.with_file_name("stress-selection.tsv");
    std::fs::write(
        &selection,
        "3\tnextest\tgoodfetch-engine dispatch::routes\n\
         3\tnextest\tgoodfetch-frontier::it claim::slices\n\
         3\tvitest\tsdk > a case\n",
    )
    .unwrap();
    // The engine's test passed twice of three times; the frontier's three
    // times, its iterations in a batch file of their own.
    let engine = [
        ("goodfetch-engine@stress-0", "dispatch::routes", true),
        ("goodfetch-engine@stress-1", "dispatch::routes", true),
        ("goodfetch-engine@stress-2", "dispatch::routes", false),
    ];
    let frontier = [
        ("goodfetch-frontier::it@stress-0", "claim::slices", true),
        ("goodfetch-frontier::it@stress-1", "claim::slices", true),
        ("goodfetch-frontier::it@stress-2", "claim::slices", true),
    ];
    std::fs::write(
        folder.join("nextest-ci.1.xml"),
        nextest_junit(&engine, true),
    )
    .unwrap();
    std::fs::write(
        folder.join("nextest-ci.2.xml"),
        nextest_junit(&frontier, true),
    )
    .unwrap();
    record_with(
        &workspace,
        &records,
        &first.head,
        "stress",
        &[
            "--selection",
            &selection.to_string_lossy(),
            "--red-commands",
            &red.to_string_lossy(),
            "--started",
            STARTED,
            "--junit",
            &folder.to_string_lossy(),
        ],
    );
    workspace.write("docs/index.md", b"# Changed\n");
    commit_all(&workspace, "Change the docs");

    let second = plan(&workspace, &records);

    assert_eq!(action(&second, "stress").0, "part");
    let rerun = members(&second, "stress");
    assert!(rerun.contains("goodfetch-engine"), "{rerun:?}");
    assert!(!rerun.contains("goodfetch-frontier"), "{rerun:?}");
}

/// The stress run of the fixture's `xtask`, engine and frontier tests, and a
/// Vitest test, each its 20 times.
const SELECTION: &str = "20\tnextest\txtask::it a::flaky\n\
                         20\tnextest\tgoodfetch-engine b::steady\n\
                         20\tnextest\tgoodfetch-frontier::it c::steady\n\
                         20\tvitest\tsdk > a case\n";

/// What `just repeat --until-red` says of a nextest run with a failing test.
const NEXTEST_TEST_FAILED: &str = "nextest\t100\n";

/// When the fixture's stress runs begin: before `NOW`, when they are recorded.
const STARTED: &str = "1799999000";

/// Records `plan`'s stress run red with `selection`, each of `iterations` the
/// nextest cases one iteration ran, as `just repeat --until-red` keeps them.
fn red_stress_run(
    workspace: &Workspace,
    records: &Path,
    plan: &Plan,
    selection: &str,
    iterations: &[Vec<(&str, &str, bool)>],
) {
    red_stress_run_saying(
        workspace,
        records,
        plan,
        Some(selection),
        iterations,
        Some(NEXTEST_TEST_FAILED),
    );
}

/// As [`red_stress_run`], with the commands the run says went red, if it says,
/// and its selection, if it left one.
fn red_stress_run_saying(
    workspace: &Workspace,
    records: &Path,
    plan: &Plan,
    selection: Option<&str>,
    iterations: &[Vec<(&str, &str, bool)>],
    red_commands: Option<&str>,
) {
    let folder = records.with_file_name(format!("repeat-of-{}", plan.head));
    std::fs::create_dir_all(&folder).unwrap();
    for (iteration, cases) in iterations.iter().enumerate() {
        std::fs::write(
            folder.join(format!("nextest-ci.1.{iteration}.xml")),
            nextest_junit(cases, true),
        )
        .unwrap();
    }
    let file = folder.with_extension("selection");
    if let Some(selection) = selection {
        std::fs::write(&file, selection).unwrap();
    }
    let red = folder.with_extension("red");
    if let Some(red_commands) = red_commands {
        std::fs::write(&red, red_commands).unwrap();
    }
    record_with(
        workspace,
        records,
        &plan.head,
        "stress",
        &[
            "--selection",
            &file.to_string_lossy(),
            "--red-commands",
            &red.to_string_lossy(),
            "--started",
            STARTED,
            "--junit",
            &folder.to_string_lossy(),
        ],
    );
}

/// The iterations of a run on SELECTION that went red on xtask's test on the
/// last of `passes` + 1: the engine's and the frontier's passed each time.
fn red_on_xtask_after(passes: usize) -> Vec<Vec<(&'static str, &'static str, bool)>> {
    (0..=passes)
        .map(|iteration| {
            vec![
                ("xtask::it@stress-0", "a::flaky", iteration < passes),
                ("goodfetch-engine@stress-0", "b::steady", true),
                ("goodfetch-frontier::it@stress-0", "c::steady", true),
            ]
        })
        .collect()
}

/// The fixture with xtask, on a first run with every step green but the stress
/// run, red on xtask's test on its 7th pass; then a fix to that test's file.
/// Returns the first head.
fn red_on_xtask_then_its_fix(workspace: &Workspace, records: &Path) -> String {
    add_xtask(workspace, records);
    let first = plan(workspace, records);
    for step in first.steps.keys() {
        if step != "stress" {
            record(workspace, records, &first.head, step, "pass");
        }
    }
    red_stress_run(
        workspace,
        records,
        &first,
        SELECTION,
        &red_on_xtask_after(6),
    );
    workspace.write("xtask/tests/it/stress.rs", b"//! The fix.\n");
    commit_all(workspace, "Fix the flaky test");
    first.head
}

/// The earned file of the run on `head`, as `<member> <origin> <passes> <test>`
/// lines, or none.
fn earned(records: &Path, head: &str) -> Vec<String> {
    std::fs::read_to_string(records.join(format!("{head}.earned")))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [member, origin, _at, passes, test] = fields.as_slice() else {
                panic!("an earned line has five fields, not {line:?}");
            };
            format!("{member} {origin} {passes} {test}")
        })
        .collect()
}

/// #2233, as on #2223's gate at f5841c69f: one test failed in xtask on the
/// 7th pass. The fix's plan reruns the members it reaches and those short of
/// their passes, each of which keeps the clean passes it has; the member with
/// the red test keeps none, and the Vitest member, which runs in the head's
/// order, none. Nothing else reruns.
#[test]
fn a_red_stress_run_leaves_its_clean_members_their_passes_and_the_fix_reruns_what_it_reaches() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = red_on_xtask_then_its_fix(&workspace, &records);

    let second = plan(&workspace, &records);

    assert_eq!(action(&second, "stress").0, "part", "{:?}", second.why);
    let mut rerun: BTreeSet<String> = SHUFFLED.iter().map(|member| (*member).to_owned()).collect();
    rerun.extend(["xtask", "goodfetch-engine", "goodfetch-frontier"].map(str::to_owned));
    assert_eq!(members(&second, "stress"), rerun);
    assert_eq!(
        earned(&records, &second.head),
        [
            format!("goodfetch-engine {first} 7 goodfetch-engine b::steady"),
            format!("goodfetch-frontier {first} 7 goodfetch-frontier::it c::steady"),
        ]
    );
    // The fix reaches xtask's tests alone in the other suites.
    let mut suites: BTreeSet<String> = SHUFFLED.iter().map(|member| (*member).to_owned()).collect();
    suites.insert("xtask".to_owned());
    assert_eq!(members(&second, "quick"), suites);
    assert_eq!(members(&second, "pre-merge"), suites);
    for step in ["perf", "visual", "tlc", "kennel", "msrv", "miri"] {
        assert_eq!(action(&second, step).0, "reuse", "{step}: {:?}", second.why);
    }
}

/// A later red run adds a clean member's passes to what it had, keeping the
/// head and the time of its earliest; a member that went red in it loses
/// them, so a planted failure both turns the run red and starts the member's
/// count again.
#[test]
fn a_later_red_run_adds_to_clean_passes_and_a_red_test_takes_its_members_away() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = red_on_xtask_then_its_fix(&workspace, &records);
    let second = plan(&workspace, &records);
    let cases = |engine: bool| {
        vec![
            ("goodfetch-engine@stress-0", "b::steady", engine),
            ("goodfetch-frontier::it@stress-0", "c::steady", true),
        ]
    };
    // The 13 passes they owe; the engine's test, planted red, fails the 4th.
    red_stress_run(
        &workspace,
        &records,
        &second,
        "13\tnextest\tgoodfetch-engine b::steady\n13\tnextest\tgoodfetch-frontier::it c::steady\n",
        &[cases(true), cases(true), cases(true), cases(false)],
    );

    assert_eq!(
        earned(&records, &second.head),
        [format!(
            "goodfetch-frontier {first} 11 goodfetch-frontier::it c::steady"
        )]
    );
    let third = plan(&workspace, &records);
    assert!(members(&third, "stress").contains("goodfetch-engine"));
    assert_eq!(
        earned(&records, &third.head),
        [format!(
            "goodfetch-frontier {first} 11 goodfetch-frontier::it c::steady"
        )],
        "{:?}",
        third.why
    );
}

/// A red run whose files do not account for its failure records every member
/// red, and with no clean pass it can vouch for: the passes it was handed go,
/// and the crate behind a crash keeps nothing. Nextest went red with no failing
/// test in its files, or exited as no test failure does, or the run says
/// nothing of what went red, or its selection is gone.
#[rstest::rstest]
#[case::nextest_red_and_no_failing_test(Some("nextest\t100\n"), true, false)]
#[case::nextest_ended_as_no_failure_does(Some("nextest\t101\n"), true, false)]
#[case::another_batch_ended_as_no_failure_does(Some("nextest\t100\nnextest\t101\n"), true, true)]
#[case::no_word_of_what_went_red(None, true, false)]
#[case::no_selection(Some("nextest\t100\n"), false, true)]
fn a_red_stress_run_its_files_do_not_account_for_leaves_no_earned_passes(
    #[case] red_commands: Option<&str>,
    #[case] has_selection: bool,
    #[case] a_test_failed: bool,
) {
    let (workspace, _folder, records) = gated_repository(&[]);
    red_on_xtask_then_its_fix(&workspace, &records);
    let second = plan(&workspace, &records);
    assert_ne!(earned(&records, &second.head), Vec::<String>::new());
    let cases = vec![
        ("goodfetch-engine@stress-0", "b::steady", true),
        (
            "goodfetch-frontier::it@stress-0",
            "c::steady",
            !a_test_failed,
        ),
    ];

    red_stress_run_saying(
        &workspace,
        &records,
        &second,
        has_selection.then_some(
            "13\tnextest\tgoodfetch-engine b::steady\n13\tnextest\tgoodfetch-frontier::it c::steady\n",
        ),
        &[cases.clone(), cases],
        red_commands,
    );

    assert_eq!(earned(&records, &second.head), Vec::<String>::new());
    let third = plan(&workspace, &records);
    assert_eq!(earned(&records, &third.head), Vec::<String>::new());
    assert!(
        members(&third, "stress").contains("goodfetch-engine"),
        "{:?}",
        third.why
    );
}

/// Vitest, Playwright and pytest run after nextest and each in its own process,
/// so a red one says nothing of the crates: each that passed its count records
/// green, and one a later count left short keeps what it has.
#[test]
fn a_red_run_of_another_runner_leaves_the_crates_what_their_files_show() {
    let (workspace, _folder, records) = gated_repository(&[]);
    add_xtask(&workspace, &records);
    let first = plan(&workspace, &records);
    for step in first.steps.keys() {
        if step != "stress" {
            record(&workspace, &records, &first.head, step, "pass");
        }
    }
    let cases = vec![("goodfetch-engine@stress-0", "b::steady", true)];

    red_stress_run_saying(
        &workspace,
        &records,
        &first,
        Some(
            "2\tnextest\tgoodfetch-engine b::steady\n2\tvitest\tsdk > a case\n\
             20\tnextest\tgoodfetch-frontier::it c::steady\n",
        ),
        &[cases.clone(), cases],
        Some("vitest\t1\n"),
    );

    let recorded = std::fs::read_to_string(records.join(format!("{}.tsv", first.head))).unwrap();
    let status = |member: &str| {
        let unit = format!("stress:{member}\t");
        recorded
            .lines()
            .rfind(|line| line.starts_with(&unit))
            .map(|line| line.split('\t').nth(1).unwrap().to_owned())
    };
    assert_eq!(status("goodfetch-engine").as_deref(), Some("pass"));
    assert_eq!(status("goodfetch-frontier").as_deref(), Some("fail"));
}

/// An earned file that cannot be read earns nothing: the plan still runs, and
/// the members run their whole count.
#[test]
fn an_unreadable_earned_file_earns_nothing() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = red_on_xtask_then_its_fix(&workspace, &records);
    std::fs::write(records.join(format!("{first}.earned")), "not a line\n").unwrap();

    let second = plan(&workspace, &records);

    assert!(
        members(&second, "stress").contains("goodfetch-engine"),
        "{:?}",
        second.why
    );
    assert_eq!(earned(&records, &second.head), Vec::<String>::new());
}

/// `xtask stress select` reads the file the plan wrote; a green run then
/// records each member that kept passes green, with the start of the first run
/// that gave one and the head it ran on, so the age limit counts from it and
/// the backstop finds it.
#[test]
fn a_green_run_keeps_where_its_earned_passes_came_from() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = red_on_xtask_then_its_fix(&workspace, &records);
    let second = plan(&workspace, &records);

    record(&workspace, &records, &second.head, "stress", "pass");

    let recorded = std::fs::read_to_string(records.join(format!("{}.tsv", second.head))).unwrap();
    let line = recorded
        .lines()
        .rfind(|line| line.starts_with("stress:goodfetch-engine\t"))
        .unwrap();
    let fields: Vec<&str> = line.split('\t').collect();
    assert_eq!(fields[1], "pass", "{line}");
    assert_eq!(fields[2], second.head, "{line}");
    assert_eq!(fields[4], STARTED, "{line}");
    assert_eq!(fields[5], first, "{line}");
    assert_eq!(earned(&records, &second.head), Vec::<String>::new());
}

/// The age limit counts from the start of the run that gave the first pass,
/// which is no later than the pass, not from the end of that run: a run that
/// began past the limit and ended within it leaves nothing to count.
#[test]
fn earned_passes_age_from_the_start_of_the_run_that_gave_them() {
    let (workspace, _folder, records) = gated_repository(&[]);
    add_xtask(&workspace, &records);
    let first = plan(&workspace, &records);
    for step in first.steps.keys() {
        if step != "stress" {
            record(&workspace, &records, &first.head, step, "pass");
        }
    }
    let folder = records.with_file_name("repeat-of-first");
    std::fs::create_dir_all(&folder).unwrap();
    for (iteration, cases) in red_on_xtask_after(6).iter().enumerate() {
        std::fs::write(
            folder.join(format!("nextest-ci.1.{iteration}.xml")),
            nextest_junit(cases, true),
        )
        .unwrap();
    }
    let selection = folder.with_extension("selection");
    std::fs::write(&selection, SELECTION).unwrap();
    let red = folder.with_extension("red");
    std::fs::write(&red, NEXTEST_TEST_FAILED).unwrap();
    let a_day_and_more_ago = NOW.parse::<u64>().unwrap() - 24 * 60 * 60 - 1;

    record_with(
        &workspace,
        &records,
        &first.head,
        "stress",
        &[
            "--selection",
            &selection.to_string_lossy(),
            "--red-commands",
            &red.to_string_lossy(),
            "--started",
            &a_day_and_more_ago.to_string(),
            "--junit",
            &folder.to_string_lossy(),
        ],
    );
    assert_ne!(earned(&records, &first.head), Vec::<String>::new());
    workspace.write("xtask/tests/it/stress.rs", b"//! The fix.\n");
    commit_all(&workspace, "Fix the flaky test");

    let second = plan(&workspace, &records);

    assert_eq!(earned(&records, &second.head), Vec::<String>::new());
}

/// The selection counted the earned file `xtask gate plan` wrote, and only the
/// record can mark a green result as resting on those passes for the backstop:
/// a file that can no longer be read, or that lost a member the plan kept, fails
/// the record, and no green result lands without its mark.
#[rstest::rstest]
#[case::cannot_be_read(Some("not a line\n"))]
#[case::is_gone(None)]
fn a_stress_result_is_not_recorded_when_its_earned_file_no_longer_says(
    #[case] now_holds: Option<&str>,
) {
    let (workspace, _folder, records) = gated_repository(&[]);
    red_on_xtask_then_its_fix(&workspace, &records);
    let second = plan(&workspace, &records);
    let file = records.join(format!("{}.earned", second.head));
    match now_holds {
        Some(text) => std::fs::write(&file, text).unwrap(),
        None => std::fs::remove_file(&file).unwrap(),
    }

    let outcome = workspace.xtask(&[
        "gate",
        "record",
        "--records",
        &records.to_string_lossy(),
        "--head",
        &second.head,
        "--now",
        NOW,
        "--step",
        "stress",
        "--status",
        "pass",
    ]);

    assert_ne!(outcome.code, 0, "{}", outcome.stdout);
    let recorded = std::fs::read_to_string(records.join(format!("{}.tsv", second.head))).unwrap();
    assert!(
        !recorded
            .lines()
            .any(|line| line.starts_with("stress:goodfetch-engine\t")),
        "{recorded}"
    );
}

/// The backstop: a full run on main's head stresses nothing, since its base is
/// that head, so its green says nothing of a result that counted passes from an
/// earlier head. The comparison lists each one, with the base its run compared
/// with, for the backstop to stress again.
#[test]
fn a_full_run_lists_each_result_that_counted_earlier_passes() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = red_on_xtask_then_its_fix(&workspace, &records);
    let second = plan(&workspace, &records);
    record_plan(&workspace, &records, &second, &[]);
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", "Merge"]);
    let backstop = records.with_file_name("backstop-runs");
    let full = plan_with(&workspace, &backstop, "CI=", &["--full"]);
    record_plan(&workspace, &backstop, &full, &[]);

    let compared = compare(&workspace, &backstop, &records, &full.head);

    assert_eq!(compared.code, 0, "{}", compared.stderr);
    let base = workspace.git(&["rev-parse", "origin/main"]);
    assert_eq!(
        found_in(&compared),
        ["goodfetch-engine", "goodfetch-frontier"].map(|member| serde_json::json!({
            "kind": "earned", "unit": format!("stress:{member}"), "step": "stress",
            "member": member, "run": second.head, "origin": first, "base": base.trim(),
        })),
        "{}",
        compared.stdout
    );
}

fn found_in(compared: &Outcome) -> Vec<serde_json::Value> {
    serde_json::from_str(&compared.stdout).unwrap()
}

/// `head`, on `trunk`'s line, merged as `gh pr merge --merge` merges: a merge
/// commit whose second parent is the gated head.
fn merged(workspace: &Workspace, head: &str) -> String {
    workspace.git(&["merge", "--quiet", "--no-ff", "--message", "Merge", head]);
    workspace.git(&["rev-parse", "HEAD"]).trim().to_owned()
}

/// Pull request A merges with earned passes, then pull request B merges on
/// another tree before the backstop runs, carrying A's results. Those results
/// are listed from any full run after it, until a green backstop run holds A's
/// merge, however many merges or skipped days came between; a head that never
/// merged is not.
#[test]
fn a_full_run_lists_the_earned_passes_of_every_merge_since_the_last_green_run() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let before_a = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    let first = red_on_xtask_then_its_fix(&workspace, &records);
    let a = plan(&workspace, &records);
    record_plan(&workspace, &records, &a, &[]);
    // `trunk` starts where A's branch left the fixture, before xtask's commits.
    workspace.git(&["switch", "--quiet", "--create", "trunk", &before_a]);
    let merged_a = merged(&workspace, &a.head);
    let a_tree = workspace.git(&["rev-parse", "HEAD^{tree}"]);
    // Pull request B, on another tree, with nothing earned.
    workspace.git(&["switch", "--quiet", "--create", "b"]);
    workspace.write("docs/b.md", b"# B\n");
    commit_all(&workspace, "Change B");
    let b = plan(&workspace, &records);
    record_plan(&workspace, &records, &b, &[]);
    workspace.git(&["switch", "--quiet", "trunk"]);
    merged(&workspace, &b.head);
    assert_ne!(workspace.git(&["rev-parse", "HEAD^{tree}"]), a_tree);
    // A head with earned passes that never merged.
    workspace.git(&["switch", "--quiet", "--create", "never-merged"]);
    workspace.write("docs/never.md", b"# Never\n");
    commit_all(&workspace, "Change never merged");
    let never = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    std::fs::write(
        records.join(format!("{never}.tsv")),
        format!(
            "base\tb\nhost\th\nstress:goodfetch-conformance\tpass\t{never}\tt\t{NOW}\t{first}\n"
        ),
    )
    .unwrap();
    workspace.git(&["switch", "--quiet", "trunk"]);
    let backstop = records.with_file_name("backstop-runs");
    let full = plan_with(&workspace, &backstop, "CI=", &["--full"]);
    record_plan(&workspace, &backstop, &full, &[]);
    let listed = |since: Option<&str>| -> Vec<(String, String)> {
        let outcome = compare_since(&workspace, &backstop, &records, &full.head, since);
        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
        found_in(&outcome)
            .iter()
            .filter(|found| found["kind"] == "earned")
            .map(|found| {
                (
                    found["run"].as_str().unwrap().to_owned(),
                    found["member"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    // The last green run was before A merged, or the backstop never ran green.
    let of_a_and_b_carrying_them: Vec<(String, String)> = [&a.head, &b.head]
        .into_iter()
        .flat_map(|run| {
            ["goodfetch-engine", "goodfetch-frontier"]
                .map(|member| (run.clone(), member.to_owned()))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(listed(Some(&before_a)), of_a_and_b_carrying_them);
    assert_eq!(listed(None), of_a_and_b_carrying_them);
    // A green run holds A's merge: those passes are done with, B's carrying them
    // included.
    assert_eq!(listed(Some(&merged_a)), []);
}

/// A pull request pushed again over its first head: the head that merged carries
/// a result that counted earned passes from the first, which `main` never held.
/// No green run stressed those passes again, so the result is listed.
#[test]
fn a_result_carried_from_a_head_main_never_held_is_listed() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let before = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.write("docs/pushed.md", b"# First push\n");
    commit_all(&workspace, "First push");
    let first_push = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.git(&["switch", "--quiet", "--create", "second-push", &before]);
    workspace.write("docs/pushed.md", b"# Second push\n");
    commit_all(&workspace, "Second push");
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join(format!("{head}.tsv")),
        format!(
            "base\tb\nhost\th\nstress:goodfetch-conformance\tpass\t{first_push}\tt\t{NOW}\t{before}\n"
        ),
    )
    .unwrap();
    workspace.git(&["switch", "--quiet", "--create", "trunk", &before]);
    merged(&workspace, &head);
    let backstop = records.with_file_name("backstop-runs");
    let full = plan_with(&workspace, &backstop, "CI=", &["--full"]);
    record_plan(&workspace, &backstop, &full, &[]);

    let compared = compare_since(&workspace, &backstop, &records, &full.head, Some(&before));

    assert_eq!(compared.code, 0, "{}", compared.stderr);
    assert!(
        found_in(&compared)
            .iter()
            .any(|found| found["kind"] == "earned"
                && found["run"] == head.as_str()
                && found["member"] == "goodfetch-conformance"),
        "{}",
        compared.stdout
    );
}

/// How `ci/merge-pr.sh` opens the message of a merge a full gate passed, and of
/// one a quick gate passed, which `merge_pr` holds it to.
const FULL_GATE_MERGE: &str = "Merged on the local gate: `just pre-merge all`, which starts with `just quick all`'s lint and Quick suite, passed.";
const QUICK_GATE_MERGE: &str = "Merged on the local gate: `just pre-merge quick` (no test, benchmark, model, container or Evaluation reads the change) passed.";

/// A commit on a branch of its own from `from`, merged nowhere yet: its head.
fn change_from(workspace: &Workspace, from: &str, name: &str) -> String {
    workspace.git(&["switch", "--quiet", "--create", name, from]);
    workspace.write(&format!("docs/{name}.md"), b"# A change\n");
    commit_all(workspace, name);
    workspace.git(&["rev-parse", "HEAD"]).trim().to_owned()
}

/// `head` merged on `trunk` with `message`, as `just merge` words its merge.
fn merged_saying(workspace: &Workspace, head: &str, message: &str) -> String {
    workspace.git(&["switch", "--quiet", "trunk"]);
    workspace.git(&["merge", "--quiet", "--no-ff", "--message", message, head]);
    workspace.git(&["rev-parse", "HEAD"]).trim().to_owned()
}

/// How `ci/merge-pr.sh` opens the message of a merge the quick gate passed on a
/// change a test reads. No plan reads a merge's message.
const QUICK_GATE_MERGE_A_TEST_READS: &str =
    "Merged on the local gate: `just pre-merge quick` (a test reads the change) passed.";

/// A pull request from `main`'s head, on the branch `name`, checked out: it
/// adds a test of that name to `source`. Its head.
fn pull_request_adding_a_test(workspace: &Workspace, name: &str, source: &str) -> String {
    workspace.git(&["switch", "--quiet", "--create", name, "main"]);
    workspace.append(
        source,
        &format!("#[test]\nfn {}() {{}}\n", name.replace('-', "_")),
    );
    commit_all(workspace, name);
    workspace.git(&["rev-parse", "HEAD"]).trim().to_owned()
}

/// `head` merged into `main` with `message`, as `just merge` merges, and
/// `origin/main` moved to the merge.
fn merged_into_main(workspace: &Workspace, head: &str, message: &str) {
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.git(&["merge", "--quiet", "--no-ff", "--message", message, head]);
    workspace.git(&["update-ref", "refs/remotes/origin/main", "main"]);
}

/// A pull request that adds a test to the engine, its full gate recorded green
/// on its head and that head merged into `main`: the gated head.
fn full_gate_merged(workspace: &Workspace, records: &Path) -> String {
    let head = pull_request_adding_a_test(
        workspace,
        "full-gated",
        "crates/goodfetch-engine/src/lib.rs",
    );
    let gate = plan(workspace, records);
    record_plan(workspace, records, &gate, &[]);
    merged_into_main(workspace, &head, FULL_GATE_MERGE);
    head
}

/// What `main` took since its last full gate, and the next pull request to
/// take one: a full gate recorded green on a head that then merged; two pull
/// requests merged on the quick gate since, each adding a test, which leaves
/// no record; and a third pull request, which adds a test too, checked out.
/// The repository, its records' folder with what holds it, and the head the
/// full gate ran on.
pub(crate) fn quick_merges_since_a_full_gate() -> (Workspace, tempfile::TempDir, PathBuf, String) {
    let (workspace, folder, records) = gated_repository(&[]);
    let full_gate = full_gate_merged(&workspace, &records);
    for (name, source) in [
        ("first-quick-merge", "crates/goodfetch-engine/src/lease.rs"),
        (
            "second-quick-merge",
            "crates/goodfetch-conformance/src/lib.rs",
        ),
    ] {
        let head = pull_request_adding_a_test(&workspace, name, source);
        merged_into_main(&workspace, &head, QUICK_GATE_MERGE_A_TEST_READS);
    }
    pull_request_adding_a_test(
        &workspace,
        "third-pull-request",
        "crates/foxhound/goodfetch-frontier/src/lib.rs",
    );
    (workspace, folder, records, full_gate)
}

/// The pull request gated again after a fix reuses what its first run left,
/// and its stress run still starts where `main`'s last full gate ended: the
/// tests it added before its first run are new since that head, whatever that
/// run found, so a record on a head `main` does not hold moves nothing.
#[test]
fn a_run_on_the_pull_requests_own_earlier_head_is_not_where_its_stress_run_starts() {
    let (workspace, _folder, records, full_gate) = quick_merges_since_a_full_gate();
    let first = plan(&workspace, &records);
    record_plan(&workspace, &records, &first, &["stress"]);
    workspace.write("docs/fix.md", b"# A fix\n");
    commit_all(&workspace, "Fix what the first gate found");

    let second = plan(&workspace, &records);

    assert_eq!(second.previous, first.head);
    assert_eq!(first.stress_base, full_gate);
    assert_eq!(second.stress_base, full_gate);
}

/// A full gate covers the tests up to its head only when its stress run
/// passed. A pull request whose full gate left its stress run red, or was
/// stopped before that run ended, and which then merged on the quick gate
/// after a fix, leaves a record on a head `main` holds: the next full gate's
/// stress run starts at the full gate before it, whose stress run passed, and
/// so covers that pull request's tests too.
#[rstest::rstest]
#[case::red(Some("fail"))]
#[case::never_ended(None)]
fn a_full_gate_whose_stress_run_did_not_pass_is_not_where_the_next_stress_run_starts(
    #[case] stress: Option<&str>,
) {
    let (workspace, _folder, records) = gated_repository(&[]);
    let passed = full_gate_merged(&workspace, &records);
    pull_request_adding_a_test(
        &workspace,
        "gated-red",
        "crates/goodfetch-engine/src/lease.rs",
    );
    let red = plan(&workspace, &records);
    for step in red.steps.keys().filter(|step| *step != "stress") {
        record(&workspace, &records, &red.head, step, "pass");
    }
    if let Some(status) = stress {
        record(&workspace, &records, &red.head, "stress", status);
    }
    workspace.write("docs/fix.md", b"# A fix\n");
    commit_all(&workspace, "Fix what the full gate found");
    let fixed = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    merged_into_main(&workspace, &fixed, QUICK_GATE_MERGE_A_TEST_READS);
    pull_request_adding_a_test(
        &workspace,
        "next-full-gate",
        "crates/goodfetch-conformance/src/lib.rs",
    );

    let next = plan(&workspace, &records);

    assert_eq!(next.stress_base, passed);
}

/// A change no test reads takes no stress run at its own merge (`xtask
/// affected`'s `gate=docs` or `gate=quick`), so one merged since the last full
/// gate leaves the next stress run nothing to cover: it compares its head with
/// the change's base, as before, whose saved listings it can then read.
#[test]
fn with_nothing_a_test_reads_merged_since_the_last_full_gate_the_stress_run_keeps_its_base() {
    let (workspace, _folder, records) = gated_repository(&[]);
    full_gate_merged(&workspace, &records);
    let documents = change_from(&workspace, "main", "documents");
    merged_into_main(&workspace, &documents, QUICK_GATE_MERGE);
    pull_request_adding_a_test(
        &workspace,
        "next-full-gate",
        "crates/goodfetch-conformance/src/lib.rs",
    );

    let next = plan(&workspace, &records);

    assert_eq!(next.stress_base, "-");
}

/// With no record of a full gate on a head `main` holds, as before the first
/// one or once the gate's records are gone, the stress run compares its head
/// with the change's base, as it did before #2455.
#[test]
fn with_no_full_gate_recorded_the_stress_run_keeps_its_base() {
    let (workspace, _folder, records, _) = quick_merges_since_a_full_gate();
    std::fs::remove_dir_all(&records).unwrap();

    let next = plan(&workspace, &records);

    assert_eq!(next.previous, "none");
    assert_eq!(next.stress_base, "-");
}

/// What the plan's log says when no record shows where the last full gate's
/// stress run ended.
const NO_FULL_GATE_RECORDED: &str =
    "no record of an earlier full gate whose stress run passed is on the base's history";

/// That stress run leaves out the tests merged on the quick gate since the
/// last full gate, while `just merge` starts its count of quick merges again
/// at this gate: the plan's log says so, since `stress.base=-` alone does not
/// tell it from a base nothing a test reads merged on.
#[test]
fn with_no_full_gate_recorded_the_plans_log_says_the_stress_run_leaves_the_quick_merges_tests_out()
{
    let (workspace, _folder, records, _) = quick_merges_since_a_full_gate();
    std::fs::remove_dir_all(&records).unwrap();

    let next = planned(&workspace, &records, "CI=", &[]);

    assert!(
        next.stderr.contains(NO_FULL_GATE_RECORDED),
        "{}",
        next.stderr
    );
}

/// With the last full gate recorded and nothing a test reads merged since, the
/// stress run keeps the change's base and leaves no test out: the plan's log
/// does not say it does.
#[test]
fn with_a_full_gate_recorded_the_plans_log_does_not_say_the_stress_run_leaves_tests_out() {
    let (workspace, _folder, records) = gated_repository(&[]);
    full_gate_merged(&workspace, &records);
    let documents = change_from(&workspace, "main", "documents");
    merged_into_main(&workspace, &documents, QUICK_GATE_MERGE);
    pull_request_adding_a_test(
        &workspace,
        "next-full-gate",
        "crates/goodfetch-conformance/src/lib.rs",
    );

    let next = planned(&workspace, &records, "CI=", &[]);

    assert_eq!(parse(&next).stress_base, "-");
    assert!(
        !next.stderr.contains(NO_FULL_GATE_RECORDED),
        "{}",
        next.stderr
    );
}

/// A run on `main` itself, as the backstop's full run is, gates no change: its
/// head is its own base, each merge it holds was judged by its own gate, and
/// its stress run selects nothing (ADR 0135's amendment of 2026-11-09). The
/// last full gate moves nothing there.
#[test]
fn a_run_on_main_itself_keeps_the_stress_runs_base() {
    let (workspace, _folder, records, _) = quick_merges_since_a_full_gate();
    workspace.git(&["switch", "--quiet", "--detach", "main"]);

    let on_main = plan_with(&workspace, &records, "CI=", &["--full"]);

    assert_eq!(on_main.stress_base, "-");
}

/// The runs the comparison says have no record, `""` for the whole folder.
fn missing_in(compared: &Outcome) -> Vec<String> {
    assert_eq!(compared.code, 0, "{}", compared.stderr);
    found_in(compared)
        .iter()
        .filter(|found| found["kind"] == "missing")
        .map(|found| found["run"].as_str().unwrap().to_owned())
        .collect()
}

/// A full gate leaves a record that may hold earned passes, so a merge it passed
/// since the last green run must have one: without it, the day could not say what
/// the merge earned. A quick or docs gate leaves none, and a merge the last green
/// run holds is done with. With no green run there is no range to hold them to:
/// history holds merges whose records went long ago.
#[test]
fn a_full_gates_merge_since_the_last_green_run_needs_its_record() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let before = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    let earlier = change_from(&workspace, &before, "earlier");
    let kept = change_from(&workspace, &before, "kept");
    let lost = change_from(&workspace, &before, "lost");
    let quick = change_from(&workspace, &before, "quick");
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(records.join(format!("{kept}.tsv")), "base\tb\nhost\th\n").unwrap();
    workspace.git(&["switch", "--quiet", "--create", "trunk", &before]);
    let last_green = merged_saying(&workspace, &earlier, FULL_GATE_MERGE);
    merged_saying(&workspace, &kept, FULL_GATE_MERGE);
    merged_saying(&workspace, &lost, FULL_GATE_MERGE);
    merged_saying(&workspace, &quick, QUICK_GATE_MERGE);
    let backstop = records.with_file_name("backstop-runs");
    let full = plan_with(&workspace, &backstop, "CI=", &["--full"]);
    record_plan(&workspace, &backstop, &full, &[]);

    let since = compare_since(
        &workspace,
        &backstop,
        &records,
        &full.head,
        Some(&last_green),
    );
    let first_run = compare_since(&workspace, &backstop, &records, &full.head, None);

    assert_eq!(missing_in(&since), [lost]);
    assert_eq!(missing_in(&first_run), Vec::<String>::new());
}

/// With `just merge`'s records gone, nothing can say what the merges since the
/// last green run earned, and no green run says so for any merge at all. With
/// nothing merged since the last green run, nothing rests on them.
#[rstest::rstest]
#[case::merges_since_the_last_green_run("before", true)]
#[case::no_green_run_yet("none", true)]
#[case::nothing_merged_since("head", false)]
fn a_records_folder_that_is_gone_is_missing(#[case] since: &str, #[case] is_missing: bool) {
    let (workspace, _folder, records) = gated_repository(&[]);
    let before = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    let change = change_from(&workspace, &before, "a-change");
    workspace.git(&["switch", "--quiet", "--create", "trunk", &before]);
    let head = merged_saying(&workspace, &change, QUICK_GATE_MERGE);
    let backstop = records.with_file_name("backstop-runs");
    let full = plan_with(&workspace, &backstop, "CI=", &["--full"]);
    record_plan(&workspace, &backstop, &full, &[]);
    let gone = records.with_file_name("no-such-folder");
    let since = match since {
        "before" => Some(before.as_str()),
        "head" => Some(head.as_str()),
        _ => None,
    };

    let compared = compare_since(&workspace, &backstop, &gone, &full.head, since);

    let expected: &[&str] = if is_missing { &[""] } else { &[] };
    assert_eq!(missing_in(&compared), expected);
}

/// A merged head whose record cannot be read may have counted earned passes, so
/// the comparison says so instead of dropping it, and the backstop counts it as
/// a miss. An unmerged head's is only logged, as before.
#[test]
fn a_merged_head_whose_record_cannot_be_read_is_listed() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let before = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.write("docs/a.md", b"# A\n");
    commit_all(&workspace, "Change A");
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.git(&["switch", "--quiet", "--create", "trunk", &before]);
    merged(&workspace, &head);
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(records.join(format!("{head}.tsv")), "not a record\n").unwrap();
    let backstop = records.with_file_name("backstop-runs");
    let full = plan_with(&workspace, &backstop, "CI=", &["--full"]);
    record_plan(&workspace, &backstop, &full, &[]);

    let compared = compare_since(&workspace, &backstop, &records, &full.head, Some(&before));

    assert_eq!(compared.code, 0, "{}", compared.stderr);
    let found = found_in(&compared);
    assert!(
        found
            .iter()
            .any(|found| found["kind"] == "unreadable" && found["run"] == head.as_str()),
        "{}",
        compared.stdout
    );
}

/// Records every step its plan runs as `ci/pre-merge.sh` does, green but
/// for `red`.
fn record_plan(workspace: &Workspace, records: &Path, plan: &Plan, red: &[&str]) {
    for step in plan.steps.keys().map(String::as_str) {
        if action(plan, step).0 == "reuse" {
            continue;
        }
        let status = if red.contains(&step) { "fail" } else { "pass" };
        record(workspace, records, &plan.head, step, status);
    }
}

/// A full green gate run on HEAD, then a change that reaches the Python
/// client, and the incremental run on it, green: their heads.
fn green_runs(workspace: &Workspace, records: &Path) -> (String, String) {
    let first = plan(workspace, records);
    record_plan(workspace, records, &first, &[]);
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"\"\"\"Changed.\"\"\"\n",
    );
    commit_all(workspace, "Change the Python client");
    let second = plan(workspace, records);
    assert_eq!(action(&second, "perf").0, "reuse", "{}", second.why["perf"]);
    record_plan(workspace, records, &second, &[]);
    (first.head, second.head)
}

/// Compares the full run on `full`, recorded in `records`, with the runs in
/// `runs`.
fn compare(workspace: &Workspace, records: &Path, runs: &Path, full: &str) -> Outcome {
    compare_since(workspace, records, runs, full, None)
}

/// As [`compare`], since the backstop's last green run on `since`.
fn compare_since(
    workspace: &Workspace,
    records: &Path,
    runs: &Path,
    full: &str,
    since: Option<&str>,
) -> Outcome {
    let records = records.to_string_lossy();
    let runs = runs.to_string_lossy();
    let mut command = vec![
        "gate",
        "compare",
        "--records",
        &records,
        "--runs",
        &runs,
        "--full",
        full,
    ];
    if let Some(since) = since {
        command.extend(["--since", since]);
    }
    workspace.xtask(&command)
}

/// The backstop (ADR 0135; the test-gate audit's option 7): a full run, in
/// its own worktree's records, on a commit whose tree an incremental run of
/// `just merge` gated finds each result that run reused and the full run
/// found red, a selection miss; a red step run per member is only a
/// suspect, since the members the incremental run reran may hold the red.
#[test]
fn a_full_run_finds_each_reused_result_it_finds_red() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let (first, second) = green_runs(&workspace, &records);
    // The merge of the gated head: a commit with its tree.
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", "Merge"]);
    let backstop = records.with_file_name("backstop-runs");
    let full = plan_with(&workspace, &backstop, "CI=", &["--full"]);
    record_plan(&workspace, &backstop, &full, &["perf", "pre-merge"]);

    let compared = compare(&workspace, &backstop, &records, &full.head);

    assert_eq!(compared.code, 0, "{}", compared.stderr);
    let found: Vec<serde_json::Value> = serde_json::from_str(&compared.stdout).unwrap();
    let base = workspace.git(&["rev-parse", "origin/main"]);
    let base = base.trim();
    assert!(
        found.contains(&serde_json::json!({
            "kind": "miss", "unit": "perf", "step": "perf", "member": null,
            "run": second, "origin": first, "base": base,
        })),
        "{}",
        compared.stdout
    );
    assert!(
        found.contains(&serde_json::json!({
            "kind": "suspect", "unit": "pre-merge:goodfetch-engine", "step": "pre-merge",
            "member": "goodfetch-engine", "run": second, "origin": first, "base": base,
        })),
        "{}",
        compared.stdout
    );
    assert!(
        !found.iter().any(|result| result["step"] == "tlc"),
        "{}",
        compared.stdout
    );
}

/// A run that reused something is not a full run to compare with.
#[test]
fn only_a_full_run_is_compared() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let (_, second) = green_runs(&workspace, &records);

    let compared = compare(&workspace, &records, &records, &second);

    assert_eq!(compared.code, 1, "{}", compared.stdout);
    assert!(
        compared.stderr.contains("is not a full run"),
        "{}",
        compared.stderr
    );
}
