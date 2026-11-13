//! `drift.reference` (ADR 0140): `xtask perf` also holds the head to a pinned
//! reference build, #1577's tuned build and then each release tag, so that
//! many changes under ADR 0071's +5 % cannot add up unseen. A figure over 1.10
//! times its reference fails naming it, whatever `GATING` says and whatever
//! the pull request accepts. With none pinned, the check says it has not run.
//! A stand-in Cargo, a shell script, plays `cargo bench` in the last test, so
//! this module runs on Unix only.
//! Covers: story:1-108

use std::os::unix::fs::PermissionsExt as _;

use rstest::rstest;
use serde_json::Value;

use crate::perf::{bench_summary, metric};
use crate::workspace::{Finding, Outcome, Workspace, finding};

const BENCHMARK: &str = "goodfetch-frontier/instructions::frontier::push";
const PIN_FILE: &str = "ci/perf-reference.toml";

/// The gungraun line of the Frontier's benchmark, measuring `value` of `key`:
/// gungraun's `new`, as a run with no baseline writes it.
fn line(key: &str, value: u64) -> String {
    bench_summary(
        "crates/foxhound/goodfetch-frontier",
        "instructions::frontier::push",
        None,
        &[metric(key, None, Some(value))],
    )
}

fn write(workspace: &Workspace, file: &str, line: &str) {
    workspace.write(file, format!("{line}\n").as_bytes());
}

fn compare_with_reference(workspace: &Workspace) -> Outcome {
    workspace.xtask(&[
        "perf",
        "compare",
        "--gungraun",
        "perf/head.jsonl",
        "--reference-gungraun",
        "perf/reference.jsonl",
    ])
}

#[test]
fn with_no_reference_pinned_the_check_reports_not_run_yet() {
    let workspace = Workspace::from_fixture("empty");
    write(&workspace, "perf/head.jsonl", &line("Ir", 2_000));

    let outcome = workspace.xtask(&["perf", "compare", "--gungraun", "perf/head.jsonl"]);

    assert_eq!((outcome.findings(), outcome.code), (vec![], 0));
    assert!(
        outcome.stdout.contains("`drift.reference`: not run yet"),
        "{}",
        outcome.stdout
    );
}

#[rstest]
#[case::instructions_at_1_10("Ir", "instructions", 1_100, false)]
#[case::instructions_over_1_10("Ir", "instructions", 1_101, true)]
#[case::allocations_over_1_10("TotalBlocks", "total-allocations", 1_101, true)]
#[case::bytes_allocated_over_1_10("TotalBytes", "total-bytes", 1_101, true)]
#[case::heap_maximum_over_1_10("AtTGmaxBytes", "global-maximum-bytes", 1_101, true)]
fn a_figure_over_1_10_times_its_reference_fails_naming_it(
    #[case] key: &str,
    #[case] figure: &str,
    #[case] head: u64,
    #[case] fails: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    write(&workspace, "perf/head.jsonl", &line(key, head));
    write(&workspace, "perf/reference.jsonl", &line(key, 1_000));

    let outcome = compare_with_reference(&workspace);

    let expected = if fails {
        let id = format!("{BENCHMARK} {figure}");
        (vec![finding("drift-reference", &id)], 1)
    } else {
        (vec![], 0)
    };
    assert_eq!(
        (outcome.findings(), outcome.code),
        expected,
        "{}",
        outcome.stdout
    );
}

#[test]
fn the_failure_says_the_figure_its_multiple_and_the_limit() {
    let workspace = Workspace::from_fixture("empty");
    write(&workspace, "perf/head.jsonl", &line("Ir", 1_250));
    write(&workspace, "perf/reference.jsonl", &line("Ir", 1_000));

    let outcome = compare_with_reference(&workspace);

    let error = outcome
        .stdout
        .lines()
        .find(|line| line.starts_with("error[drift-reference]"))
        .unwrap_or_default();
    assert!(
        error.contains("1250 is 1.25 times its reference's 1000, over 1.10"),
        "{error}"
    );
}

#[test]
fn a_size_limit_entry_over_1_10_times_its_reference_fails() {
    let workspace = Workspace::from_fixture("empty");
    let entry = |size: u64| format!(r#"[{{"name":"collie-sdk","size":{size}}}]"#);
    workspace.write("perf/size-head.json", entry(111_000).as_bytes());
    workspace.write("perf/size-reference.json", entry(100_000).as_bytes());

    let outcome = workspace.xtask(&[
        "perf",
        "compare",
        "--size-head",
        "perf/size-head.json",
        "--reference-size",
        "perf/size-reference.json",
    ]);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("drift-reference", "size-limit/collie-sdk brotli")],
            1
        ),
        "{}",
        outcome.stdout
    );
}

/// A figure the reference does not hold is new since it: there is nothing to
/// compare it with, and the table says so.
#[test]
fn a_benchmark_new_since_the_reference_is_listed_and_passes() {
    let workspace = Workspace::from_fixture("empty");
    let other = bench_summary(
        "crates/foxhound/goodfetch-frontier",
        "instructions::frontier::pop",
        None,
        &[metric("Ir", None, Some(9_000))],
    );
    workspace.write(
        "perf/head.jsonl",
        format!("{}\n{other}\n", line("Ir", 1_000)).as_bytes(),
    );
    write(&workspace, "perf/reference.jsonl", &line("Ir", 1_000));

    let outcome = compare_with_reference(&workspace);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .lines()
            .any(|line| line.contains("frontier::pop") && line.contains("new since it")),
        "{}",
        outcome.stdout
    );
}

/// A reference that measured nothing the head measured compared nothing: a
/// broken measurement, never a pass.
#[test]
fn a_reference_holding_none_of_the_heads_figures_fails() {
    let workspace = Workspace::from_fixture("empty");
    write(&workspace, "perf/head.jsonl", &line("Ir", 1_000));
    workspace.write("perf/reference.jsonl", b"");

    let outcome = compare_with_reference(&workspace);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding("drift-reference-not-measured", "drift.reference")],
            1
        ),
        "{}",
        outcome.stdout
    );
}

/// The pin file accepting `figure` up to `up_to`, as an ADR 0140 note the
/// owner approved does.
fn accepting(figure: &str, up_to: u64) -> String {
    format!(
        "revision = \"v0.0.0\"\n\n[[accepted]]\nfigure = \"{figure}\"\nup_to = {up_to}\ncause = \"a deliberate cost\"\ndecision = \"ADR 0140, note of 2026-11-13\"\n"
    )
}

/// A figure the owner accepted over 1.10 times its reference is held to its
/// accepted value instead, with the 0.5 % the register gives a figure read
/// under Valgrind: past it, it fails as any drift does.
#[rstest]
#[case::instructions_at_its_tolerance("Ir", "instructions", 1_206, false)]
#[case::instructions_past_its_tolerance("Ir", "instructions", 1_207, true)]
#[case::allocations_at_its_tolerance("TotalBlocks", "total-allocations", 1_206, false)]
#[case::allocations_past_its_tolerance("TotalBlocks", "total-allocations", 1_207, true)]
fn an_accepted_figure_is_held_to_its_accepted_value(
    #[case] key: &str,
    #[case] figure: &str,
    #[case] head: u64,
    #[case] fails: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    let id = format!("{BENCHMARK} {figure}");
    workspace.write(PIN_FILE, accepting(&id, 1_200).as_bytes());
    write(&workspace, "perf/head.jsonl", &line(key, head));
    write(&workspace, "perf/reference.jsonl", &line(key, 1_000));

    let outcome = compare_with_reference(&workspace);

    let expected = if fails {
        (vec![finding("drift-reference", &id)], 1)
    } else {
        (vec![], 0)
    };
    assert_eq!(
        (outcome.findings(), outcome.code),
        expected,
        "{}",
        outcome.stdout
    );
    if !fails {
        assert!(
            outcome
                .stdout
                .lines()
                .any(|line| line.contains("frontier::push")
                    && line.contains("accepted up to 1200 (ADR 0140, note of 2026-11-13)")),
            "{}",
            outcome.stdout
        );
    }
}

/// An acceptance passes its own figure alone: the benchmark's other figures
/// stay held to 1.10 times their reference.
#[test]
fn an_acceptance_passes_no_other_figure() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        PIN_FILE,
        accepting(&format!("{BENCHMARK} total-allocations"), 1_200).as_bytes(),
    );
    write(&workspace, "perf/head.jsonl", &line("Ir", 1_101));
    write(&workspace, "perf/reference.jsonl", &line("Ir", 1_000));

    let outcome = compare_with_reference(&workspace);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding(
                "drift-reference",
                &format!("{BENCHMARK} instructions")
            )],
            1
        ),
        "{}",
        outcome.stdout
    );
}

/// A size-limit entry is not read under Valgrind: its acceptance has no
/// tolerance.
#[rstest]
#[case::at_its_accepted_value(115_000, vec![])]
#[case::past_it(115_001, vec![finding("drift-reference", "size-limit/collie-sdk brotli")])]
fn a_size_limit_entry_is_held_to_its_accepted_value_without_tolerance(
    #[case] head: u64,
    #[case] findings: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        PIN_FILE,
        accepting("size-limit/collie-sdk brotli", 115_000).as_bytes(),
    );
    let entry = |size: u64| format!(r#"[{{"name":"collie-sdk","size":{size}}}]"#);
    workspace.write("perf/size-head.json", entry(head).as_bytes());
    workspace.write("perf/size-reference.json", entry(100_000).as_bytes());

    let outcome = workspace.xtask(&[
        "perf",
        "compare",
        "--size-head",
        "perf/size-head.json",
        "--reference-size",
        "perf/size-reference.json",
    ]);

    assert_eq!(outcome.findings(), findings, "{}", outcome.stdout);
}

/// An acceptance whose figure is back within 1.10 times its reference is
/// no longer needed, and fails until it is removed, so none outlives its
/// cause; its figure is then held to 1.10 alone, even past a value accepted
/// under it.
#[rstest]
#[case::under_its_accepted_value(1_200, 1_100)]
#[case::past_an_accepted_value_under_the_limit(1_050, 1_080)]
fn an_acceptance_whose_figure_is_back_within_the_limit_fails(
    #[case] up_to: u64,
    #[case] head: u64,
) {
    let workspace = Workspace::from_fixture("empty");
    let id = format!("{BENCHMARK} instructions");
    workspace.write(PIN_FILE, accepting(&id, up_to).as_bytes());
    write(&workspace, "perf/head.jsonl", &line("Ir", head));
    write(&workspace, "perf/reference.jsonl", &line("Ir", 1_000));

    let outcome = compare_with_reference(&workspace);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![finding("drift-reference-acceptance-unneeded", &id)], 1),
        "{}",
        outcome.stdout
    );
}

/// A figure accepted twice is refused: which value held it would depend on
/// the entries' order.
#[test]
fn a_figure_accepted_twice_is_refused() {
    let workspace = Workspace::from_fixture("empty");
    let id = format!("{BENCHMARK} instructions");
    let entry = accepting(&id, 1_200);
    let second = entry
        .split_once("[[accepted]]")
        .map(|(_, entry)| format!("\n[[accepted]]{entry}"))
        .unwrap();
    workspace.write(PIN_FILE, format!("{entry}{second}").as_bytes());
    write(&workspace, "perf/head.jsonl", &line("Ir", 1_150));
    write(&workspace, "perf/reference.jsonl", &line("Ir", 1_000));

    let outcome = compare_with_reference(&workspace);

    assert_ne!(outcome.code, 0, "{}", outcome.stdout);
    assert!(outcome.stderr.contains(&id), "{}", outcome.stderr);
}

/// A gate measures only the bench targets its change selects, so an
/// acceptance whose figure it did not compare fails nothing.
#[test]
fn an_acceptance_of_a_figure_not_compared_fails_nothing() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        PIN_FILE,
        accepting(
            "goodfetch-frontier/instructions::frontier::pop instructions",
            1_200,
        )
        .as_bytes(),
    );
    write(&workspace, "perf/head.jsonl", &line("Ir", 1_000));
    write(&workspace, "perf/reference.jsonl", &line("Ir", 1_000));

    let outcome = compare_with_reference(&workspace);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stdout
    );
}

/// A workspace of one crate with one bench target and `pin` as its pin file,
/// committed, and that commit.
fn pinned_workspace(pin: &str) -> (Workspace, String) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "Cargo.toml",
        b"[workspace]\nresolver = \"3\"\nmembers = [\"crates/foxhound/goodfetch-frontier\"]\n",
    );
    workspace.write(
        "Cargo.lock",
        b"version = 4\n\n[[package]]\nname = \"goodfetch-frontier\"\nversion = \"0.0.0\"\n",
    );
    workspace.write(
        "crates/foxhound/goodfetch-frontier/Cargo.toml",
        b"[package]\nname = \"goodfetch-frontier\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[[bench]]\nname = \"instructions\"\nharness = false\n",
    );
    workspace.write(
        "crates/foxhound/goodfetch-frontier/src/lib.rs",
        b"//! The Frontier.\n",
    );
    workspace.write(
        "crates/foxhound/goodfetch-frontier/benches/instructions.rs",
        b"//! A benchmark.\n\nfn main() {}\n",
    );
    workspace.write(PIN_FILE, pin.as_bytes());
    let commit = workspace.commit_all();
    (workspace, commit.trim().to_owned())
}

fn build(workspace: &Workspace) -> Outcome {
    workspace.xtask(&[
        "perf",
        "build",
        "--base",
        "HEAD",
        "--bench-targets",
        "goodfetch-frontier/instructions",
    ])
}

fn plan(workspace: &Workspace) -> Value {
    serde_json::from_slice(&workspace.read("target/perf/plan.json").unwrap()).unwrap()
}

#[test]
fn a_pinned_revision_is_checked_out_and_planned_beside_the_base() {
    let (workspace, commit) = pinned_workspace("");
    workspace.write(PIN_FILE, format!("revision = \"{commit}\"\n").as_bytes());
    workspace.commit_all();

    let outcome = build(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let plan = plan(&workspace);
    assert_eq!(
        (&plan["reference"]["commit"], &plan["reference"]["targets"]),
        (
            &Value::from(commit),
            &serde_json::json!(["goodfetch-frontier/instructions"])
        )
    );
    assert!(
        workspace
            .path()
            .join("target/perf/reference/Cargo.toml")
            .is_file()
    );
}

#[test]
fn with_no_revision_in_the_pin_file_nothing_is_built_for_it() {
    let (workspace, _) = pinned_workspace("# No reference build yet.\n");

    let outcome = build(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(plan(&workspace).get("reference"), None);
    assert!(!workspace.path().join("target/perf/reference").exists());
}

/// The file is the gate's switch: losing it is an error, never a gate that
/// quietly stops.
#[test]
fn a_missing_pin_file_fails_the_build() {
    let (workspace, _) = pinned_workspace("");
    workspace.remove(PIN_FILE);
    workspace.commit_all();

    let outcome = build(&workspace);

    assert_ne!(outcome.code, 0);
    assert!(outcome.stderr.contains(PIN_FILE), "{}", outcome.stderr);
}

#[test]
fn a_pinned_revision_that_does_not_exist_fails_the_build() {
    let (workspace, _) = pinned_workspace("revision = \"no-such-tag\"\n");

    let outcome = build(&workspace);

    assert_ne!(outcome.code, 0);
    assert!(outcome.stderr.contains("no-such-tag"), "{}", outcome.stderr);
}

/// A stand-in for `cargo bench` measuring `reference` in the reference's tree
/// and `head` elsewhere.
fn stand_in_cargo(workspace: &Workspace, reference: u64, head: u64) -> std::path::PathBuf {
    write(
        workspace,
        "stand-in/reference.jsonl",
        &line("Ir", reference),
    );
    write(workspace, "stand-in/head.jsonl", &line("Ir", head));
    let folder = workspace.path().join("stand-in");
    let script = format!(
        r#"#!/bin/sh
case "$PWD" in
  */target/perf/reference) cat "{folder}/reference.jsonl" ;;
  *) cat "{folder}/head.jsonl" ;;
esac
"#,
        folder = folder.display(),
    );
    let cargo = folder.join("cargo");
    std::fs::write(&cargo, script).unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    cargo
}

/// `xtask perf run` over a plan with a reference: the path a gate takes from
/// the plan to the failure.
#[rstest]
#[case::within(1_100, vec![])]
#[case::over(1_101, vec![finding("drift-reference", &format!("{BENCHMARK} instructions"))])]
fn a_run_measures_the_reference_and_holds_the_head_to_it(
    #[case] head: u64,
    #[case] findings: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("empty");
    let commit = workspace.commit_all();
    let tree = workspace.path().join("target/perf/reference");
    workspace.git(&[
        "worktree",
        "add",
        "--quiet",
        "--detach",
        tree.to_str().unwrap(),
        commit.trim(),
    ]);
    let cargo = stand_in_cargo(&workspace, 1_000, head);
    let target = "goodfetch-frontier/instructions";
    let plan = serde_json::json!({
        "base": commit.trim(),
        "head_targets": [target],
        "base_targets": [],
        "target_directory": workspace.path().join("target"),
        "reference": { "commit": commit.trim(), "targets": [target] },
    });
    workspace.write(
        "target/perf/plan.json",
        serde_json::to_vec(&plan).unwrap().as_slice(),
    );

    let outcome = workspace.xtask_with_env(&["perf", "run"], &[("CARGO", cargo.as_os_str())]);

    assert_eq!(outcome.findings(), findings, "{}", outcome.stdout);
    assert!(
        workspace
            .read("target/junit/perf/reference.jsonl")
            .is_some(),
        "the reference's report joins the job's others"
    );
    assert!(!tree.exists(), "the reference's tree is removed");
}
