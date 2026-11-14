//! The test-health runs the gate host starts (`just test-health`,
//! `ci/test-health/scheduled.sh`; TESTING.md's After merge): a run does not start
//! while `just merge`'s gate runs, and says so in its results line. It marks the
//! lock it holds for a gate started during it to wait on, which `merge_pr` tests.
//! The post-merge run checks what `main` gained since the commit it last
//! checked, and only when `main` moved without a full gate, in the merge
//! gate's worktree, where a gate starts it as it ends.
//! `just` is a stand-in that records each call. The
//! local gate is repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use rstest::rstest;

use crate::affected::repository;
use crate::merge_pr::until;
use crate::workspace::{Started, Workspace, write_executable};

/// A process ID above every system's limit, so no process has it.
const NO_PROCESS: &str = "2147483646";

/// The lock a run takes, which holds its mark while it runs.
fn backstop_lock(workspace: &Workspace) -> PathBuf {
    workspace
        .path()
        .join(".claude/worktrees/gate-backstop.lock")
}

/// A stand-in `just` that appends its arguments to `calls`, its first argument
/// and its working folder to `where`, and its arguments with the
/// OS services' variables it was given to `variables`, writes its whole
/// environment to `environment-<first argument>`, appends its first argument
/// to `post-merge-marked` while the lock says a post-merge run holds it, and
/// fails with status 3 while `failing-<first argument>` exists. While
/// `blocking-<first argument>` exists, it starts a `sleep` it waits for, and
/// writes its own process and the sleep's to `blocked-<first argument>`; told
/// to stop while `lingering` exists, it takes a second more to end and then
/// writes `lingered`; a blocking one then also starts a process in a process
/// group of its own, as nextest starts a test, noted in `apart`, which told
/// to stop takes a moment and writes `lingered-apart`.
/// Beside it, `sudo` keeps
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
             echo \"$1 $PWD\" >>\"{folder}/where\"\n\
             echo \"$1|${{XDG_RUNTIME_DIR:-}}|${{GOODFETCH_OS_SERVICES:-}}\" >>\"{variables}\"\n\
             env >\"{folder}/environment-$1\"\n\
             if [ -f \"{mark}\" ]; then cat \"{pid}\" >\"{folder}/marked-by\"; fi\n\
             if [ -f \"{post_merge_mark}\" ]; then echo \"$1\" >>\"{folder}/post-merge-marked\"; fi\n\
             if [ -f \"{folder}/lingering\" ]; then\n\
               trap 'sleep 1; : >\"{folder}/lingered\"; exit 0' TERM\n\
             fi\n\
             if [ -f \"{folder}/blocking-$1\" ]; then\n\
               if [ -f \"{folder}/lingering\" ]; then\n\
                 perl -e 'setpgrp(0, 0); exec @ARGV' sh -c 'trap \"sleep 0.3; : >\\\"$0\\\"; exit 0\" TERM; sleep 600 & wait' \"{folder}/lingered-apart\" </dev/null >/dev/null 2>&1 &\n\
                 echo $! >\"{folder}/apart\"\n\
               fi\n\
               sleep 600 & echo \"$$ $!\" >\"{folder}/blocking\"; mv \"{folder}/blocking\" \"{folder}/blocked-$1\"; wait\n\
             fi\n\
             if [ -f \"{folder}/failing-$1\" ]; then exit 3; fi\n",
            calls = folder.join("calls").display(),
            variables = folder.join("variables").display(),
            folder = folder.display(),
            mark = backstop_lock(workspace).join("test-health").display(),
            post_merge_mark = backstop_lock(workspace).join("post-merge").display(),
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

/// The commit `revision` names in the fixture.
fn commit(workspace: &Workspace, revision: &str) -> String {
    workspace.git(&["rev-parse", revision]).trim().to_owned()
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

/// The repository's `ci/test-health/scheduled.sh` in the workspace, the
/// stand-in first on `PATH`, the logs in the workspace, and any disk enough.
fn scheduled_command(workspace: &Workspace, stand_in: &Path, run: &str) -> Command {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/test-health/scheduled.sh");
    let mut command = workspace.command(script);
    command
        .arg(run)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env("GOODFETCH_TEST_HEALTH_LOG", workspace.path().join("logs"))
        .env("GOODFETCH_TEST_HEALTH_MIN_FREE_GB", "0")
        .env("RUNNER_TEMP", workspace.path().join("runner-temp"))
        .env("TMPDIR", workspace.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    command
}

/// Runs the repository's `ci/test-health/scheduled.sh` as
/// [`scheduled_command`] sets it up.
fn scheduled(workspace: &Workspace, stand_in: &Path, run: &str) -> Run {
    let output = scheduled_command(workspace, stand_in, run)
        .output()
        .unwrap();
    ended(workspace, stand_in, output.status.code())
}

/// What a run that ended with `code` called and wrote.
fn ended(workspace: &Workspace, stand_in: &Path, code: Option<i32>) -> Run {
    let logs = workspace.path().join("logs");
    let read = |path: PathBuf| std::fs::read_to_string(path).unwrap_or_default();
    let log = std::fs::read_dir(&logs)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .map(|entry| read(entry.path()))
        .collect();
    Run {
        code,
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
/// the day this came in may predate it. The post-merge run has no line: a
/// `just merge` gate starts it (`merge_pr`).
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

/// The commit the post-merge run last checked, as it keeps it beside its log.
fn post_merge_checked(workspace: &Workspace) -> PathBuf {
    workspace.path().join("logs/post-merge-checked")
}

/// Marks `revision` as the commit the post-merge run last checked.
fn checked_up_to(workspace: &Workspace, revision: &str) {
    let checked = commit(workspace, revision);
    workspace.write("logs/post-merge-checked", format!("{checked}\n").as_bytes());
}

fn checked(workspace: &Workspace) -> String {
    std::fs::read_to_string(post_merge_checked(workspace))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// Every gate that held its lock starts the post-merge run: while `main` has
/// not moved since the commit it checked, it runs nothing and writes no line.
#[test]
fn a_post_merge_run_does_nothing_while_main_has_not_moved() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    checked_up_to(&workspace, "main");

    let outcome = scheduled(&workspace, &stand_in, "post-merge");

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    assert!(outcome.calls.is_empty(), "{:?}", outcome.calls);
    assert_eq!(outcome.results, "");
    assert_eq!(checked(&workspace), commit(&workspace, "main"));
}

/// A post-merge run that cannot run now leaves the commit it checked where it
/// was, for the next run to cover the range, and says so in results.txt,
/// where the watch reads (#2619: its log alone said it, and nothing showed
/// that `main` was no longer checked): once for a commit and a reason, however
/// many runs find the same.
#[rstest]
#[case::the_merge_gate_runs("merge-gate", "the merge gate is running")]
#[case::the_backstops_lock_is_held("gate-backstop", "the backstop's worktree is busy")]
fn a_post_merge_run_that_cannot_start_says_so_once_in_its_results(
    #[case] held: &str,
    #[case] reason: &str,
) {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    checked_up_to(&workspace, "main^");
    workspace.write(
        &format!(".claude/worktrees/{held}.lock/pid"),
        std::process::id().to_string().as_bytes(),
    );

    let first = scheduled(&workspace, &stand_in, "post-merge");
    let second = scheduled(&workspace, &stand_in, "post-merge");

    assert_eq!(first.code, Some(0), "{}", first.log);
    assert_eq!(second.code, Some(0), "{}", second.log);
    assert!(second.calls.is_empty(), "{:?}", second.calls);
    let head = commit(&workspace, "main");
    assert!(
        second
            .results
            .ends_with(&format!(" {head} post-merge=skipped ({reason})\n")),
        "{}",
        second.results
    );
    assert_eq!(second.results.lines().count(), 1, "{}", second.results);
    assert_eq!(checked(&workspace), commit(&workspace, "main^"));
    assert_eq!(backstop_lock(&workspace).exists(), held == "gate-backstop");
}

/// A post-merge run short of room says on which file system, the one its
/// build goes to, with no figure that moves between two runs.
#[test]
fn a_post_merge_run_short_of_room_names_the_file_system_in_its_results() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    stand_in_df(&stand_in);
    checked_up_to(&workspace, "main^");

    let status = scheduled_command(&workspace, &stand_in, "post-merge")
        .env("GOODFETCH_TEST_HEALTH_MIN_FREE_GB", "2000")
        .status()
        .unwrap();

    let outcome = ended(&workspace, &stand_in, status.code());
    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    assert!(outcome.calls.is_empty(), "{:?}", outcome.calls);
    assert!(
        outcome
            .results
            .ends_with(" post-merge=skipped (free disk under 2000 GB on /stand-in)\n"),
        "{}",
        outcome.results
    );
    assert_eq!(checked(&workspace), commit(&workspace, "main^"));
}

/// A post-merge run that ends on its setup, before its suites, reads as
/// failed and leaves the commit it checked where it was (#2619: it moved it,
/// so a range no suite had run was listed red once and never run).
#[test]
fn a_post_merge_run_whose_setup_fails_leaves_its_range() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing-fetch"), "").unwrap();
    checked_up_to(&workspace, "main^");

    let outcome = scheduled(&workspace, &stand_in, "post-merge");

    assert_eq!(outcome.code, Some(3), "{}", outcome.log);
    let (head, since) = (commit(&workspace, "main"), commit(&workspace, "main^"));
    assert!(
        outcome
            .results
            .ends_with(&format!(" {head} post-merge=fail since {since}\n")),
        "{}",
        outcome.results
    );
    assert_eq!(checked(&workspace), since);
}

/// The skipped line is written once for a commit and a reason, whatever
/// came between: two reasons that take turns leave a line each.
#[test]
fn a_post_merge_runs_reasons_that_take_turns_are_each_written_once() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    checked_up_to(&workspace, "main^");
    let lock = |name: &str| format!(".claude/worktrees/{name}.lock");
    let hold = |name: &str| {
        workspace.write(
            &format!("{}/pid", lock(name)),
            std::process::id().to_string().as_bytes(),
        );
    };
    let release = |name: &str| std::fs::remove_dir_all(workspace.path().join(lock(name))).unwrap();

    hold("merge-gate");
    let first = scheduled(&workspace, &stand_in, "post-merge");
    release("merge-gate");
    hold("gate-backstop");
    let second = scheduled(&workspace, &stand_in, "post-merge");
    release("gate-backstop");
    hold("merge-gate");
    let third = scheduled(&workspace, &stand_in, "post-merge");

    assert_eq!(
        (first.code, second.code, third.code),
        (Some(0), Some(0), Some(0)),
        "{}",
        third.log
    );
    let reasons: Vec<&str> = third
        .results
        .lines()
        .filter_map(|line| line.split_once(" post-merge=").map(|(_, said)| said))
        .collect();
    assert_eq!(
        reasons,
        [
            "skipped (the merge gate is running)",
            "skipped (the backstop's worktree is busy)"
        ]
    );
}

/// Lands a merge on the fixture's `main`, with `main`'s tree, whose message
/// says the full gate passed, in the words `ci/merge-pr.sh` writes.
fn land_on_the_full_gate(workspace: &Workspace) {
    let main = "refs/heads/main";
    let tree = format!("{main}^{{tree}}");
    let change = workspace.git(&["commit-tree", &tree, "-p", main, "-m", "A change"]);
    let change = change.trim();
    let message = format!(
        "A change (#1)\n\nMerged on the local gate: `just pre-merge all` (asked for with `--full`), which starts with `just quick all`'s lint and Quick suite, passed at {change}."
    );
    let merge = workspace.git(&[
        "commit-tree",
        &tree,
        "-p",
        main,
        "-p",
        change,
        "-m",
        &message,
    ]);
    workspace.git(&["update-ref", main, merge.trim()]);
}

/// A full gate ran the Pre-merge suite and `just counts` on the merge's tree
/// already: a `main` whose newest merge passed it is recorded as checked, and
/// nothing runs again.
#[test]
fn a_post_merge_run_records_a_full_gates_merge_as_checked_without_running() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    checked_up_to(&workspace, "main");
    land_on_the_full_gate(&workspace);

    let outcome = scheduled(&workspace, &stand_in, "post-merge");

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    assert!(outcome.calls.is_empty(), "{:?}", outcome.calls);
    assert_eq!(outcome.results, "");
    assert_eq!(checked(&workspace), commit(&workspace, "main"));
}

/// Quick merges skip `just counts` and the Pre-merge suite, so the post-merge
/// run runs both over what `main` gained since the commit it checked, the
/// second even when the first fails, says how they ended in results.txt, and
/// moves on either way: a red range is reported, not retried. Its first run
/// starts from `main`'s commit before its head.
#[rstest]
#[case::green(false)]
#[case::red(true)]
fn a_post_merge_run_runs_the_counts_and_the_pre_merge_suite_over_the_range(#[case] red: bool) {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    if red {
        std::fs::write(stand_in.join("failing-counts"), "").unwrap();
    }
    let since = commit(&workspace, "main^");
    let head = commit(&workspace, "main");

    let outcome = scheduled(&workspace, &stand_in, "post-merge");

    assert_eq!(outcome.code, Some(i32::from(red)), "{}", outcome.log);
    for call in [
        "counts".to_owned(),
        format!("test-affected pre-merge --base {since} --head {head}"),
    ] {
        assert!(
            outcome.calls.contains(&call),
            "no `{call}` in {:?}",
            outcome.calls
        );
    }
    let verdict = if red { "fail" } else { "pass" };
    assert!(
        outcome
            .results
            .ends_with(&format!(" {head} post-merge={verdict} since {since}\n")),
        "{}",
        outcome.results
    );
    assert_eq!(outcome.results.lines().count(), 1, "{}", outcome.results);
    assert_eq!(checked(&workspace), head);
}

/// The post-merge run builds where `just merge`'s gate just built `main`'s
/// tree, in the merge gate's worktree, so no second build folder exists
/// (#2619: its own, in the backstop's worktree, took 36 GB and kept the disk
/// under the run's own floor); the other runs keep the backstop's worktree.
#[rstest]
#[case::post_merge("post-merge", "counts", "merge-gate", "gate-backstop")]
#[case::mutants("mutants", "mutants", "gate-backstop", "merge-gate")]
fn a_run_builds_in_its_worktree(
    #[case] run: &str,
    #[case] suite: &str,
    #[case] builds_in: &str,
    #[case] untouched: &str,
) {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);

    let outcome = scheduled(&workspace, &stand_in, run);

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    let folders = std::fs::read_to_string(stand_in.join("where")).unwrap();
    let folder = folders
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{suite} ")))
        .unwrap_or_else(|| panic!("no `{suite}` in {folders}"));
    assert!(
        folder.ends_with(&format!(".claude/worktrees/{builds_in}")),
        "{folders}"
    );
    let worktrees = workspace.path().join(".claude/worktrees");
    assert!(!worktrees.join(untouched).exists(), "{folders}");
}

/// A stand-in `df` that appends the folder it is asked about to `df-asked`
/// and answers with 1,024 GB free.
fn stand_in_df(stand_in: &Path) {
    write_executable(
        &stand_in.join("df"),
        &format!(
            "#!/bin/sh\nfor folder; do :; done\necho \"$folder\" >>\"{asked}\"\n\
             echo 'Filesystem 1024-blocks Used Available Capacity Mounted on'\n\
             echo '/dev/stand-in 2147483648 1073741824 1073741824 50% /stand-in'\n",
            asked = stand_in.join("df-asked").display(),
        ),
    );
}

/// The folder the last `df` was asked about.
fn df_asked(stand_in: &Path) -> String {
    let asked = std::fs::read_to_string(stand_in.join("df-asked")).unwrap();
    asked.lines().last().unwrap().to_owned()
}

/// A run asks for room on the file system its build goes to (#2619: a run
/// read the checkout's disk and built on another): that of its worktree's
/// `target/`, which can be a mount of its own, once it exists; before that
/// of the worktree, and before the worktree exists of the checkout it is
/// created in.
#[rstest]
#[case::post_merge("post-merge", "merge-gate")]
#[case::mutants("mutants", "gate-backstop")]
fn a_run_reads_the_free_space_where_its_build_goes(#[case] run: &str, #[case] builds_in: &str) {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    stand_in_df(&stand_in);
    let root = workspace.path().canonicalize().unwrap();

    let first = scheduled(&workspace, &stand_in, run);

    assert_eq!(first.code, Some(0), "{}", first.log);
    assert_eq!(df_asked(&stand_in), format!("{}/", root.display()));

    // The first run created the worktree, and no build its `target/`.
    let worktree = root.join(".claude/worktrees").join(builds_in);
    commit_to_main(&workspace, &[("moved", b"main moved\n")]);

    let second = scheduled(&workspace, &stand_in, run);

    assert_eq!(second.code, Some(0), "{}", second.log);
    assert_eq!(df_asked(&stand_in), format!("{}/", worktree.display()));

    let target = worktree.join("target");
    std::fs::create_dir_all(&target).unwrap();
    commit_to_main(&workspace, &[("moved", b"main moved again\n")]);

    let third = scheduled(&workspace, &stand_in, run);

    assert_eq!(third.code, Some(0), "{}", third.log);
    assert_eq!(df_asked(&stand_in), format!("{}/", target.display()));
}

/// A gate stops a post-merge run rather than wait for it (ci/merge-pr.sh), so
/// the run marks its lock as a post-merge run's; the other runs do not.
#[rstest]
#[case::post_merge("post-merge", true)]
#[case::mutants("mutants", false)]
fn only_a_post_merge_run_marks_its_lock_as_a_post_merge_runs(
    #[case] run: &str,
    #[case] marked: bool,
) {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);

    let outcome = scheduled(&workspace, &stand_in, run);

    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    let seen = std::fs::read_to_string(stand_in.join("post-merge-marked")).unwrap_or_default();
    assert_eq!(seen.lines().any(|call| call == "counts"), marked, "{seen}");
    assert_eq!(seen.is_empty(), !marked, "{seen}");
}

/// Whether process `pid` still runs, a zombie not counted.
fn runs(pid: &str) -> bool {
    Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .is_ok_and(|output| {
            let state = String::from_utf8_lossy(&output.stdout);
            output.status.success() && !state.trim_start().starts_with('Z')
        })
}

/// A gate stops a post-merge run with TERM (ci/merge-pr.sh): the run ends its
/// suites and every process they started, says in results.txt that a gate
/// stopped it and leaves the commit it checked where it was, so the run that
/// gate starts as it ends covers the range. The gate checks its head out in
/// the worktree the run built in once the run's lock is gone, so the run ends,
/// and its lock goes, only when no process of its suites is left, one that
/// takes a moment to stop included, and one in a process group of its own,
/// as nextest gives each test, which the stop of the suites' group does not
/// reach.
#[test]
fn a_post_merge_run_stopped_by_a_gate_ends_its_suites_and_leaves_its_range() {
    let workspace = origin_repository();
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("blocking-counts"), "").unwrap();
    std::fs::write(stand_in.join("lingering"), "").unwrap();
    checked_up_to(&workspace, "main^");
    let mut run = Started::spawn(
        scheduled_command(&workspace, &stand_in, "post-merge")
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
    let blocked = stand_in.join("blocked-counts");
    until("the counts' start", || blocked.is_file());
    let holder = std::fs::read_to_string(backstop_lock(&workspace).join("pid")).unwrap();

    let sent = Command::new("kill")
        .args(["-TERM", holder.trim()])
        .status()
        .unwrap();
    let code = run.wait().code();

    assert!(sent.success());
    let left: Vec<String> = std::fs::read_to_string(&blocked)
        .unwrap()
        .split_whitespace()
        .filter(|process| runs(process))
        .map(str::to_owned)
        .collect();
    let lingered = stand_in.join("lingered").is_file();
    let apart = stand_in.join("lingered-apart").is_file();
    let apart_runs = runs(
        std::fs::read_to_string(stand_in.join("apart"))
            .unwrap()
            .trim(),
    );
    let outcome = ended(&workspace, &stand_in, code);
    assert_eq!(outcome.code, Some(0), "{}", outcome.log);
    assert!(
        lingered,
        "the run ended before its suite had: {}",
        outcome.log
    );
    assert!(
        apart && !apart_runs,
        "the run ended before its suite's process in a group of its own had: {}",
        outcome.log
    );
    assert!(left.is_empty(), "{left:?} still ran: {}", outcome.log);
    let head = commit(&workspace, "main");
    assert!(
        outcome.results.ends_with(&format!(
            " {head} post-merge=skipped (a merge gate stopped the run)\n"
        )),
        "{}",
        outcome.results
    );
    assert_eq!(outcome.results.lines().count(), 1, "{}", outcome.results);
    assert_eq!(checked(&workspace), commit(&workspace, "main^"));
    assert!(!backstop_lock(&workspace).exists());
}
