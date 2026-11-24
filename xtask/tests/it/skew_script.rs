//! A skew job of the Release suite (`ci/release/skew.sh`, ADR 0066) on a
//! sample `ci/release/skew-jobs.json`: `build` keeps both Products' current
//! executables, fetches a minor's tag and keeps its older engines, and `run`
//! runs each of the job's steps through `just journeys` or `just
//! older-engine` and fails on one that ran no test. A stand-in `cargo` plays
//! `xtask test-tools` and a stand-in `just` records each call, writes the
//! JUnit file a run leaves and names the artifact set's `versions.json`.
//! Repository tooling: the module declares no Feature.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use rstest::rstest;

use crate::workspace::{Workspace, write_executable};

const JOBS: &str = r#"{
  "jobs": [
    {"name": "current", "version": null, "tag": null, "minutes": 35, "steps": [
      {"checks": ["collie-cli"], "executable": "goodfetch-collie", "stdio": false, "journeys": ["--product", "collie", "--surface", "cli"]},
      {"checks": ["foxhound-cli", "foxhound-mcp"], "executable": "goodfetch-foxhound", "stdio": false, "journeys": ["--product", "foxhound", "--surface", "cli"]},
      {"checks": ["collie-older-engine"], "older_engine": "collie"}
    ]},
    {"name": "1.2", "version": "1.2.3", "tag": "v1.2.3", "minutes": 30, "steps": [
      {"checks": ["collie-sdk-typescript-stdio"], "executable": "goodfetch-collie", "stdio": true, "journeys": ["--product", "collie", "--surface", "sdk-typescript", "--exclude", "J-001@sdk-typescript"]},
      {"checks": ["foxhound-older-engine"], "older_engine": "foxhound"}
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
    release_archive(&workspace);
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
if [ "$1" = artifacts ]; then echo "{root}/stand-in/set/versions.json"; fi
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

/// Where the artifact set keeps 1.2.3's Foxhound executable for Linux x64,
/// from the workspace's folder.
const RELEASED_FOXHOUND: &str = "stand-in/set/1.2.3/foxhound/executables/x86_64-unknown-linux-gnu";

/// 1.2.3's Foxhound as the set keeps it: an archive holding the executable
/// in a folder of its own, with its attestation bundle beside it.
fn release_archive(workspace: &Workspace) {
    workspace.write(
        "stand-in/archive/goodfetch-foxhound-1.2.3/goodfetch-foxhound",
        b"older foxhound",
    );
    workspace.write(
        &format!("{RELEASED_FOXHOUND}/x.tar.gz.sigstore.json"),
        b"{}",
    );
    release_archive_of(workspace);
}

/// Packs `stand-in/archive/` as the set's archive of 1.2.3's Foxhound.
fn release_archive_of(workspace: &Workspace) {
    let packed = workspace
        .command("tar")
        .env("PATH", "/usr/bin:/bin")
        .args(["-czf", &format!("{RELEASED_FOXHOUND}/x.tar.gz")])
        .args(["-C", "stand-in/archive", "goodfetch-foxhound-1.2.3"])
        .status()
        .unwrap();
    assert!(packed.success());
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
const OLDER_COLLIE_STEP: &str = "older-engine collie --junit target/release-suite/skew/junit/collie-older-engine released=none engines=,";

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
    assert_eq!(
        run.calls,
        [
            "fetch released=1.2.3 engines=,",
            "artifacts released=none engines=,"
        ]
    );
    let worktrees = workspace.git(&["worktree", "list", "--porcelain"]);
    assert_eq!(worktrees.matches("worktree ").count(), 1, "{worktrees}");
}

/// An older-engine step's executable, from the set the current tree's
/// release list names, whether published in an archive or as itself.
#[rstest]
#[case::in_an_archive(None)]
#[case::as_itself(Some("goodfetch-foxhound-x86_64-unknown-linux-gnu"))]
fn build_keeps_a_released_minors_older_engine_from_the_artifact_set(#[case] bare: Option<&str>) {
    let workspace = workspace();
    if let Some(name) = bare {
        workspace.remove_folder(RELEASED_FOXHOUND);
        workspace.write(&format!("{RELEASED_FOXHOUND}/{name}"), b"older foxhound");
    }

    let run = skew(&workspace, "build", "1.2");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let kept = "target/release-suite/skew/older/goodfetch-foxhound";
    assert_eq!(workspace.read_text(kept), "older foxhound");
    let mode = std::fs::metadata(workspace.path().join(kept))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
}

/// Never a skip: a set without the minor's executable, or an archive
/// without it, fails the job.
#[rstest]
#[case::no_executable(
    "goodfetch-foxhound",
    "error: the artifact set holds no 1.2.3 executable of foxhound in "
)]
#[case::an_archive_without_it("README", "x.tar.gz holds no goodfetch-foxhound\n")]
fn build_fails_on_a_set_without_the_older_engine(#[case] archived: &str, #[case] error: &str) {
    let workspace = workspace();
    workspace.remove(&format!("{RELEASED_FOXHOUND}/x.tar.gz"));
    if archived != "goodfetch-foxhound" {
        workspace.remove_folder("stand-in/archive/goodfetch-foxhound-1.2.3");
        workspace.write(
            &format!("stand-in/archive/goodfetch-foxhound-1.2.3/{archived}"),
            b"",
        );
        release_archive_of(&workspace);
    }

    let run = skew(&workspace, "build", "1.2");

    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains(error), "{}", run.stderr);
    assert!(
        !workspace
            .path()
            .join("target/release-suite/skew/older/goodfetch-foxhound")
            .exists()
    );
}

/// The current build against itself, which the matrix runs while no minor is
/// released: the older-engine check takes the commit's own executable.
#[test]
fn each_step_runs_its_tests_on_its_products_executable() {
    let workspace = workspace();

    let run = skew(&workspace, "run", "current");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls, [COLLIE_STEP, FOXHOUND_STEP, OLDER_COLLIE_STEP]);
    for checks in [
        "collie-cli",
        "foxhound-cli+foxhound-mcp",
        "collie-older-engine",
    ] {
        let report = format!("target/junit/vitest-{checks}.xml");
        assert_eq!(workspace.read_text(&report), ONE_TEST);
    }
}

#[test]
fn a_released_minors_steps_take_its_tag_and_its_older_engine() {
    let workspace = workspace();

    let run = skew(&workspace, "run", "1.2");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "journeys --tag v1.2.3 --product collie --surface sdk-typescript --exclude J-001@sdk-typescript --engine target/release-suite/skew/goodfetch-collie --junit target/release-suite/skew/junit/collie-sdk-typescript-stdio released=none engines=target/release-suite/skew/goodfetch-collie,target/release-suite/skew/goodfetch-foxhound",
            "older-engine foxhound --engine target/release-suite/skew/older/goodfetch-foxhound --junit target/release-suite/skew/junit/foxhound-older-engine released=none engines=,"
        ]
    );
}

#[test]
fn a_failed_step_fails_the_job_and_stops_none_after_it() {
    let workspace = workspace();
    workspace.write("stand-in/failing", b"--product collie");

    let run = skew(&workspace, "run", "current");

    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert_eq!(run.calls, [COLLIE_STEP, FOXHOUND_STEP, OLDER_COLLIE_STEP]);
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
        "error: skew current, collie-cli: no test ran\nerror: skew current, foxhound-cli+foxhound-mcp: no test ran\nerror: skew current, collie-older-engine: no test ran\n"
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
