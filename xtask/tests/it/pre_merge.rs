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

/// The OS-service setup the suite runs first on a Linux host, which runs the
/// tests only Linux decides itself (`ci/linux.sh`); elsewhere `just linux`'s
/// container sets the services up for them.
const HOST_SETUP: &[&str] = if cfg!(target_os = "linux") {
    &["_os-services"]
} else {
    &[]
};

/// A stand-in `just` that appends its arguments to `calls`, and them with
/// the OS services' variables it was given to `variables`, and fails with
/// status 3 when they are `failing`'s content.
fn stand_in_just(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let script = format!(
        r#"#!/bin/sh
echo "$*" >>"{calls}"
echo "$*|${{DBUS_SESSION_BUS_ADDRESS:-}}|${{GOODFETCH_OS_SERVICES:-}}|${{GOODFETCH_TEST_DNS_NAMESPACE:-}}" >>"{variables}"
if [ -f "{failing}" ] && [ "$*" = "$(cat "{failing}")" ]; then
  exit 3
fi
"#,
        calls = folder.join("calls").display(),
        variables = folder.join("variables").display(),
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
    /// Each call as `<arguments>|<session bus>|<GOODFETCH_OS_SERVICES>|<GOODFETCH_TEST_DNS_NAMESPACE>`.
    variables: Vec<String>,
    stderr: String,
}

/// The session bus that the stand-in setup's environment file names.
const BUS: &str = "unix:path=/stand-in/bus";

/// Runs the repository's `ci/pre-merge.sh` in the workspace, with the
/// stand-in first on `PATH`.
fn pre_merge(workspace: &Workspace, stand_in: &Path, args: &[&str]) -> Run {
    pre_merge_with_env(workspace, stand_in, args, &[])
}

/// Runs `ci/pre-merge.sh` with these variables added, and the environment
/// file `ci/os-services.sh setup` writes in the workspace's temporary folder.
fn pre_merge_with_env(
    workspace: &Workspace,
    stand_in: &Path,
    args: &[&str],
    variables: &[(&str, &str)],
) -> Run {
    workspace.write(
        "runner-temp/goodfetch-os-services/env",
        format!("DBUS_SESSION_BUS_ADDRESS={BUS}\n").as_bytes(),
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/pre-merge.sh");
    let path = format!("{}:/usr/bin:/bin", stand_in.display());
    let output = workspace
        .command(script)
        .args(args)
        .env("PATH", path)
        .env("RUNNER_TEMP", workspace.path().join("runner-temp"))
        .envs(variables.iter().copied())
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
        variables: read("variables"),
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
    let mut expected: Vec<String> = HOST_SETUP
        .iter()
        .chain(&STEPS)
        .map(|step| (*step).to_owned())
        .collect();
    expected.sort();
    assert_eq!(ran, expected);
    let host: Vec<&String> = run
        .calls
        .iter()
        .filter(|call| !BESIDE.contains(&call.as_str()) && *call != "_skips-report")
        .collect();
    assert_eq!(
        host,
        HOST_SETUP
            .iter()
            .chain(&STEPS[..STEPS.len() - BESIDE.len()])
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
    let mut expected: Vec<String> = HOST_SETUP
        .iter()
        .chain(&STEPS)
        .map(|step| step.replace(" all", " "))
        .chain(["_skips-report".to_owned()])
        .collect();
    expected.sort();
    assert_eq!(ran, expected);
}

/// The quick form's steps, in order: those that judge every change, never a
/// test of the Pre-merge suite or a step beside the host's.
const QUICK_STEPS: [&str; 8] = [
    "dco",
    "commit-messages",
    "lint",
    "audit",
    "commit-leaks",
    "_suite quick all",
    "journeys-tasks --check",
    "notices",
];

#[test]
fn the_quick_form_runs_the_checks_every_change_needs_and_nothing_beside() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &["quick"]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut expected: Vec<String> = HOST_SETUP
        .iter()
        .chain(&QUICK_STEPS)
        .map(|step| (*step).to_owned())
        .collect();
    expected.push("_skips-report".to_owned());
    assert_eq!(run.calls, expected);
}

#[test]
fn an_unknown_argument_is_refused_before_any_step() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &["most"]);

    assert_eq!(run.code, Some(2), "{}", run.stderr);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
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

/// On a Linux host every step after the setup meets the services it
/// started, and runs where a missing one fails instead of skipping its
/// tests (`ci/os-services.sh skipped`), with the DNS namespace's way
/// declared, so a silent fallback fails too.
#[cfg(target_os = "linux")]
#[rstest]
#[case::all("all")]
#[case::quick("quick")]
fn on_linux_every_step_runs_with_the_os_services_set_up(#[case] form: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &[form]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls.first().map(String::as_str), Some("_os-services"));
    let steps: Vec<(&str, &str)> = run
        .variables
        .iter()
        .filter_map(|line| line.split_once('|'))
        .filter(|(call, _)| !["_os-services", "_skips-report"].contains(call))
        .collect();
    assert!(steps.len() > 1, "{:?}", run.variables);
    for (call, variables) in steps {
        assert_eq!(variables, format!("{BUS}|required|sudo"), "{call}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn on_linux_a_declared_dns_namespace_way_is_kept() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge_with_env(
        &workspace,
        &stand_in,
        &["quick"],
        &[("GOODFETCH_TEST_DNS_NAMESPACE", "user_namespace")],
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        run.variables
            .contains(&format!("lint|{BUS}|required|user_namespace")),
        "{:?}",
        run.variables
    );
}

#[cfg(target_os = "linux")]
#[test]
fn on_linux_a_failed_os_service_setup_stops_the_suite_before_any_step() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), "_os-services").unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert_eq!(run.calls, ["_os-services", "_skips-report"]);
}
