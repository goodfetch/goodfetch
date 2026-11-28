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

use rstest::rstest;

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
        self.test_tools_build_call().is_some()
    }

    fn test_tools_build_call(&self) -> Option<&str> {
        self.calls
            .iter()
            .find(|call| call.contains("-- test-tools build "))
            .map(String::as_str)
    }

    fn starts_the_test_runner(&self) -> bool {
        self.calls
            .iter()
            .any(|call| call.starts_with("insta test ") || call.starts_with("nextest run "))
    }
}

/// Runs `just test <suite> <packages...>` in a fixture workspace, with a stale
/// manifest path in `GOODFETCH_TEST_TOOLS` and extra `env` overrides, as a
/// shell that ran `just test` earlier may hold one. The stand-in `cargo`
/// answers `metadata` with one library crate, `quarantine exclude` with no
/// exclusion and `os-services check` with no skipped service, writes the
/// manifest a `test-tools build` asks for, records the variable when the test
/// runner starts, and fails on any later command, which ends the recipe after
/// what this module checks.
fn test_recipe_with(suite: &str, packages: &[&str], env: &[(&str, &str)]) -> Run {
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
    for manifest in [
        "crates/goodfetch-engine/Cargo.toml",
        "crates/goodfetch-collie/Cargo.toml",
        "crates/goodfetch-page/Cargo.toml",
        "sdks/typescript/collie/package.json",
        "sdks/typescript/collie-client/package.json",
        "sdks/typescript/smoke/package.json",
        "sdks/typescript/foxhound/package.json",
        "sdks/python/collie/pyproject.toml",
        "sdks/python/collie/tests/integration/test_five_word_port.py",
        "sdks/python/collie-client/pyproject.toml",
        "sdks/python/foxhound/pyproject.toml",
        "bench/package.json",
        "adapters/foxhound/custom/package.json",
        "crates/foxhound/goodfetch-custom-future/Cargo.toml",
    ] {
        let path = workspace.path().join(manifest);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{}\n").unwrap();
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
    for runner in ["pnpm", "uv"] {
        write_executable(&stand_in.join(runner), "#!/bin/sh\nexit 0\n");
    }
    let mut command = workspace.just(&stand_in);
    command
        .arg("test")
        .arg(suite)
        .args(packages)
        .env("GOODFETCH_TEST_TOOLS", "/stale/test-tools.json")
        .env("CARGO_NET_OFFLINE", "true");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().unwrap();
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

fn test_recipe(suite: &str) -> Run {
    test_recipe_with(suite, &["sample"], &[])
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

#[rstest]
#[case::engine(&["goodfetch-engine"])]
#[case::collie(&["goodfetch-collie"])]
#[case::page_and_engine(&["goodfetch-page", "goodfetch-engine"])]
#[case::proving_ground(&["goodfetch-proving-ground"])]
#[case::ts_collie(&["sdks/typescript/collie"])]
#[case::ts_collie_normalized(&["./sdks/typescript/collie/"])]
#[case::ts_collie_client(&["sdks/typescript/collie-client"])]
#[case::ts_smoke(&["sdks/typescript/smoke"])]
#[case::py_collie_client(&["sdks/python/collie-client"])]
fn collie_only_package_selection_builds_minimal_tools_and_skips_foxhound(
    #[case] packages: &[&str],
) {
    let run = test_recipe_with("pre-merge", packages, &[]);

    let build = run
        .test_tools_build_call()
        .unwrap_or_else(|| panic!("no test-tools build call:\n{}", run.output));
    assert!(
        build.contains(
            "--tool collie --tool proving_ground --tool test_env --tool scripted_engine --tool scripted_plugin"
        ),
        "expected minimal Collie tools, got: {build}"
    );
    assert!(
        !build.contains("foxhound"),
        "Collie-only selection must not build foxhound: {build}"
    );
}

#[rstest]
#[case::workspace(&[])]
#[case::foxhound(&["goodfetch-foxhound"])]
#[case::crawl(&["goodfetch-crawl"])]
#[case::fetch_tier(&["goodfetch-fetch-tier"])]
#[case::sink_wildcard(&["goodfetch-sink-s3"])]
#[case::foxhound_dir(&["crates/foxhound/goodfetch-frontier"])]
#[case::foxhound_crate_by_dir(&["goodfetch-custom-future"])]
#[case::ts_foxhound(&["sdks/typescript/foxhound"])]
#[case::ts_foxhound_dot_slash(&["./sdks/typescript/foxhound"])]
#[case::py_collie(&["sdks/python/collie"])]
#[case::py_collie_five_word_port(&["sdks/python/collie/tests/integration/test_five_word_port.py"])]
#[case::py_foxhound(&["sdks/python/foxhound"])]
#[case::bench(&["bench"])]
#[case::adapter_foxhound(&["adapters/foxhound/custom"])]
#[case::conformance(&["goodfetch-conformance"])]
#[case::stub_gateway(&["goodfetch-stub-gateway"])]
#[case::test_support(&["goodfetch-test-support"])]
#[case::xtask(&["xtask"])]
#[case::shuffled_all(&["--shuffled-all", "goodfetch-engine"])]
fn foxhound_cross_product_or_workspace_selection_builds_all_tools(#[case] packages: &[&str]) {
    let run = test_recipe_with("pre-merge", packages, &[]);

    let build = run
        .test_tools_build_call()
        .unwrap_or_else(|| panic!("no test-tools build call:\n{}", run.output));
    assert!(
        !build.contains("--tool "),
        "expected all tools (no --tool flag), got: {build}"
    );
}

#[test]
fn engine_override_builds_all_tools() {
    let run = test_recipe_with(
        "pre-merge",
        &["goodfetch-engine"],
        &[("GOODFETCH_TEST_ENGINE", "/tmp/custom-engine")],
    );

    let build = run
        .test_tools_build_call()
        .unwrap_or_else(|| panic!("no test-tools build call:\n{}", run.output));
    assert!(!build.contains("--tool "), "got: {build}");
    assert!(
        build.contains("--engine /tmp/custom-engine"),
        "got: {build}"
    );
}

#[test]
fn test_selection_builds_all_tools() {
    let selection_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(selection_file.path(), "nextest\ttest(foo)\n").unwrap();

    let run = test_recipe_with(
        "pre-merge",
        &["goodfetch-engine"],
        &[(
            "GOODFETCH_TEST_SELECTION",
            selection_file.path().to_str().unwrap(),
        )],
    );

    let build = run
        .test_tools_build_call()
        .unwrap_or_else(|| panic!("no test-tools build call:\n{}", run.output));
    assert!(!build.contains("--tool "), "got: {build}");
}
