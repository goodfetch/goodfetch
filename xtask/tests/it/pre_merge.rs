//! The Pre-merge suite on a local host (`just pre-merge`, `ci/pre-merge.sh`),
//! which the local gate runs before every merge (ADR 0135): every check of
//! TESTING.md's Pre-merge tier runs, the ones that need Linux beside the host
//! steps, and the suite fails when any of them fails. The steps run through a
//! stand-in `just` that records each call. The local gate is repository
//! tooling, so the module declares no Feature.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::workspace::Workspace;

/// Every step of `just pre-merge all`: the host's in the order it runs them,
/// Collie's Console build first, which the steps beside find in the
/// workspace they mount, then those beside them.
const STEPS: [&str; 22] = [
    "console-build",
    "dco",
    "commit-messages",
    "lint",
    "feature-combinations",
    "audit",
    "commit-leaks",
    "fuzz-check",
    "proofs",
    "_suite quick all",
    "_suite pre-merge all",
    "features-audit",
    "stress",
    "journeys-tasks --check",
    "features-tests clients-tests",
    "_tlc all",
    "msrv",
    "miri",
    "notices",
    "perf",
    "linux",
    "visual",
];

/// The steps that need Linux, run beside the host's.
const BESIDE: [&str; 3] = ["perf", "linux", "visual"];

/// A stand-in `just` that appends its arguments to `calls` and fails with
/// status 3 when they are `failing`'s content.
fn stand_in_just(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let script = format!(
        r#"#!/bin/sh
echo "$*" >>"{calls}"
if [ -f "{failing}" ] && [ "$*" = "$(cat "{failing}")" ]; then
  exit 3
fi
"#,
        calls = folder.join("calls").display(),
        failing = folder.join("failing").display(),
    );
    let just = folder.join("just");
    std::fs::write(&just, script).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&just, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

struct Run {
    code: Option<i32>,
    calls: Vec<String>,
    stderr: String,
}

/// Runs the repository's `ci/pre-merge.sh` in the workspace, with the
/// stand-in first on `PATH`.
fn pre_merge(workspace: &Workspace, stand_in: &Path, args: &[&str]) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/pre-merge.sh");
    let path = format!("{}:/usr/bin:/bin", stand_in.display());
    let output = workspace
        .command(script)
        .args(args)
        .env("PATH", path)
        .output()
        .unwrap();
    let calls = std::fs::read_to_string(stand_in.join("calls")).unwrap_or_default();
    Run {
        code: output.status.code(),
        calls: calls.lines().map(str::to_owned).collect(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn every_pre_merge_check_runs_and_the_skipped_parts_are_listed_last() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut ran = run.calls.clone();
    assert_eq!(ran.pop().as_deref(), Some("_skips-report"));
    ran.sort();
    let mut expected = STEPS.map(str::to_owned).to_vec();
    expected.sort();
    assert_eq!(ran, expected);
    let host: Vec<&String> = run
        .calls
        .iter()
        .filter(|call| !BESIDE.contains(&call.as_str()) && *call != "_skips-report")
        .collect();
    assert_eq!(
        host,
        STEPS[..STEPS.len() - BESIDE.len()]
            .iter()
            .collect::<Vec<_>>()
    );
}

#[test]
fn without_all_the_suites_run_over_what_changed() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut ran = run.calls;
    ran.sort();
    // The same steps, `all` left empty: `just _suite quick ""`.
    let mut expected: Vec<String> = STEPS
        .iter()
        .map(|step| step.replace(" all", " "))
        .chain(["_skips-report".to_owned()])
        .collect();
    expected.sort();
    assert_eq!(ran, expected);
}

#[rstest]
#[case::perf("perf")]
#[case::linux("linux")]
#[case::visual("visual")]
fn a_failed_check_beside_the_host_steps_fails_the_suite(#[case] step: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), step).unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        run.stderr.contains(&format!("just {step} failed")),
        "{}",
        run.stderr
    );
    assert!(
        run.calls.iter().any(|call| call == "notices"),
        "{:?}",
        run.calls
    );
}

#[test]
fn a_failed_stress_run_stops_the_suite_before_the_later_steps() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), "stress").unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        !run.calls.iter().any(|call| call == "notices"),
        "{:?}",
        run.calls
    );
    assert_eq!(run.calls.last().map(String::as_str), Some("_skips-report"));
}
