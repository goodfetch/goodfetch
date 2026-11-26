//! `just older-engine <product>`, through which a skew job (ADR 0066) runs
//! one Product's older-engine check: Foxhound's `older_engine` tests of that
//! Product, with `--engine` as the older executable they read from
//! `GOODFETCH_TEST_OLDER_ENGINE`. A stand-in `cargo` plays xtask and nextest:
//! nextest records its arguments and that variable as its JUnit file, and
//! runs no test.
//! Repository tooling: the module declares no Feature.
#![cfg(unix)]

use std::path::Path;

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, write_executable};

/// A workspace with the repository's justfile and scripts, a stand-in
/// `cargo`, and an `engine` that is an executable and a `plain` file that is
/// not. nextest fails when `failing` is there.
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
        b"/target/\n/stand-in/\n/failing\n/reports/\n/engine\n/plain\n",
    );
    workspace.commit_all();
    let stand_in = workspace.path().join("stand-in");
    std::fs::create_dir_all(&stand_in).unwrap();
    let cargo = format!(
        r#"#!/bin/sh
case "$*" in
  *" quarantine exclude"*) echo '{{"nextest": [], "vitest": [], "playwright": [], "pytest": []}}' ;;
  "nextest run "*)
    mkdir -p target/nextest/ci
    printf '%s\nolder=%s\n' "$*" "${{GOODFETCH_TEST_OLDER_ENGINE-}}" > target/nextest/ci/junit.xml
    if [ -f "{failing}" ]; then exit 100; fi ;;
esac
"#,
        failing = workspace.path().join("failing").display()
    );
    write_executable(&stand_in.join("cargo"), &cargo);
    write_executable(&workspace.path().join("engine"), "#!/bin/sh\nexit 0\n");
    workspace.write("plain", b"");
    workspace
}

fn older_engine(workspace: &Workspace, arguments: &[&str]) -> Outcome {
    let output = workspace
        .just(&workspace.path().join("stand-in"))
        .arg("older-engine")
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

/// nextest's run, as the stand-in recorded it: its arguments, then the older
/// executable it was given.
fn recorded(workspace: &Workspace) -> (String, String) {
    let report = workspace.read_text("reports/nextest.xml");
    let (arguments, older) = report.split_once("\nolder=").unwrap();
    (arguments.to_owned(), older.trim_end().to_owned())
}

/// Only Foxhound's crate runs, and only the tests of the Product named.
#[rstest]
#[case::collie("collie")]
#[case::foxhound("foxhound")]
fn it_runs_the_products_older_engine_tests_alone(#[case] product: &str) {
    let workspace = workspace();

    let outcome = older_engine(&workspace, &[product, "--junit", "reports"]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    let (arguments, older) = recorded(&workspace);
    let filter = format!("-E test(/^older_engine::/) & test(/older_{product}/)");
    assert!(
        arguments.starts_with("nextest run -p goodfetch-foxhound "),
        "{arguments}"
    );
    assert!(arguments.contains(&filter), "{arguments}");
    assert_eq!(older, "");
}

#[test]
fn the_engine_given_is_the_older_executable_by_its_full_path() {
    let workspace = workspace();

    let outcome = older_engine(
        &workspace,
        &["collie", "--engine", "engine", "--junit=reports"],
    );

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    let engine = std::fs::canonicalize(workspace.path().join("engine")).unwrap();
    assert_eq!(recorded(&workspace).1, engine.display().to_string());
}

#[test]
fn a_failed_run_still_gives_its_junit_file() {
    let workspace = workspace();
    workspace.write("failing", b"");

    let outcome = older_engine(&workspace, &["foxhound", "--junit", "reports"]);

    assert_eq!(outcome.code, 100, "{}{}", outcome.stdout, outcome.stderr);
    assert!(recorded(&workspace).0.starts_with("nextest run "));
    assert_eq!(workspace.read("target/nextest/ci/junit.xml"), None);
}

#[rstest]
#[case::another_product(&["sdk"], "error: the Product is collie or foxhound, not sdk\n")]
#[case::an_engine_that_is_no_executable(&["collie", "--engine", "plain"], "error: plain is not an executable\n")]
#[case::an_unknown_argument(&["collie", "--tag", "v1"], "error: unknown argument --tag\n")]
fn a_bad_call_runs_nothing(#[case] arguments: &[&str], #[case] error: &str) {
    let workspace = workspace();

    let outcome = older_engine(&workspace, arguments);

    assert_eq!(outcome.code, 2, "{}{}", outcome.stdout, outcome.stderr);
    assert!(outcome.stderr.starts_with(error), "{}", outcome.stderr);
    assert_eq!(workspace.read("target/nextest/ci/junit.xml"), None);
}
