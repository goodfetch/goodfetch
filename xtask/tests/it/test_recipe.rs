//! The `test` recipe's tool build (ADRs 0062 and 0094): every suite but
//! Quick builds the commit's executables through `xtask test-tools build`
//! before a runner starts and names the manifest in `GOODFETCH_TEST_TOOLS`.
//! The Quick suite builds none and leaves the variable unset: its tests run
//! in process (nextest's `quick` profile, Vitest's unit project, pytest
//! outside `tests/integration`) and none runs a tool, while the build of the
//! seven tools rebuilt every crate above the touched one and linked about
//! 1.3 GB of executables on each run, most of a `just test quick`'s time
//! (the implementation audit of 2026-11-25, measure 1). A quick test that
//! asks for one fails at once, naming `just test` (`tools()`). The recipe
//! runs here on a fixture workspace whose justfile imports the repository's,
//! so that the recipe's own `just _test-tools` finds it, with a stand-in
//! `cargo` that records each call and the variable's state when the test
//! runner starts.
#![cfg(unix)]

use std::path::Path;

use crate::workspace::{Workspace, write_executable};

/// The recipe's helper scripts, copied from the repository.
const SCRIPTS: [&str; 5] = [
    "ci/order-seed.sh",
    "ci/order-arguments.sh",
    "ci/nextest-skips.sh",
    "ci/os-services.sh",
    "ci/quick-targets.sh",
];

struct Run {
    code: Option<i32>,
    /// Each `cargo` call's arguments, in order.
    calls: Vec<String>,
    /// `GOODFETCH_TEST_TOOLS` as the test runner saw it: the manifest's
    /// path, or `unset`; `None` when no runner started.
    tools_variable: Option<String>,
    output: String,
}

impl Run {
    fn builds_the_tools(&self) -> bool {
        self.calls
            .iter()
            .any(|call| call.contains("-- test-tools build "))
    }

    fn starts_the_test_runner(&self) -> bool {
        self.calls
            .iter()
            .any(|call| call.starts_with("insta test ") || call.starts_with("nextest run "))
    }
}

/// Runs `just test <suite> sample` in a fixture workspace, with a stale
/// manifest path in `GOODFETCH_TEST_TOOLS`, as a shell that ran `just test`
/// earlier may hold one. The stand-in `cargo` answers `metadata` with one
/// library crate, `quarantine exclude` with no exclusion and `os-services
/// check` with no skipped service, writes the manifest a `test-tools build`
/// asks for, records the variable when the test runner starts, and fails on
/// any later command, which ends the recipe after what this module checks.
fn test_recipe(suite: &str) -> Run {
    let workspace = Workspace::from_fixture("empty");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    for script in SCRIPTS {
        let copy = workspace.path().join(script);
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::copy(repository.join(script), copy).unwrap();
    }
    std::fs::write(
        workspace.path().join("justfile"),
        format!("import '{}'\n", repository.join("justfile").display()),
    )
    .unwrap();
    // `ci/order-seed.sh` reads the committed tree.
    workspace.git(&["init", "--quiet"]);
    workspace.git(&["add", "--all"]);
    workspace.git(&["commit", "--quiet", "--no-verify", "--message", "fixture"]);
    let stand_in = workspace.path().join("stand-in");
    std::fs::create_dir_all(&stand_in).unwrap();
    let calls = stand_in.join("calls");
    let variable = stand_in.join("tools-variable");
    write_executable(
        &stand_in.join("cargo"),
        &format!(
            r#"#!/bin/sh
echo "$*" >>"{calls}"
case "$1" in
  metadata) echo '{{"packages":[{{"name":"sample","targets":[{{"kind":["lib"],"name":"sample"}}]}}]}}' ;;
  run)
    while [ "$1" != -- ]; do shift; done
    shift
    case "$1" in
      quarantine) echo '{{"nextest":[],"vitest":[],"playwright":[],"pytest":[]}}' ;;
      test-tools)
        while [ "$1" != --out ]; do shift; done
        mkdir -p "$(dirname "$2")" && echo '{{}}' >"$2"
        ;;
      *) : ;;
    esac
    ;;
  insta | nextest) echo "${{GOODFETCH_TEST_TOOLS-unset}}" >"{variable}" ;;
  *) exit 2 ;;
esac
"#,
            calls = calls.display(),
            variable = variable.display()
        ),
    );
    let output = workspace
        .just(&stand_in)
        .args(["test", suite, "sample"])
        .env("GOODFETCH_TEST_TOOLS", "/stale/test-tools.json")
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        calls: std::fs::read_to_string(&calls)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
        tools_variable: std::fs::read_to_string(&variable)
            .ok()
            .map(|text| text.trim().to_owned()),
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

#[test]
fn the_quick_suite_builds_no_test_tool_and_runs_its_tests_with_the_manifest_unset() {
    let run = test_recipe("quick");

    assert_eq!(run.code, Some(0), "{}", run.output);
    assert!(
        !run.builds_the_tools(),
        "the Quick suite built the test tools:\n{}",
        run.calls.join("\n")
    );
    assert!(run.starts_the_test_runner(), "{}", run.calls.join("\n"));
    assert_eq!(
        run.tools_variable.as_deref(),
        Some("unset"),
        "{}",
        run.output
    );
}

#[test]
fn every_other_suite_builds_the_tools_before_its_runners_start_and_names_the_manifest() {
    let run = test_recipe("pre-merge");

    let built = run
        .calls
        .iter()
        .position(|call| call.contains("-- test-tools build "));
    let started = run
        .calls
        .iter()
        .position(|call| call.starts_with("insta test ") || call.starts_with("nextest run "));
    assert!(
        matches!((built, started), (Some(built), Some(started)) if built < started),
        "tools built at {built:?}, runner started at {started:?}:\n{}\n{}",
        run.calls.join("\n"),
        run.output
    );
    let manifest = run.tools_variable.as_deref().unwrap_or("unset");
    assert!(
        manifest.ends_with("/target/test-tools.json"),
        "{manifest}\n{}",
        run.output
    );
}
