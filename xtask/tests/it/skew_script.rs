//! A skew job of the Release suite (`ci/release/skew.sh`, ADR 0066) on a
//! sample `ci/release/skew-jobs.json`: `build` keeps both Products' current
//! executables and fetches a minor's tag, and `run` runs each of the job's
//! steps through `just journeys` and fails on one that ran no test. A
//! stand-in `cargo` plays `xtask test-tools` and a stand-in `just` records
//! each call and writes the JUnit file a run leaves.
//! Repository tooling: the module declares no Feature.
#![cfg(target_os = "linux")]

use std::path::Path;

use rstest::rstest;

use crate::workspace::{Workspace, write_executable};

const JOBS: &str = r#"{
  "jobs": [
    {"name": "current", "version": null, "tag": null, "minutes": 35, "steps": [
      {"checks": ["collie-cli"], "executable": "goodfetch-collie", "stdio": false, "journeys": ["--product", "collie", "--surface", "cli"]},
      {"checks": ["foxhound-cli", "foxhound-mcp"], "executable": "goodfetch-foxhound", "stdio": false, "journeys": ["--product", "foxhound", "--surface", "cli"]}
    ]},
    {"name": "1.2", "version": "1.2.3", "tag": "v1.2.3", "minutes": 30, "steps": [
      {"checks": ["collie-sdk-typescript-stdio"], "executable": "goodfetch-collie", "stdio": true, "journeys": ["--product", "collie", "--surface", "sdk-typescript", "--exclude", "J-001@sdk-typescript"]}
    ]}
  ],
  "awaiting": [{"check": "two-majors", "issue": 983}]
}
"#;

/// The JUnit file of a run whose one test passed.
const ONE_TEST: &str =
    r#"<testsuites><testsuite name="s"><testcase name="t"/></testsuite></testsuites>"#;

struct Run {
    code: Option<i32>,
    /// The stand-in `just`'s calls, each with the tree's `released` file and
    /// the engine paths it inherits; the workspace's path is left out.
    calls: Vec<String>,
    stdout: String,
    stderr: String,
}

/// A workspace holding [`JOBS`], the tag `v1.2.3` and the stand-ins. A call of
/// `just` that `failing` is part of fails; `junit.xml`, when there is one,
/// becomes its run's JUnit file.
fn workspace() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(".gitignore", b"/stand-in/\n/target/\n");
    workspace.write("ci/release/skew-jobs.json", JOBS.as_bytes());
    workspace.write("released", b"1.2.3\n");
    workspace.commit_all();
    workspace.git(&["tag", "v1.2.3"]);
    workspace.write("released", b"none\n");
    workspace.commit_all();
    workspace.write("stand-in/junit.xml", ONE_TEST.as_bytes());
    for product in ["collie", "foxhound"] {
        workspace.write(&format!("stand-in/built/{product}"), product.as_bytes());
    }
    let root = workspace.path().display();
    write_executable(
        &workspace.path().join("stand-in/cargo"),
        &format!(
            r#"#!/bin/sh
while [ "$1" != --out ]; do shift; done
echo '{{"collie": "{root}/stand-in/built/collie", "foxhound": "{root}/stand-in/built/foxhound"}}' >"$2"
"#
        ),
    );
    write_executable(
        &workspace.path().join("stand-in/just"),
        &format!(
            r#"#!/bin/sh
call="$*"
echo "$call released=$(cat released) engines=${{GOODFETCH_COLLIE_ENGINE_PATH-}},${{GOODFETCH_FOXHOUND_ENGINE_PATH-}}" | sed "s|{root}/||g" >>"{root}/stand-in/calls"
while [ $# -gt 0 ] && [ "$1" != --junit ]; do shift; done
if [ $# -gt 0 ] && [ -f "{root}/stand-in/junit.xml" ]; then cp "{root}/stand-in/junit.xml" "$2/vitest.xml"; fi
if [ -f "{root}/stand-in/failing" ]; then
  case "$call" in *"$(cat "{root}/stand-in/failing")"*) exit 3 ;; esac
fi
"#
        ),
    );
    workspace
}

fn skew(workspace: &Workspace, part: &str, job: &str) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/release/skew.sh");
    let stand_ins = workspace.path().join("stand-in");
    let output = workspace
        .command(script)
        .args([part, job])
        .env("PATH", format!("{}:/usr/bin:/bin", stand_ins.display()))
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        calls: std::fs::read_to_string(stand_ins.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

const COLLIE_STEP: &str = "journeys --product collie --surface cli --engine target/release-suite/skew/goodfetch-collie --junit target/release-suite/skew/junit/collie-cli released=none engines=,";
const FOXHOUND_STEP: &str = "journeys --product foxhound --surface cli --engine target/release-suite/skew/goodfetch-foxhound --junit target/release-suite/skew/junit/foxhound-cli+foxhound-mcp released=none engines=,";

#[rstest]
fn a_job_the_file_does_not_name_is_refused(#[values("build", "run")] part: &str) {
    let workspace = workspace();

    let run = skew(&workspace, part, "1.1");

    assert_eq!(run.code, Some(2), "{}", run.stderr);
    assert_eq!(
        run.stderr,
        "error: ci/release/skew-jobs.json has no job 1.1\n"
    );
    assert!(run.calls.is_empty(), "{:?}", run.calls);
}

#[test]
fn build_keeps_a_copy_of_each_products_executable() {
    let workspace = workspace();

    let run = skew(&workspace, "build", "current");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    for product in ["collie", "foxhound"] {
        let kept = format!("target/release-suite/skew/goodfetch-{product}");
        assert_eq!(workspace.read_text(&kept), product);
    }
    assert!(run.calls.is_empty(), "{:?}", run.calls);
}

#[test]
fn build_fetches_a_released_minors_dependencies_in_its_own_tree_then_removes_it() {
    let workspace = workspace();

    let run = skew(&workspace, "build", "1.2");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls, ["fetch released=1.2.3 engines=,"]);
    let worktrees = workspace.git(&["worktree", "list", "--porcelain"]);
    assert_eq!(worktrees.matches("worktree ").count(), 1, "{worktrees}");
}

/// The current build against itself, which the matrix runs while no minor is
/// released.
#[test]
fn each_step_runs_its_journeys_on_its_products_executable() {
    let workspace = workspace();

    let run = skew(&workspace, "run", "current");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls, [COLLIE_STEP, FOXHOUND_STEP]);
    for checks in ["collie-cli", "foxhound-cli+foxhound-mcp"] {
        let report = format!("target/junit/vitest-{checks}.xml");
        assert_eq!(workspace.read_text(&report), ONE_TEST);
    }
}

#[test]
fn a_stdio_step_of_a_released_minor_names_both_engines_and_its_tag() {
    let workspace = workspace();

    let run = skew(&workspace, "run", "1.2");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "journeys --tag v1.2.3 --product collie --surface sdk-typescript --exclude J-001@sdk-typescript --engine target/release-suite/skew/goodfetch-collie --junit target/release-suite/skew/junit/collie-sdk-typescript-stdio released=none engines=target/release-suite/skew/goodfetch-collie,target/release-suite/skew/goodfetch-foxhound"
        ]
    );
}

#[test]
fn a_failed_step_fails_the_job_and_stops_none_after_it() {
    let workspace = workspace();
    workspace.write("stand-in/failing", b"--product collie");

    let run = skew(&workspace, "run", "current");

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert_eq!(run.calls, [COLLIE_STEP, FOXHOUND_STEP]);
    assert_eq!(run.stderr, "Skew current, collie-cli: failed\n");
}

#[rstest]
#[case::no_junit_file(None)]
#[case::an_empty_one(Some("<testsuites></testsuites>"))]
#[case::only_a_skipped_test(Some(
    r#"<testsuites><testsuite name="s"><testcase name="t"><skipped/></testcase></testsuite></testsuites>"#
))]
fn a_step_that_runs_no_test_fails_the_job(#[case] junit: Option<&str>) {
    let workspace = workspace();
    match junit {
        Some(junit) => workspace.write("stand-in/junit.xml", junit.as_bytes()),
        None => workspace.remove("stand-in/junit.xml"),
    }

    let run = skew(&workspace, "run", "current");

    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert_eq!(
        run.stderr,
        "error: skew current, collie-cli: no test ran\nerror: skew current, foxhound-cli+foxhound-mcp: no test ran\n"
    );
}

#[test]
fn an_earlier_runs_junit_file_is_not_this_runs() {
    let workspace = workspace();
    skew(&workspace, "run", "current");
    workspace.remove("stand-in/junit.xml");

    let run = skew(&workspace, "run", "current");

    assert_eq!(run.code, Some(1), "{}", run.stdout);
}

#[test]
fn run_names_each_awaiting_check_with_its_ticket() {
    let workspace = workspace();

    let run = skew(&workspace, "run", "current");

    let first = run.stdout.lines().next();
    assert_eq!(first, Some("Not run, awaiting #983: two-majors"));
}
