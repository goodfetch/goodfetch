//! The pre-push tier (T1), `just quick` (`ci/quick.sh`), which the pre-push
//! hook runs: the floor guard and the scan of the change's commits first,
//! then the checks the change's own gate would run at merge. A change of
//! documents alone (`xtask affected`'s `gate=docs`) takes the docs gate's
//! checks, as `just merge` does (ADR 0135); any other change, and `all`,
//! takes every lint and the Quick suite. The steps run through a stand-in
//! `just` that records each call, and `xtask affected` through a stand-in
//! `cargo`. The pre-push tier is repository tooling, so the module declares
//! no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::workspace::Workspace;

/// A stand-in `just` that appends its arguments to `calls` and fails with
/// status 3 when they are `failing`'s content, and a stand-in `cargo` that
/// appends its own to `cargo-calls` and prints `selection`'s content, as
/// `xtask affected` prints its selection.
fn stand_ins(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let just = format!(
        r#"#!/bin/sh
echo "$*" >>"{calls}"
if [ -f "{failing}" ] && [ "$*" = "$(cat "{failing}")" ]; then
  exit 3
fi
"#,
        calls = folder.join("calls").display(),
        failing = folder.join("failing").display(),
    );
    let cargo = format!(
        r#"#!/bin/sh
echo "$*" >>"{calls}"
cat "{selection}"
"#,
        calls = folder.join("cargo-calls").display(),
        selection = folder.join("selection").display(),
    );
    for (name, script) in [("just", just), ("cargo", cargo)] {
        let path = folder.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

/// `xtask affected`'s selection for a change whose gate is `gate`.
fn selection(stand_in: &Path, gate: &str) {
    std::fs::write(
        stand_in.join("selection"),
        format!("products=\njobs=[\"docs\"]\ncrates=\ngate={gate}\n"),
    )
    .unwrap();
}

struct Run {
    code: Option<i32>,
    calls: Vec<String>,
    cargo_calls: Vec<String>,
    stdout: String,
    stderr: String,
}

/// Runs the repository's `ci/quick.sh` in the workspace, with the stand-ins
/// first on `PATH`.
fn quick(workspace: &Workspace, stand_in: &Path, args: &[&str]) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/quick.sh");
    let output = workspace
        .command(script)
        .args(args)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .output()
        .unwrap();
    let read = |file: &str| -> Vec<String> {
        std::fs::read_to_string(stand_in.join(file))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    Run {
        code: output.status.code(),
        calls: read("calls"),
        cargo_calls: read("cargo-calls"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// What `xtask affected` is asked: the working tree's change since the
/// merge base with `origin/main`, as the Quick suite's own selection reads
/// it.
const AFFECTED: &str = "run --quiet --locked -p xtask -- affected --local";

#[test]
fn a_change_of_documents_alone_runs_the_docs_gates_checks_and_no_code_lint_or_suite() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "docs");

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "floor-guard",
            "commit-leaks",
            "lint-documents",
            "_skips-report"
        ]
    );
    assert_eq!(run.cargo_calls, [AFFECTED]);
    assert!(run.stdout.contains("gate=docs"), "{}", run.stdout);
}

#[rstest]
#[case::no_test_reads_it("quick")]
#[case::a_test_reads_it("full")]
fn any_other_change_runs_every_lint_and_the_quick_suite_over_what_changed(#[case] gate: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, gate);

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "floor-guard",
            "commit-leaks",
            "lint",
            "_suite quick ",
            "_skips-report"
        ]
    );
}

#[test]
fn all_runs_every_lint_and_the_whole_quick_suite_without_asking_what_changed() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "docs");

    let run = quick(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "floor-guard",
            "commit-leaks",
            "lint",
            "_suite quick all",
            "_skips-report"
        ]
    );
    assert!(run.cargo_calls.is_empty(), "{:?}", run.cargo_calls);
}

#[rstest]
#[case::floor_guard("floor-guard", &["floor-guard", "_skips-report"])]
#[case::commit_leaks("commit-leaks", &["floor-guard", "commit-leaks", "_skips-report"])]
fn a_red_commit_check_ends_the_tier_before_any_lint(#[case] failing: &str, #[case] calls: &[&str]) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);
    selection(&stand_in, "docs");
    std::fs::write(stand_in.join("failing"), failing).unwrap();

    let run = quick(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert_eq!(run.calls, calls);
}

#[test]
fn an_unknown_argument_is_refused_before_any_step() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_ins(&workspace);

    let run = quick(&workspace, &stand_in, &["everything"]);

    assert_eq!(run.code, Some(2), "{}", run.stderr);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
}
