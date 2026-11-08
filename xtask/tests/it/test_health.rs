//! The test-health runs the gate host's cron starts (`just test-health`,
//! `ci/test-health/scheduled.sh`; TESTING.md's After merge): a run skips while
//! `just merge`'s gate runs, and says so in its results line, so the two never
//! share the host's cores. `just` is a stand-in that records each call. The
//! local gate is repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::affected::repository;
use crate::workspace::Workspace;

/// A process ID above every system's limit, so no process has it.
const NO_PROCESS: &str = "2147483646";

/// A stand-in `just` that appends its arguments to `calls`, and them with the
/// OS services' variables it was given to `variables`.
fn stand_in_just(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let just = folder.join("just");
    std::fs::write(
        &just,
        format!(
            "#!/bin/sh\necho \"$*\" >>\"{calls}\"\n\
             echo \"$1|${{XDG_RUNTIME_DIR:-}}|${{GOODFETCH_OS_SERVICES:-}}\" >>\"{variables}\"\n",
            calls = folder.join("calls").display(),
            variables = folder.join("variables").display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&just, std::fs::Permissions::from_mode(0o755)).unwrap();
    folder
}

struct Run {
    code: Option<i32>,
    calls: Vec<String>,
    results: String,
    log: String,
}

/// The fixture as a repository whose `origin` is itself, so `origin/main` is
/// its `main`.
fn origin_repository() -> Workspace {
    let workspace = repository();
    let path = workspace.path().display().to_string();
    workspace.git(&["remote", "add", "origin", &path]);
    workspace.write(
        "runner-temp/goodfetch-os-services/env",
        b"XDG_RUNTIME_DIR=/stand-in/runtime\n",
    );
    workspace
}

/// Runs the repository's `ci/test-health/scheduled.sh` in the workspace, the
/// stand-in first on `PATH`, the logs in the workspace, and any disk enough.
fn scheduled(workspace: &Workspace, stand_in: &Path, run: &str) -> Run {
    let logs = workspace.path().join("logs");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/test-health/scheduled.sh");
    let output = workspace
        .command(script)
        .arg(run)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env("GOODFETCH_TEST_HEALTH_LOG", &logs)
        .env("GOODFETCH_TEST_HEALTH_MIN_FREE_GB", "0")
        .env("RUNNER_TEMP", workspace.path().join("runner-temp"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    let read = |path: PathBuf| std::fs::read_to_string(path).unwrap_or_default();
    let log = std::fs::read_dir(&logs)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .map(|entry| read(entry.path()))
        .collect();
    Run {
        code: output.status.code(),
        calls: read(stand_in.join("calls"))
            .lines()
            .map(str::to_owned)
            .collect(),
        results: read(logs.join("results.txt")),
        log,
    }
}

/// Leaves `just merge`'s lock as a gate holds it, naming `holder`.
fn hold_the_merge_gate(workspace: &Workspace, holder: &str) {
    workspace.write(".claude/worktrees/merge-gate.lock/pid", holder.as_bytes());
}

/// On 2026-11-08 the daily mutation run ran beside a gate, and the load it
/// added failed the gate on a leak timeout.
#[rstest]
#[case::mutants("mutants")]
#[case::constrained("constrained")]
fn a_run_skips_while_the_merge_gate_has_a_live_owner(#[case] run: &str) {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    hold_the_merge_gate(&workspace, &std::process::id().to_string());

    let outcome = scheduled(&workspace, &stand_in, run);

    assert_eq!(outcome.code, Some(1), "{}", outcome.log);
    assert!(
        outcome
            .results
            .ends_with(&format!(" {run}=skipped (the merge gate is running)\n")),
        "{}",
        outcome.results
    );
    assert!(outcome.calls.is_empty(), "{:?}", outcome.calls);
}

/// Cron gives a run no user session, so without the OS services' setup the
/// mutation run's baseline failed on Collie's governor test, which needs the
/// user's systemd through `XDG_RUNTIME_DIR`, and tested no mutant (exit 4 on
/// 2026-11-07 and 2026-11-08). It starts them as the constrained run does.
#[cfg(target_os = "linux")]
#[test]
fn a_mutation_run_finds_the_os_services_the_setup_started() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);

    let outcome = scheduled(&workspace, &stand_in, "mutants");

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    let variables = std::fs::read_to_string(stand_in.join("variables")).unwrap();
    assert!(
        variables.contains("\n_os-services||\n"),
        "the setup did not run: {variables}"
    );
    assert!(
        variables.contains("\nmutants|/stand-in/runtime|required\n"),
        "{variables}"
    );
}

/// A gate that died left its lock behind: that holds up no run, and the lock
/// stays for the next gate to clear.
#[test]
fn a_run_goes_on_past_a_dead_owners_merge_gate_lock() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    hold_the_merge_gate(&workspace, NO_PROCESS);

    let outcome = scheduled(&workspace, &stand_in, "mutants");

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    assert!(
        outcome.results.ends_with(" mutants=pass\n"),
        "{}",
        outcome.results
    );
    assert!(
        outcome
            .calls
            .iter()
            .any(|call| call.starts_with("mutants ")),
        "{:?}",
        outcome.calls
    );
    assert!(
        workspace
            .path()
            .join(".claude/worktrees/merge-gate.lock")
            .exists()
    );
}
