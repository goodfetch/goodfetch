//! The incremental local gate (`xtask gate`, ADR 0135): a gate run records
//! each step's result by head, and a later run on a descendant head reruns
//! the steps and members `xtask affected` finds the change since then
//! reaches, every check that judges the whole change, and every red or
//! missing result; it reuses the rest. The local gate is repository
//! tooling, so the module declares no Feature.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::affected::{commit_all, repository, value};
use crate::workspace::{Outcome, Workspace};

/// Each step's `(action, members)` from the plan's lines, and the
/// `previous` line's head.
struct Plan {
    previous: String,
    steps: BTreeMap<String, (String, String)>,
    why: BTreeMap<String, String>,
}

fn plan(workspace: &Workspace, records: &Path) -> Plan {
    let outcome = workspace.xtask(&["gate", "plan", "--records", &records.to_string_lossy()]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    parse(&outcome)
}

fn parse(outcome: &Outcome) -> Plan {
    let mut plan = Plan {
        previous: String::new(),
        steps: BTreeMap::new(),
        why: BTreeMap::new(),
    };
    for line in outcome.stdout.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["previous", head] => (*head).clone_into(&mut plan.previous),
            ["members", _] => {}
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

fn record(workspace: &Workspace, records: &Path, step: &str, status: &str) {
    let outcome = workspace.xtask(&[
        "gate",
        "record",
        "--records",
        &records.to_string_lossy(),
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

/// The records' folder, outside the repository so the tree stays clean.
fn records() -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let records = folder.path().join("gate-runs");
    (folder, records)
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
        record(workspace, records, step, status);
    }
    workspace.git(&["rev-parse", "HEAD"]).trim().to_owned()
}

#[test]
fn a_later_run_reruns_what_the_change_since_the_first_reaches_and_every_red_result() {
    let workspace = repository();
    let (_folder, records) = records();
    let first = first_run(&workspace, &records);
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"\"\"\"Changed.\"\"\"\n",
    );
    commit_all(&workspace, "Change the Python client");

    let second = plan(&workspace, &records);

    assert_eq!(second.previous, first);
    // What `xtask affected` selects for the change since the first run.
    let affected = workspace.xtask(&["affected", "--base", &first, "--head", "HEAD"]);
    let reached = [
        value(&affected, "crates"),
        value(&affected, "vitest-packages"),
        value(&affected, "pytest-packages"),
    ]
    .iter()
    .filter(|members| !members.is_empty())
    .cloned()
    .collect::<Vec<_>>()
    .join(" ");
    assert_eq!(reached, "sdks/python/collie-client sdks/python/foxhound");
    assert_eq!(action(&second, "pre-merge"), ("part", reached.as_str()));
    // Green, and reached by nothing that changed.
    assert_eq!(action(&second, "perf").0, "reuse");
    assert_eq!(action(&second, "tlc").0, "reuse");
    assert_eq!(action(&second, "visual").0, "reuse");
    // Red on the first head: never reused.
    assert_eq!(action(&second, "quick"), ("run", "-"));
    assert!(
        second.why["quick"].starts_with("red on "),
        "{}",
        second.why["quick"]
    );
    // Never recorded on the first head.
    assert_eq!(action(&second, "msrv"), ("run", "-"));
    // The checks that judge the whole change always run.
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

#[test]
fn a_change_to_the_justfile_reruns_every_step_in_full() {
    let workspace = repository();
    let (_folder, records) = records();
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
    let workspace = repository();
    let (_folder, records) = records();
    let first = first_run(&workspace, &records);
    let record_file = records.join(format!("{first}.tsv"));
    let recorded = std::fs::read_to_string(&record_file).unwrap();
    workspace.write("crates/goodfetch-engine/src/lib.rs", b"//! Changed.\n");

    let dirty = plan(&workspace, &records);
    record(&workspace, &records, "perf", "fail");

    assert_eq!(dirty.previous, "none");
    assert!(dirty.steps.is_empty());
    assert_eq!(std::fs::read_to_string(&record_file).unwrap(), recorded);
}
