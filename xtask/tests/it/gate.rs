//! The incremental local gate (`xtask gate`, ADR 0135): a gate run records
//! each step's result by head, and a later run on a descendant head reruns
//! the steps and members `xtask affected` finds the change since then
//! reaches, those the evidence of what each member read reaches, every check
//! that judges the whole change, and every red or missing result; it reuses
//! the rest. The local gate is repository tooling, so the module declares no
//! Feature.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
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
    let outcome = plan_outcome(workspace, records, environment, extra);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    parse(&outcome)
}

fn plan_outcome(
    workspace: &Workspace,
    records: &Path,
    environment: &str,
    extra: &[&str],
) -> Outcome {
    let target = target(records);
    let key = key(records);
    let mut args = vec![
        "gate",
        "plan",
        "--records",
        records.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
        "--now",
        NOW,
        "--target-dir",
        target.to_str().unwrap(),
        "--environment",
        environment,
    ];
    args.extend(extra);
    workspace.xtask(&args)
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
        "--key",
        &key(records).to_string_lossy(),
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
        "--key",
        &key(records).to_string_lossy(),
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

/// The gate's key beside the records, as the host keeps it outside the
/// gate's worktree.
fn key(records: &Path) -> PathBuf {
    records.with_file_name("gate.key")
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

/// A forced full run (`GOODFETCH_GATE_FULL=1`) changes what the run reuses,
/// not how its steps run, so its results count for the next run in the
/// same environment without the variable.
#[test]
fn the_run_after_a_forced_full_run_reuses_its_results() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let full = plan_with(
        &workspace,
        &records,
        "CI=\nGOODFETCH_GATE_FULL=1",
        &["--full"],
    );
    assert!(full.steps.values().all(|(action, _)| action == "run"));
    record(&workspace, &records, &full.head, "perf", "pass");
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"\"\"\"Changed.\"\"\"\n",
    );
    commit_all(&workspace, "Change the Python client");

    let next = plan_in(&workspace, &records, "CI=");

    assert_eq!(next.previous, full.head);
    assert_eq!(action(&next, "perf").0, "reuse", "{}", next.why["perf"]);
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
/// for `red`, and leaves out `missing`.
fn record_plan(workspace: &Workspace, records: &Path, plan: &Plan, red: &[&str], missing: &[&str]) {
    for step in STEPS {
        if action(plan, step).0 == "reuse" || missing.contains(&step) {
            continue;
        }
        let status = if red.contains(&step) { "fail" } else { "pass" };
        record(workspace, records, &plan.head, step, status);
    }
}

/// A full green gate run on HEAD, then a change that reaches the Python
/// client, and the incremental run on it, green: its head.
fn green_runs(workspace: &Workspace, records: &Path) -> (String, String) {
    let first = plan(workspace, records);
    record_plan(workspace, records, &first, &[], &[]);
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"\"\"\"Changed.\"\"\"\n",
    );
    commit_all(workspace, "Change the Python client");
    let second = plan(workspace, records);
    assert_eq!(action(&second, "perf").0, "reuse");
    record_plan(workspace, records, &second, &[], &[]);
    (first.head, second.head)
}

fn verify(workspace: &Workspace, records: &Path, head: &str) -> Outcome {
    workspace.xtask(&[
        "gate",
        "verify",
        "--records",
        &records.to_string_lossy(),
        "--key",
        &key(records).to_string_lossy(),
        "--head",
        head,
    ])
}

/// The merge's check of a verdict (`ci/merge-pr.sh`): every unit of every
/// step is green, and every result reused from an earlier run is that run's
/// own signed green result on an ancestor, whose run it names.
#[test]
fn a_green_verdict_verifies_and_names_every_run_it_reuses() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let (first, second) = green_runs(&workspace, &records);

    let verified = verify(&workspace, &records, &second);

    assert_eq!(verified.code, 0, "{}", verified.stderr);
    assert_eq!(verified.stdout, format!("origin\t{first}\n"));
}

#[rstest::rstest]
#[case::a_red_step(&["quick"], &[], "quick is red")]
#[case::a_step_with_no_result(&[], &["msrv"], "msrv has no result")]
fn a_verdict_with_a_red_or_missing_result_is_refused(
    #[case] red: &[&str],
    #[case] missing: &[&str],
    #[case] why: &str,
) {
    let (workspace, _folder, records) = gated_repository(&[]);
    let head = plan(&workspace, &records);
    record_plan(&workspace, &records, &head, red, missing);

    let verified = verify(&workspace, &records, &head.head);

    assert_eq!(verified.code, 1, "{}", verified.stdout);
    assert!(verified.stderr.contains(why), "{}", verified.stderr);
}

/// A reused result counts only while the run it came from still holds it,
/// signed: a removed or replaced record of that run refuses the verdict.
#[test]
fn a_verdict_whose_reused_run_has_no_record_is_refused() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let (first, second) = green_runs(&workspace, &records);
    std::fs::write(
        records.join(format!("{first}.tsv")),
        "base\tb\nhost\th\nperf\tpass\tx\tt\t1\n",
    )
    .unwrap();

    let verified = verify(&workspace, &records, &second);

    assert_eq!(verified.code, 1, "{}", verified.stdout);
    assert!(
        verified
            .stderr
            .contains(&format!("perf reuses the run on {first}")),
        "{}",
        verified.stderr
    );
}

/// Signs `text`'s lines with the host's gate key for the run on `head`, as
/// the gate's records do, to forge a record no gate run wrote.
fn forge(records: &Path, head: &str, text: &str) -> String {
    use hmac::Mac as _;
    let key = std::fs::read_to_string(key(records)).unwrap();
    text.lines().fold(String::new(), |mut signed, line| {
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(key.trim().as_bytes()).unwrap();
        mac.update(format!("{head}\n{line}").as_bytes());
        let _ = writeln!(signed, "{line}\t{:x}", mac.finalize().into_bytes());
        signed
    })
}

/// A result reused from a commit that is not an ancestor of the head, or
/// whose tree is not that commit's, does not hold for the head.
#[test]
fn a_verdict_reusing_a_result_from_outside_the_heads_history_is_refused() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let (_, second) = green_runs(&workspace, &records);
    workspace.git(&["switch", "--quiet", "--create", "aside", "main"]);
    workspace.write("docs/index.md", b"# Aside\n");
    commit_all(&workspace, "Change the docs aside");
    let aside = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    let tree = workspace
        .git(&["rev-parse", "HEAD^{tree}"])
        .trim()
        .to_owned();
    workspace.git(&["switch", "--quiet", "--detach", &second]);
    let record_file = records.join(format!("{second}.tsv"));
    let mut record = std::fs::read_to_string(&record_file).unwrap();
    record.push_str(&forge(
        &records,
        &second,
        &format!("perf\tpass\t{aside}\t{tree}\t{NOW}"),
    ));
    std::fs::write(&record_file, record).unwrap();
    std::fs::write(
        records.join(format!("{aside}.tsv")),
        forge(
            &records,
            &aside,
            &format!("base\tb\nhost\th\nperf\tpass\t{aside}\t{tree}\t{NOW}"),
        ),
    )
    .unwrap();

    let verified = verify(&workspace, &records, &second);

    assert_eq!(verified.code, 1, "{}", verified.stdout);
    assert!(
        verified
            .stderr
            .contains(&format!("{aside} is not an ancestor of {second}")),
        "{}",
        verified.stderr
    );
}

/// A record that the gate's key did not sign, such as one a test or a tool
/// wrote, is no earlier run: nothing of it is reused.
#[test]
fn a_record_the_gate_did_not_sign_is_not_reused() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let first = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    let tree = workspace
        .git(&["rev-parse", "HEAD^{tree}"])
        .trim()
        .to_owned();
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join(format!("{first}.tsv")),
        format!("base\tb\nhost\th\nperf\tpass\t{first}\t{tree}\t{NOW}\n"),
    )
    .unwrap();
    workspace.write("docs/index.md", b"# Changed\n");
    commit_all(&workspace, "Change the docs");

    let outcome = plan_outcome(&workspace, &records, "CI=", &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let second = parse(&outcome);
    assert_eq!(second.previous, "none");
    assert!(
        outcome
            .stderr
            .contains("not signed by this host's gate key"),
        "{}",
        outcome.stderr
    );
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
        "--key",
        &key(records).to_string_lossy(),
        "--full",
        full,
    ])
}

/// The backstop (ADR 0135): a full run, in its own worktree's records, on a
/// commit whose tree an incremental run of `just merge` gated finds each
/// result that run reused and the full run found red, a selection miss; a
/// red step run per member is only a suspect, since the members the
/// incremental run reran may hold the red.
#[test]
fn a_full_run_finds_each_reused_result_it_finds_red() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let (first, second) = green_runs(&workspace, &records);
    // The merge of the gated head: a commit with its tree.
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", "Merge"]);
    let backstop = records.with_file_name("backstop-runs");
    let full = plan_with(&workspace, &backstop, "CI=", &["--full"]);
    record_plan(&workspace, &backstop, &full, &["perf", "pre-merge"], &[]);

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

/// A run that reused nothing is not a full run to compare with.
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

/// Runs Cargo's tests in the fixture, with `args`, into the target folder
/// beside `records`: a real build, whose dep-info the plan reads.
fn cargo_test(workspace: &Workspace, records: &Path, args: &[&str]) -> std::process::Output {
    workspace
        .command("cargo")
        .args(["test", "--offline", "--quiet"])
        .args(args)
        .env("CARGO_TARGET_DIR", target(records))
        .output()
        .unwrap()
}

/// Fault injection (ADR 0135's backstop's proof): from a green record, a
/// break planted where `xtask affected` alone does not look, a file of one
/// crate that another crate's test reads through `include_str!`. The real
/// build's dep-info reaches the reading crate, the plan reruns it, and that
/// rerun fails.
#[test]
fn a_planted_break_through_a_cross_crate_include_reruns_and_fails() {
    let workspace = repository();
    let (_folder, records) = records();
    workspace.write("crates/goodfetch-engine/limits.txt", b"3\n");
    workspace.write(
        "crates/goodfetch-conformance/src/lib.rs",
        b"//! The harness.\n\n/// The engine's limit, which the harness checks.\npub const LIMIT: &str = include_str!(\"../../goodfetch-engine/limits.txt\");\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn the_engine_s_limit_is_three() {\n        assert_eq!(super::LIMIT.trim(), \"3\");\n    }\n}\n",
    );
    let built = cargo_test(&workspace, &records, &["--workspace"]);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    commit_all(&workspace, "Check the engine's limit");
    let first = plan(&workspace, &records);
    record_plan(&workspace, &records, &first, &[], &[]);
    workspace.write("crates/goodfetch-engine/limits.txt", b"4\n");
    commit_all(&workspace, "Raise the engine's limit");

    let second = plan(&workspace, &records);

    assert!(
        !affected_members(&workspace, &first.head).contains("goodfetch-conformance"),
        "the selection alone reaches the reading crate, so this plants nothing"
    );
    assert_eq!(action(&second, "pre-merge").0, "part");
    assert!(
        members(&second, "pre-merge").contains("goodfetch-conformance"),
        "{:?}",
        second.steps["pre-merge"]
    );
    assert!(
        second.why["pre-merge"].contains("its build read crates/goodfetch-engine/limits.txt"),
        "{}",
        second.why["pre-merge"]
    );
    let rerun = cargo_test(&workspace, &records, &["-p", "goodfetch-conformance"]);
    assert!(!rerun.status.success(), "the planted break passed");
}

/// Fault injection: a package that only the root Vitest config loads, so
/// every Vitest run loads it, breaks another package's tests; the plan
/// reruns that package (`xtask affected`'s runner-config edge, and the
/// head's test order, F8 option B).
#[test]
fn a_planted_break_in_a_package_the_root_vitest_config_loads_reruns_every_package() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write(
        "vitest.config.ts",
        b"import { setup } from '@goodfetch/foxhound';\n\nexport default { test: { setupFiles: [setup] } };\n",
    );
    commit_all(&workspace, "Load the Foxhound SDK in every Vitest run");
    let first = plan(&workspace, &records);
    record_plan(&workspace, &records, &first, &[], &[]);
    workspace.write(
        "sdks/typescript/foxhound/src/setup.ts",
        b"throw new Error('planted');\n",
    );
    commit_all(&workspace, "Break the setup every Vitest run loads");

    let second = plan(&workspace, &records);

    let rerun = members(&second, "pre-merge");
    for package in ["sdks/typescript/foxhound", "sdks/typescript/collie-client"] {
        assert!(rerun.contains(package), "{package} not in {rerun:?}");
    }
    assert!(
        affected_members(&workspace, &first.head).contains("sdks/typescript/collie-client"),
        "the root config's edge does not reach the package"
    );
}
