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
use crate::workspace::{Outcome, Workspace};

/// Now, in seconds since the Unix epoch, for every run.
const NOW: &str = "1800000000";

/// Each step's `(action, members)` from the plan's lines, and the
/// `previous` line's head.
struct Plan {
    previous: String,
    head: String,
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
    let target = target(records);
    let records = records.to_string_lossy();
    let target = target.to_string_lossy();
    let mut args = vec![
        "gate",
        "plan",
        "--records",
        &records,
        "--now",
        NOW,
        "--target-dir",
        &target,
        "--environment",
        environment,
    ];
    args.extend(extra);
    let outcome = workspace.xtask(&args);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    parse(&outcome)
}

fn parse(outcome: &Outcome) -> Plan {
    let mut plan = Plan {
        previous: String::new(),
        head: String::new(),
        steps: BTreeMap::new(),
        why: BTreeMap::new(),
    };
    for line in outcome.stdout.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["previous", head] => (*head).clone_into(&mut plan.previous),
            ["head", head] => (*head).clone_into(&mut plan.head),
            ["members" | "unowned", _] => {}
            [step, action, members, why] => {
                plan.steps.insert(
                    (*step).to_owned(),
                    ((*action).to_owned(), (*members).to_owned()),
                );
                plan.why.insert((*step).to_owned(), (*why).to_owned());
            }
            _ => panic!("an unexpected plan line: {line:?}"),
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

#[test]
fn a_change_to_the_justfile_reruns_every_step_in_full() {
    let (workspace, _folder, records) = gated_repository(&[]);
    first_run(&workspace, &records);
    workspace.write("justfile", b"default:\n    @true\n");
    commit_all(&workspace, "Change the justfile");

    let second = plan(&workspace, &records);

    assert!(
        second.steps.values().all(|(action, _)| action == "run"),
        "{:?}",
        second.steps
    );
    assert!(second.why["perf"].starts_with("justfile changed since "));
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
    assert!(dirty.steps.is_empty());
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
    let first = plan(workspace, records);
    for step in ["perf", "tlc", "visual", "pre-merge"] {
        record(workspace, records, &first.head, step, "pass");
    }
    let file = records.with_file_name("quick-junit.xml");
    std::fs::write(&file, junit).unwrap();
    record_with(
        workspace,
        records,
        &first.head,
        "quick",
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

/// Every step of `just pre-merge all`, by its name in the plan.
const STEPS: [&str; 20] = [
    "dco",
    "commit-messages",
    "lint",
    "feature-combinations",
    "audit",
    "commit-leaks",
    "fuzz-check",
    "proofs",
    "journeys-tasks",
    "features-tests",
    "notices",
    "perf",
    "linux",
    "visual",
    "quick",
    "pre-merge",
    "stress",
    "tlc",
    "msrv",
    "miri",
];

/// Records every step its plan runs as `ci/pre-merge.sh` does, green but
/// for `red`.
fn record_plan(workspace: &Workspace, records: &Path, plan: &Plan, red: &[&str]) {
    for step in STEPS {
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
    workspace.xtask(&[
        "gate",
        "compare",
        "--records",
        &records.to_string_lossy(),
        "--runs",
        &runs.to_string_lossy(),
        "--full",
        full,
    ])
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
    let lines: BTreeSet<&str> = compared.stdout.lines().collect();
    assert!(
        lines.contains(format!("miss\tperf\t{second}\t{first}").as_str()),
        "{}",
        compared.stdout
    );
    assert!(
        lines.contains(format!("suspect\tpre-merge:goodfetch-engine\t{second}\t{first}").as_str()),
        "{}",
        compared.stdout
    );
    assert!(
        !lines.iter().any(|line| line.contains("\ttlc\t")),
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
