//! `xtask perf run`, under the deny (ADR 0071): it measures what `xtask perf
//! build` planned, then runs each head target holding a new benchmark 20
//! more times on the same build for its admission. Those runs are separate
//! processes, several at once, since counts under Callgrind and DHAT do not
//! depend on what else runs. A stand-in Cargo, a shell script, plays `cargo
//! bench` here, so these tests run on Unix only.

use std::os::unix::fs::PermissionsExt as _;

use crate::perf::{bench_summary, metric};
use crate::workspace::Workspace;

const TARGET: &str = "goodfetch-frontier/instructions";

/// A stand-in for `cargo bench` that prints one new benchmark's gungraun
/// summary. An admission run (one without `--baseline`) first waits, up to
/// 10 s, until another admission run is running beside it, so it fails
/// when the runs go one at a time.
fn stand_in_cargo(workspace: &Workspace) -> std::path::PathBuf {
    let summary = bench_summary(
        "crates/foxhound/goodfetch-frontier",
        "instructions::frontier::push",
        None,
        &[metric("Ir", None, Some(1_000_000))],
    );
    workspace.write("stand-in/summary.jsonl", format!("{summary}\n").as_bytes());
    let folder = workspace.path().join("stand-in");
    let runs = folder.join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    let script = format!(
        r#"#!/bin/sh
case "$*" in
  *--baseline=*) ;;
  *)
    touch "{runs}/running.$$"
    waited=0
    until [ -e "{runs}/overlapped" ]; do
      if [ "$(ls "{runs}" | grep -c '^running\.')" -gt 1 ]; then
        touch "{runs}/overlapped"
      elif [ "$waited" -ge 200 ]; then
        echo "no other admission run started beside this one" >&2
        exit 1
      else
        sleep 0.05
        waited=$((waited + 1))
      fi
    done
    rm "{runs}/running.$$"
    ;;
esac
cat "{summary}"
"#,
        runs = runs.display(),
        summary = folder.join("summary.jsonl").display(),
    );
    let cargo = folder.join("cargo");
    std::fs::write(&cargo, script).unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    cargo
}

#[test]
fn admission_runs_several_runs_at_once_and_admits_on_all_20() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let cargo = stand_in_cargo(&workspace);
    let plan = serde_json::json!({
        "base": base,
        "head_targets": [TARGET],
        "base_targets": [],
        "target_directory": workspace.path().join("target"),
    });
    workspace.write(
        "target/perf/plan.json",
        serde_json::to_vec(&plan).unwrap().as_slice(),
    );

    let outcome = workspace.xtask_with_env(&["perf", "run"], &[("CARGO", cargo.as_os_str())]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}
