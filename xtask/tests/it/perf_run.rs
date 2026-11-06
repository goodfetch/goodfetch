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

/// The targets the base and the head measure, each one benchmark.
const MEASURED: [(&str, &str); 3] = [
    (
        "goodfetch-frontier/instructions",
        "instructions::frontier::push",
    ),
    ("goodfetch-frontier/heap", "heap::frontier::push"),
    ("goodfetch-frontier/admission", "admission::frontier::push"),
];

/// A stand-in for `cargo bench` that prints the summary of the target it is
/// given, with the base's figure once the head runs against it. A base run
/// (`--save-baseline=base`) waits, up to 10 s, until another base run runs
/// beside it, and fails when they go one at a time; a head run
/// (`--baseline=base`) fails unless every base run has ended.
fn measuring_cargo(workspace: &Workspace) -> std::path::PathBuf {
    let folder = workspace.path().join("stand-in");
    let runs = folder.join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    for (target, module) in MEASURED {
        let package = "crates/foxhound/goodfetch-frontier";
        let name = target.replace('/', "-");
        let base = bench_summary(package, module, None, &[metric("Ir", None, Some(1_000))]);
        let head = bench_summary(
            package,
            module,
            None,
            &[metric("Ir", Some(1_000), Some(1_000))],
        );
        workspace.write(
            &format!("stand-in/{name}.base.jsonl"),
            format!("{base}\n").as_bytes(),
        );
        workspace.write(
            &format!("stand-in/{name}.head.jsonl"),
            format!("{head}\n").as_bytes(),
        );
    }
    let script = format!(
        r#"#!/bin/sh
package="" bench=""
while [ $# -gt 0 ]; do
  case "$1" in
    --package) package=$2; shift ;;
    --bench) bench=$2; shift ;;
    --save-baseline=*) side=base ;;
    --baseline=*) side=head ;;
  esac
  shift
done
name="$package-$bench"
if [ "$side" = base ]; then
  touch "{runs}/running.$name"
  waited=0
  until [ -e "{runs}/overlapped" ]; do
    if [ "$(ls "{runs}" | grep -c '^running\.')" -gt 1 ]; then
      touch "{runs}/overlapped"
    elif [ "$waited" -ge 200 ]; then
      echo "no other base run started beside this one" >&2
      exit 1
    else
      sleep 0.05
      waited=$((waited + 1))
    fi
  done
  sleep 0.2
  rm "{runs}/running.$name"
  touch "{runs}/saved.$name"
else
  if [ "$(ls "{runs}" | grep -c '^saved\.')" -ne {count} ]; then
    echo "the head's $name ran before every base run had saved its baseline" >&2
    exit 1
  fi
fi
cat "{folder}/$name.$side.jsonl"
"#,
        runs = runs.display(),
        folder = folder.display(),
        count = MEASURED.len(),
    );
    let cargo = folder.join("cargo");
    std::fs::write(&cargo, script).unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    cargo
}

/// The base's targets are measured at once, then the head's, each against
/// its saved baseline, since counts under Callgrind and DHAT do not depend
/// on what else runs (the test-gate audit's F2); each side's report keeps
/// the plan's order of targets.
#[test]
fn the_bases_targets_are_measured_at_once_then_the_heads() {
    let workspace = Workspace::from_fixture("empty");
    let base = workspace.commit_all();
    let cargo = measuring_cargo(&workspace);
    let targets: Vec<&str> = MEASURED.iter().map(|(target, _)| *target).collect();
    let plan = serde_json::json!({
        "base": base,
        "head_targets": targets,
        "base_targets": targets,
        "target_directory": workspace.path().join("target"),
    });
    workspace.write(
        "target/perf/plan.json",
        serde_json::to_vec(&plan).unwrap().as_slice(),
    );
    workspace.write("target/perf-base-tree/Cargo.toml", b"");

    let outcome = workspace.xtask_with_env(&["perf", "run"], &[("CARGO", cargo.as_os_str())]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), vec![]);
    for side in ["base", "head"] {
        let report = String::from_utf8(
            workspace
                .read(&format!("target/perf/{side}.jsonl"))
                .unwrap(),
        )
        .unwrap();
        let expected: String = MEASURED
            .iter()
            .map(|(target, _)| {
                String::from_utf8(
                    workspace
                        .read(&format!(
                            "stand-in/{}.{side}.jsonl",
                            target.replace('/', "-")
                        ))
                        .unwrap(),
                )
                .unwrap()
            })
            .collect();
        assert_eq!(report, expected, "{side}");
    }
}
