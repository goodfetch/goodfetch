//! Hardening tests for the incremental local gate (ADR 0135; issue 1726):
//! - Signed gate records: forged, unsigned, tampered, or foreign-key lines are
//!   never reused.
//! - `xtask gate verify`: refuses missing, red, unsigned, wrong-tree, or
//!   non-ancestor reused results before merge, and prints every verified
//!   reused ancestor.
//! - `GOODFETCH_GATE_FULL` out of the host fingerprint: the run after a forced
//!   full run reuses its green results.
//! - Planted-break tests: fault injection through real Cargo dep-info
//!   (`include_str!`), root `vitest.config.ts`, and every single gate step in
//!   `Step::ALL`.
//!
//! Repository tooling: declares no Feature.
#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::affected::{commit_all, repository, value};
use crate::gate::{gated_repository, write_signed_record};
use crate::workspace::{Outcome, Workspace};

const NOW: &str = "1800000000";

fn records_dir() -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let records = folder.path().join("gate-runs");
    (folder, records)
}

fn target_dir(records: &Path) -> PathBuf {
    records.with_file_name("target")
}

fn plan_raw(workspace: &Workspace, records: &Path, environment: &str, extra: &[&str]) -> Outcome {
    let target = target_dir(records);
    let rec_str = records.to_string_lossy();
    let tgt_str = target.to_string_lossy();
    let mut args = vec![
        "gate",
        "plan",
        "--records",
        &rec_str,
        "--now",
        NOW,
        "--target-dir",
        &tgt_str,
        "--environment",
        environment,
    ];
    args.extend(extra);
    workspace.xtask(&args)
}

fn record_step(workspace: &Workspace, records: &Path, head: &str, step: &str, status: &str) {
    let out = workspace.xtask(&[
        "gate",
        "record",
        "--by",
        "merge",
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
    assert_eq!(out.code, 0, "record {step}={status}: {}", out.stderr);
}

fn parse_plan_actions(out: &Outcome) -> (String, String, BTreeMap<String, String>) {
    let mut previous = String::new();
    let mut head = String::new();
    let mut actions = BTreeMap::new();
    for line in out.stdout.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k == "previous" {
            v.clone_into(&mut previous);
        } else if k == "head" {
            v.clone_into(&mut head);
        } else if let Some(step) = k.strip_suffix(".action") {
            actions.insert(step.to_owned(), v.to_owned());
        }
    }
    (previous, head, actions)
}

fn record_all_planned(
    workspace: &Workspace,
    records: &Path,
    head: &str,
    actions: &BTreeMap<String, String>,
    failing: &[&str],
) {
    for (step, action) in actions {
        if action == "run" || action == "part" {
            let status = if failing.contains(&step.as_str()) {
                "fail"
            } else {
                "pass"
            };
            record_step(workspace, records, head, step, status);
        }
    }
}

fn verify(workspace: &Workspace, records: &Path, head: &str, form: &str) -> Outcome {
    workspace.xtask(&[
        "gate",
        "verify",
        "--records",
        &records.to_string_lossy(),
        "--head",
        head,
        "--form",
        form,
    ])
}

/// AC1: A record line not signed by the host's gate key (forged/unsigned or
/// tampered) is never reused.
#[test]
fn forged_or_unsigned_record_is_not_reused() {
    let workspace = repository();
    let (_folder, records) = records_dir();
    let first = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    let tree = workspace
        .git(&["rev-parse", "HEAD^{tree}"])
        .trim()
        .to_owned();
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join(format!("{first}.tsv")),
        format!("base\tb\nhost\th\nperf\tpass\t{first}\t{tree}\t{NOW}\tmerge\n"),
    )
    .unwrap();
    workspace.write("docs/forged.md", b"# Changed\n");
    commit_all(&workspace, "Change docs");

    let out = plan_raw(&workspace, &records, "CI=", &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let (previous, _, _) = parse_plan_actions(&out);
    assert_eq!(previous, "none");
    assert!(
        out.stderr.contains("not signed by this host's gate key"),
        "{}",
        out.stderr
    );
}

/// AC4: `GOODFETCH_GATE_FULL` is kept out of the host fingerprint, so the
/// incremental run after a forced full run reuses its green results.
#[test]
fn run_after_forced_full_run_reuses_its_results() {
    let (workspace, _folder, records) = gated_repository(&[]);
    let full = plan_raw(
        &workspace,
        &records,
        "CI=\nGOODFETCH_GATE_FULL=1",
        &["--full"],
    );
    assert_eq!(full.code, 0, "{}", full.stderr);
    let (_, first_head, first_actions) = parse_plan_actions(&full);
    record_all_planned(&workspace, &records, &first_head, &first_actions, &[]);

    workspace.write("docs/after-full.md", b"# Doc only\n");
    commit_all(&workspace, "Doc change after full run");

    let next = plan_raw(&workspace, &records, "CI=", &[]);
    assert_eq!(next.code, 0, "{}", next.stderr);
    let (previous, _, actions) = parse_plan_actions(&next);
    assert_eq!(previous, first_head);
    assert_eq!(actions.get("perf").map(String::as_str), Some("reuse"));
    assert_eq!(actions.get("tlc").map(String::as_str), Some("reuse"));
}

/// AC2: `xtask gate verify` accepts a complete signed verdict (printing reused
/// ancestor origins) and refuses red, missing, or foreign/non-ancestor results.
#[test]
fn gate_verify_accepts_signed_verdict_and_refuses_red_missing_or_foreign() {
    let workspace = repository();
    let (_folder, records) = records_dir();
    let first_out = plan_raw(&workspace, &records, "CI=", &[]);
    let (_, first, first_actions) = parse_plan_actions(&first_out);
    record_all_planned(&workspace, &records, &first, &first_actions, &[]);

    // 1. Complete signed green run verifies with no reused origins.
    let ok_first = verify(&workspace, &records, &first, "all");
    assert_eq!(ok_first.code, 0, "{}", ok_first.stderr);
    assert!(ok_first.stdout.trim().is_empty(), "{}", ok_first.stdout);

    // 2. Descendant reusing green results verifies and prints reused origin.
    workspace.write("docs/reused.md", b"# Reused\n");
    commit_all(&workspace, "Doc change");
    let second_out = plan_raw(&workspace, &records, "CI=", &[]);
    let (_, second, second_actions) = parse_plan_actions(&second_out);
    record_all_planned(&workspace, &records, &second, &second_actions, &[]);
    let ok_second = verify(&workspace, &records, &second, "all");
    assert_eq!(ok_second.code, 0, "{}", ok_second.stderr);
    assert!(
        ok_second.stdout.contains(&format!("origin\t{first}")),
        "{}",
        ok_second.stdout
    );

    // 3. Red unit is refused.
    record_step(&workspace, &records, &second, "lint", "fail");
    let red = verify(&workspace, &records, &second, "all");
    assert_ne!(red.code, 0);
    assert!(red.stderr.contains("lint is red on"), "{}", red.stderr);

    // 4. Missing unit is refused.
    workspace.write("docs/missing.md", b"# Missing\n");
    commit_all(&workspace, "Missing step commit");
    let third_out = plan_raw(&workspace, &records, "CI=", &[]);
    let (_, third, _) = parse_plan_actions(&third_out);
    let missing = verify(&workspace, &records, &third, "all");
    assert_ne!(missing.code, 0);
    assert!(
        missing.stderr.contains("has no result"),
        "{}",
        missing.stderr
    );

    // 5. Foreign (non-ancestor) origin is refused even if validly signed.
    workspace.git(&["switch", "--quiet", "--create", "aside", &first]);
    workspace.write("docs/aside.md", b"# Aside\n");
    commit_all(&workspace, "Side branch commit");
    let aside = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    let aside_tree = workspace
        .git(&["rev-parse", "HEAD^{tree}"])
        .trim()
        .to_owned();
    workspace.git(&["switch", "--quiet", "change"]);
    write_signed_record(
        &workspace,
        &records,
        &aside,
        &[
            "base\tb",
            "host\th",
            &format!("perf\tpass\t{aside}\t{aside_tree}\t{NOW}\tmerge"),
        ],
    );
    write_signed_record(
        &workspace,
        &records,
        &third,
        &[
            "base\tb",
            "host\th",
            &format!("perf\tpass\t{aside}\t{aside_tree}\t{NOW}\tmerge"),
        ],
    );
    let foreign = verify(&workspace, &records, &third, "all");
    assert_ne!(foreign.code, 0);
    assert!(
        foreign
            .stderr
            .contains(&format!("{aside} is not an ancestor of {third}")),
        "{}",
        foreign.stderr
    );
}

fn cargo_test(workspace: &Workspace, records: &Path, args: &[&str]) -> std::process::Output {
    workspace
        .command("cargo")
        .args(["test", "--offline", "--quiet"])
        .args(args)
        .env("CARGO_TARGET_DIR", target_dir(records))
        .output()
        .unwrap()
}

/// AC5 (real dep-info fault injection): a break planted in a file one crate
/// holds that another crate reads via `include_str!` is caught by Cargo's real
/// dep-info, forces a rerun of the reading crate, and turns `cargo test` red.
#[test]
fn planted_break_through_cross_crate_include_reruns_and_fails() {
    let workspace = repository();
    let (_folder, records) = records_dir();
    workspace.write("crates/goodfetch-engine/limits.txt", b"3\n");
    workspace.write(
        "crates/goodfetch-conformance/src/lib.rs",
        b"//! Harness.\npub const LIMIT: &str = include_str!(\"../../goodfetch-engine/limits.txt\");\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn limit_is_three() {\n        assert_eq!(super::LIMIT.trim(), \"3\");\n    }\n}\n",
    );
    let built = cargo_test(&workspace, &records, &["--workspace"]);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    commit_all(&workspace, "Check engine limit via include_str!");
    let first = plan_raw(&workspace, &records, "CI=", &[]);
    let (_, first_head, first_actions) = parse_plan_actions(&first);
    record_all_planned(&workspace, &records, &first_head, &first_actions, &[]);

    workspace.write("crates/goodfetch-engine/limits.txt", b"4\n");
    commit_all(&workspace, "Plant break in engine limit");

    let second = plan_raw(&workspace, &records, "CI=", &[]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    let (_, _, actions) = parse_plan_actions(&second);
    assert_eq!(actions.get("pre-merge").map(String::as_str), Some("part"));
    assert!(
        value(&second, "pre-merge.members")
            .split(' ')
            .any(|m| m == "goodfetch-conformance"),
        "{}",
        second.stdout
    );
    let rerun = cargo_test(&workspace, &records, &["-p", "goodfetch-conformance"]);
    assert!(!rerun.status.success(), "planted break must turn red");
}

/// AC5 (root Vitest config fault injection): a break planted in a package that
/// only the root `vitest.config.ts` loads forces every TypeScript package to
/// rerun.
#[test]
fn planted_break_in_root_vitest_config_package_reruns_every_package() {
    let (workspace, _folder, records) = gated_repository(&[]);
    workspace.write(
        "vitest.config.ts",
        b"import { setup } from '@goodfetch/foxhound';\nexport default { test: { setupFiles: [setup] } };\n",
    );
    commit_all(&workspace, "Load Foxhound SDK in root Vitest config");
    let first = plan_raw(&workspace, &records, "CI=", &[]);
    let (_, first_head, first_actions) = parse_plan_actions(&first);
    record_all_planned(&workspace, &records, &first_head, &first_actions, &[]);

    workspace.write(
        "sdks/typescript/foxhound/src/setup.ts",
        b"throw new Error('planted');\n",
    );
    commit_all(&workspace, "Plant break in root Vitest setup");

    let second = plan_raw(&workspace, &records, "CI=", &[]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    let members_line = value(&second, "pre-merge.members");
    let rerun: BTreeSet<&str> = members_line.split(' ').collect();
    for pkg in ["sdks/typescript/foxhound", "sdks/typescript/collie-client"] {
        assert!(rerun.contains(pkg), "{pkg} not in {rerun:?}");
    }
}

/// Which file break triggers a rerun of each gate step, and which gate form
/// runs that step.
fn planted_break_for_step(step: &str) -> (&'static str, &'static str, &'static [u8]) {
    match step {
        "dco"
        | "commit-messages"
        | "floor-guard"
        | "commit-leaks"
        | "audit"
        | "notices"
        | "lint"
        | "feature-combinations"
        | "semver-checks"
        | "deslop-please"
        | "fuzz-check"
        | "proofs"
        | "journeys-tasks"
        | "features-tests" => ("all", "docs/planted-break.md", b"# Planted break\n"),
        "perf" | "counts" | "linux" | "msrv" | "miri" | "loom" | "simulation" | "windows"
        | "pre-merge" | "stress" => (
            "all",
            "crates/goodfetch-engine/src/lib.rs",
            b"//! Planted break in engine.\n",
        ),
        "visual" => ("all", "ci/visual/image.toml", b"# planted visual change\n"),
        "tlc" => (
            "all",
            "test-support/models/tools.toml",
            b"# planted tlc tools change\n",
        ),
        "kennel" => ("all", "kennel/src/lib.rs", b"//! Planted kennel break.\n"),
        "quick" | "codegen-tests" => (
            "quick",
            "crates/goodfetch-engine/src/lib.rs",
            b"//! Planted quick/codegen break.\n",
        ),
        other => panic!("missing planted-break case for gate step `{other}`"),
    }
}

type StepGroups<'a> = BTreeMap<(&'a str, &'a str), (Vec<String>, &'a [u8])>;

/// AC5 (exhaustive): every gate step in `xtask gate steps` (across `all` and
/// `quick`) reruns when a break is planted in what it covers, turns red, and
/// causes `xtask gate verify` to refuse the merge.
#[test]
fn every_gate_step_turns_red_on_a_planted_break() {
    let probe = Workspace::from_fixture("empty");
    let mut all_steps = BTreeSet::new();
    for form in ["all", "quick"] {
        let out = probe.xtask(&["gate", "steps", "--form", form]);
        assert_eq!(out.code, 0, "{}", out.stderr);
        for step in value(&out, "steps").split(' ') {
            all_steps.insert(step.to_owned());
        }
    }
    assert!(
        all_steps.len() >= 29,
        "expected every gate step, got {all_steps:?}"
    );

    // Group steps by (form, planted_file) so we verify all 29 steps rapidly
    // without creating 29 full git repos.
    let mut groups: StepGroups<'_> = BTreeMap::new();
    for step in &all_steps {
        let (form, file, bytes) = planted_break_for_step(step);
        groups
            .entry((form, file))
            .or_insert_with(|| (Vec::new(), bytes))
            .0
            .push(step.clone());
    }

    for ((form, file), (steps, bytes)) in groups {
        let workspace = repository();
        let (_folder, records) = records_dir();
        let first_out = plan_raw(&workspace, &records, "CI=", &["--form", form]);
        assert_eq!(first_out.code, 0, "{}", first_out.stderr);
        let (_, first, first_actions) = parse_plan_actions(&first_out);
        record_all_planned(&workspace, &records, &first, &first_actions, &[]);

        workspace.write(file, bytes);
        commit_all(&workspace, &format!("Plant break in {file}"));

        let second_out = plan_raw(&workspace, &records, "CI=", &["--form", form]);
        assert_eq!(second_out.code, 0, "{}", second_out.stderr);
        let (_, second, second_actions) = parse_plan_actions(&second_out);
        for step in &steps {
            let act = second_actions.get(step).map_or("missing", String::as_str);
            assert!(
                act == "run" || act == "part",
                "planted break in {file} must rerun step `{step}` in form `{form}`, got action `{act}`"
            );
        }
        // Record every non-target step green and each target step red one by one,
        // proving `gate verify` refuses every single red gate step.
        record_all_planned(&workspace, &records, &second, &second_actions, &[]);
        for step in &steps {
            record_step(&workspace, &records, &second, step, "fail");
            let verdict = verify(&workspace, &records, &second, form);
            assert_ne!(
                verdict.code, 0,
                "verdict must refuse red step `{step}` in form `{form}`"
            );
            assert!(
                verdict.stderr.contains(&format!("{step} is red on")),
                "expected `{step} is red on` in stderr, got: {}",
                verdict.stderr
            );
            record_step(&workspace, &records, &second, step, "pass");
        }
    }
}
