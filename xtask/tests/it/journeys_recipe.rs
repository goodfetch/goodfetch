//! `just journeys --junit <folder>`, through which a skew job (ADR 0066) reads
//! what a run's runners reported: their JUnit files, from the tree the tests
//! ran in. A stand-in `cargo` plays xtask and nextest, whose JUnit file holds
//! the tree's `version`; nextest itself runs no test.
//! Repository tooling: the module declares no Feature.
#![cfg(unix)]

use std::path::Path;

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, write_executable};

/// A workspace at `version` 2 whose tag `v1` is at 1, with the repository's
/// justfile and scripts, and a nextest that fails when `failing` is there.
fn workspace() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    workspace.write(
        "justfile",
        &std::fs::read(repository.join("justfile")).unwrap(),
    );
    std::os::unix::fs::symlink(repository.join("ci"), workspace.path().join("ci")).unwrap();
    workspace.write(
        ".gitignore",
        b"/target/\n/stand-in/\n/failing\n/reports/\n/engine\n",
    );
    workspace.write("version", b"1\n");
    workspace.commit_all();
    workspace.git(&["tag", "v1"]);
    workspace.write("version", b"2\n");
    workspace.commit_all();
    let stand_in = workspace.path().join("stand-in");
    std::fs::create_dir_all(&stand_in).unwrap();
    let cargo = format!(
        r#"#!/bin/sh
case "$*" in
  *" journeys select "*) printf 'nextest\t-E\nnextest\ttest(chosen)\n' ;;
  *" quarantine exclude"*) echo '{{"nextest": [], "vitest": [], "playwright": [], "pytest": []}}' ;;
  "nextest run "*)
    mkdir -p target/nextest/ci
    cp version target/nextest/ci/junit.xml
    if [ -f "{failing}" ]; then exit 100; fi ;;
esac
"#,
        failing = workspace.path().join("failing").display()
    );
    write_executable(&stand_in.join("cargo"), &cargo);
    for program in ["pnpm", "uv"] {
        write_executable(&stand_in.join(program), "#!/bin/sh\nexit 0\n");
    }
    write_executable(&workspace.path().join("engine"), "#!/bin/sh\nexit 0\n");
    workspace
}

fn journeys(workspace: &Workspace, arguments: &[&str]) -> Outcome {
    let output = workspace
        .just(&workspace.path().join("stand-in"))
        .arg("journeys")
        .args(arguments)
        .env("HOME", workspace.path())
        .output()
        .unwrap();
    Outcome {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

#[rstest]
#[case::this_tree(&[], "2\n")]
#[case::a_tags_tree(&["--tag", "v1", "--engine", "engine"], "1\n")]
fn the_folder_gets_each_runners_junit_file_from_the_tree_its_tests_ran_in(
    #[case] flags: &[&str],
    #[case] version: &str,
) {
    let workspace = workspace();

    let outcome = journeys(&workspace, &[&["--junit", "reports"], flags].concat());

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(workspace.read_text("reports/nextest.xml"), version);
}

#[test]
fn a_failed_run_still_gives_its_junit_files() {
    let workspace = workspace();
    workspace.write("failing", b"");

    let outcome = journeys(&workspace, &["--junit=reports"]);

    assert_eq!(outcome.code, 100, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(workspace.read_text("reports/nextest.xml"), "2\n");
}

/// A job that uploads target/junit/ beside the folder counts no test twice.
#[test]
fn the_runs_own_junit_files_move_to_the_folder() {
    let workspace = workspace();

    let outcome = journeys(&workspace, &["--junit", "reports"]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(workspace.read("target/nextest/ci/junit.xml"), None);
}

/// A runner this run does not start, such as Vitest here, gives no file of an
/// earlier run's.
#[test]
fn an_earlier_runs_junit_file_is_not_this_runs() {
    let workspace = workspace();
    workspace.write("target/junit/vitest.xml", b"earlier\n");

    let outcome = journeys(&workspace, &["--junit", "reports"]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(workspace.read("reports/vitest.xml"), None);
}
