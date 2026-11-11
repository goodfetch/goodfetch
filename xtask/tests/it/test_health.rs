//! The test-health runs the gate host's cron starts (`just test-health`,
//! `ci/test-health/scheduled.sh`; TESTING.md's After merge): a run does not start
//! while `just merge`'s gate runs, and says so in its results line. It marks the
//! lock it holds for a gate started during it to wait on, which `merge_pr` tests.
//! `just` is a stand-in that records each call. The
//! local gate is repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Output;

use rstest::rstest;

use crate::affected::repository;
use crate::workspace::Workspace;

/// A process ID above every system's limit, so no process has it.
const NO_PROCESS: &str = "2147483646";

/// The lock a run takes, which holds its mark while it runs.
fn backstop_lock(workspace: &Workspace) -> PathBuf {
    workspace
        .path()
        .join(".claude/worktrees/gate-backstop.lock")
}

fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in `just` that appends its arguments to `calls`, and them with the
/// OS services' variables it was given to `variables`, writes its whole
/// environment to `environment-<first argument>`, and fails with status 3
/// while `failing-<first argument>` exists. Beside it, `sudo` keeps
/// nothing of its caller's environment, as `env_reset` does, and `systemd-run`
/// runs what follows `--`, so the constrained run's suite sees only what the
/// script hands it.
fn stand_in_just(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    write_executable(
        &folder.join("just"),
        &format!(
            "#!/bin/sh\necho \"$*\" >>\"{calls}\"\n\
             echo \"$1|${{XDG_RUNTIME_DIR:-}}|${{GOODFETCH_OS_SERVICES:-}}\" >>\"{variables}\"\n\
             env >\"{folder}/environment-$1\"\n\
             if [ -f \"{mark}\" ]; then cat \"{pid}\" >\"{folder}/marked-by\"; fi\n\
             if [ -f \"{folder}/failing-$1\" ]; then exit 3; fi\n",
            calls = folder.join("calls").display(),
            variables = folder.join("variables").display(),
            folder = folder.display(),
            mark = backstop_lock(workspace).join("test-health").display(),
            pid = backstop_lock(workspace).join("pid").display(),
        ),
    );
    write_executable(
        &folder.join("sudo"),
        "#!/bin/sh\nexec /usr/bin/env -i PATH=/usr/bin:/bin \"$@\"\n",
    );
    write_executable(
        &folder.join("systemd-run"),
        "#!/bin/sh\nwhile [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"\n",
    );
    folder
}

/// The variables the stand-in `just` was given when it ran the suite.
fn suite_environment(stand_in: &Path) -> Vec<String> {
    std::fs::read_to_string(stand_in.join("environment-test"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
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
        b"DBUS_SESSION_BUS_ADDRESS=unix:path=/stand-in/bus\nXDG_RUNTIME_DIR=/stand-in/runtime\n",
    );
    let repository_ci = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci");
    commit_to_main(
        &workspace,
        &[
            (
                "ci/test-health/scheduled.sh",
                &std::fs::read(repository_ci.join("test-health/scheduled.sh")).unwrap(),
            ),
            (
                "ci/gate-worktree.sh",
                &std::fs::read(repository_ci.join("gate-worktree.sh")).unwrap(),
            ),
        ],
    );
    workspace
}

/// Commits `files` to the fixture's `main` and goes back to its other branch.
fn commit_to_main(workspace: &Workspace, files: &[(&str, &[u8])]) {
    workspace.git(&["switch", "--quiet", "main"]);
    for (file, contents) in files {
        workspace.write(file, contents);
        workspace.git(&["add", file]);
    }
    workspace.git(&["commit", "--quiet", "--message", "Change the scripts"]);
    workspace.git(&["switch", "--quiet", "change"]);
}

/// The repository's `ci/test-health/constrained.sh`.
fn constrained_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/test-health/constrained.sh")
}

/// Commits the repository's constrained script to the fixture's `main`, where
/// `scheduled.sh`'s worktree finds it.
fn commit_constrained_script(workspace: &Workspace) {
    let relative = "ci/test-health/constrained.sh";
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.write(relative, &std::fs::read(constrained_script()).unwrap());
    let script = workspace.path().join(relative);
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755)).unwrap();
    workspace.git(&["add", relative]);
    workspace.git(&["commit", "--quiet", "--message", "Add the constrained run"]);
}

/// Runs the repository's `ci/test-health/constrained.sh` alone, with a seed,
/// the stand-ins first on `PATH`, and only these other variables.
fn constrained(workspace: &Workspace, stand_in: &Path, variables: &[(&str, &str)]) -> Output {
    workspace
        .command(constrained_script())
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env("GOODFETCH_ORDER_SEED", "1")
        .envs(variables.iter().copied())
        .output()
        .unwrap()
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
        .env("TMPDIR", workspace.path())
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

/// A checkout nothing fast-forwards holds an old script. A run is the commit's
/// own, whichever script started it.
#[test]
fn a_run_uses_the_script_of_the_commit_it_tests() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    let marker = workspace.path().join("the-commits-script-ran");
    commit_to_main(
        &workspace,
        &[(
            "ci/test-health/scheduled.sh",
            format!("#!/bin/bash\necho \"$*\" >>\"{}\"\n", marker.display()).as_bytes(),
        )],
    );

    let outcome = scheduled(&workspace, &stand_in, "mutants");

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    assert!(outcome.calls.is_empty(), "{:?}", outcome.calls);
    let commit = workspace.git(&["rev-parse", "main"]);
    assert_eq!(
        std::fs::read_to_string(marker).unwrap().trim(),
        format!("mutants {}", commit.trim())
    );
}

/// Cron's lines start from `origin/main`'s script, not the checkout's, which on
/// the day this came in may predate it.
#[rstest]
#[case::mutants(0, "mutants")]
#[case::constrained(1, "constrained")]
fn a_crontab_line_runs_the_script_of_origin_main(#[case] index: usize, #[case] run: &str) {
    let crontab = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/test-health.crontab"),
    )
    .unwrap();
    let lines: Vec<&str> = crontab
        .lines()
        .filter_map(|line| line.split_once(" bash -lc '"))
        .filter_map(|(_, line)| line.strip_suffix('\''))
        .collect();
    assert_eq!(lines.len(), 2, "{crontab}");
    let line = lines[index];
    assert!(
        line.ends_with(&format!(" scheduled {run} origin/main")),
        "{line}"
    );
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    let marker = workspace.path().join("ran");
    commit_to_main(
        &workspace,
        &[(
            "ci/test-health/scheduled.sh",
            format!("#!/bin/bash\necho \"$*\" >>\"{}\"\n", marker.display()).as_bytes(),
        )],
    );

    let output = workspace
        .command("bash")
        .args(["-c", line])
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env("GOODFETCH_CHECKOUT", workspace.path())
        .env("GOODFETCH_TEST_HEALTH_LOG", workspace.path().join("logs"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();

    assert!(output.status.success(), "{line}: {output:?}");
    assert!(marker.is_file(), "{line}: {output:?}");
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
    assert!(!backstop_lock(&workspace).exists());
}

/// A gate started during a run waits for what the lock marks, and a run that
/// ended leaves nothing to wait for.
#[test]
fn a_run_marks_its_lock_as_a_test_health_runs_while_it_runs() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);

    let outcome = scheduled(&workspace, &stand_in, "mutants");

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    assert!(stand_in.join("marked-by").is_file(), "{}", outcome.log);
    assert!(!backstop_lock(&workspace).exists());
}

/// A reader of the results sees every day's run: one that ends before its
/// tests, on a setup it needs, reads as failed, never as nothing.
#[cfg(target_os = "linux")]
#[rstest]
#[case::the_fetch("fetch")]
#[case::the_os_services("_os-services")]
fn a_run_whose_setup_fails_leaves_a_failed_results_line(#[case] setup: &str) {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join(format!("failing-{setup}")), "").unwrap();

    let outcome = scheduled(&workspace, &stand_in, "mutants");

    assert_eq!(outcome.code, Some(3), "{}", outcome.log);
    assert!(
        outcome.results.ends_with(" mutants=fail\n"),
        "{}",
        outcome.results
    );
    assert_eq!(outcome.results.lines().count(), 1, "{}", outcome.results);
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

/// `sudo` resets the environment: on 2026-11-08 the constrained run built for
/// 13 minutes, then `just test pre-merge` stopped before any test, its session
/// bus and runtime folder gone. The way the DNS namespace comes up, declared
/// so a fallback fails, and the temporary folder, whose files in a tmpfs would
/// count against the run's memory limit, go through with them.
#[cfg(target_os = "linux")]
#[test]
fn a_constrained_run_hands_its_suite_what_the_run_set_up() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    commit_constrained_script(&workspace);

    let outcome = scheduled(&workspace, &stand_in, "constrained");

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    assert!(
        outcome.calls.iter().any(|call| call == "test pre-merge"),
        "{:?}",
        outcome.calls
    );
    let environment = suite_environment(&stand_in);
    let temporary_folder = format!("TMPDIR={}", workspace.path().display());
    for variable in [
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/stand-in/bus",
        "XDG_RUNTIME_DIR=/stand-in/runtime",
        "GOODFETCH_TEST_DNS_NAMESPACE=sudo",
        &temporary_folder,
    ] {
        assert!(
            environment.iter().any(|line| line == variable),
            "no {variable} in {environment:?}"
        );
    }
}

/// The hosted job declares no way for the DNS namespace and no temporary
/// folder. Set empty, the first would fail every test that enters a namespace.
#[cfg(target_os = "linux")]
#[test]
fn a_constrained_run_sets_no_variable_its_caller_left_unset() {
    let workspace = repository();
    let stand_in = stand_in_just(&workspace);

    let output = constrained(
        &workspace,
        &stand_in,
        &[
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/stand-in/bus"),
            ("XDG_RUNTIME_DIR", "/stand-in/runtime"),
        ],
    );

    assert!(output.status.success(), "{output:?}");
    let environment = suite_environment(&stand_in);
    for variable in ["GOODFETCH_TEST_DNS_NAMESPACE=", "TMPDIR="] {
        assert!(
            !environment.iter().any(|line| line.starts_with(variable)),
            "{variable} in {environment:?}"
        );
    }
}

/// A suite without them stops only after its build, so the run stops first.
#[cfg(target_os = "linux")]
#[rstest]
#[case::no_session_bus(&[("XDG_RUNTIME_DIR", "/stand-in/runtime")])]
#[case::no_runtime_folder(&[("DBUS_SESSION_BUS_ADDRESS", "unix:path=/stand-in/bus")])]
fn a_constrained_run_without_the_os_services_starts_no_suite(#[case] variables: &[(&str, &str)]) {
    let workspace = repository();
    let stand_in = stand_in_just(&workspace);

    let output = constrained(&workspace, &stand_in, variables);

    assert!(!output.status.success(), "{output:?}");
    assert!(!stand_in.join("calls").exists(), "{output:?}");
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
