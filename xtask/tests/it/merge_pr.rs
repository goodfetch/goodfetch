//! `just merge` (`ci/merge-pr.sh`, AGENTS.md's Merging; TESTING.md's
//! Commands) around its gate: it refuses to start one while the file system
//! of the gate's `target/` is short of space (#1822), keeps a failing gate's
//! log and test results in the repository's common folder (#2087), and
//! sweeps the gate's `target/` after each gate, still holding its lock; and
//! the gate outlives the shell that started it, whose output it no longer
//! needs. `gh`, `cargo` and `just` are stand-ins that record each call, and
//! `date` one that gives the gate's start as [`STARTED`]. The local gate is
//! repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rstest::rstest;

use crate::affected::{commit_all, repository};
use crate::workspace::Workspace;

const NUMBER: &str = "7";

/// The Unix time the `date` stand-in gives for `date -u +%s`.
const STARTED: &str = "1800000000";

/// Stand-ins for `gh`, `cargo` and `just` that append each call to `calls`,
/// and for `date`, which gives [`STARTED`] for `-u +%s` and leaves every
/// other format to the system's `date`.
/// `gh pr view` shows an open pull request at the head of `change`; `cargo
/// run … affected` prints `gate=quick`, or the gate `gate` names, its sweep
/// notes `sweeping` and waits while `sweep-hold` exists, and its `perf
/// pull-request` fails while `acceptance-unread` exists; `just pre-merge`
/// writes the plan a full gate reads, unless `no-plan` exists, and a JUnit file,
/// notes the signals it ignores in `signals` where the system lists them,
/// notes its process in `reached`, waits while `hold` exists, writes a line
/// and, while `failing` exists, fails with status 3.
fn stand_ins(workspace: &Workspace, head: &str) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let calls = folder.join("calls");
    let scripts = [
        (
            "gh",
            format!(
                "#!/bin/sh\necho \"gh $*\" >>\"{calls}\"\n\
                 if [ \"$1 $2\" = \"pr view\" ]; then printf 'OPEN\\tfalse\\t{head}\\tmain\\tA change\\n'; fi\n",
                calls = calls.display(),
            ),
        ),
        (
            "cargo",
            format!(
                "#!/bin/sh\necho \"cargo $* in $PWD\" >>\"{calls}\"\n\
                 case \" $* \" in\n\
                   *\" affected \"*) if [ -f \"{gate}\" ]; then echo gate=$(cat \"{gate}\"); else echo gate=quick; fi ;;\n\
                   *\" sweep-target \"*)\n\
                     : >\"{sweeping}\"\n\
                     while [ -f \"{sweep_hold}\" ]; do sleep 0.02; done\n\
                     ;;\n\
                   *\" perf pull-request \"*) if [ -f \"{acceptance_unread}\" ]; then exit 1; fi ;;\n\
                 esac\n",
                calls = calls.display(),
                gate = folder.join("gate").display(),
                acceptance_unread = folder.join("acceptance-unread").display(),
                sweeping = folder.join("sweeping").display(),
                sweep_hold = folder.join("sweep-hold").display(),
            ),
        ),
        (
            "just",
            format!(
                "#!/bin/sh\necho \"just $*\" >>\"{calls}\"\n\
                 if [ \"$1\" = pre-merge ]; then\n\
                   mkdir -p target/gate-runs\n\
                   if [ ! -f \"{no_plan}\" ]; then echo previous=none >target/gate-runs/{head}.plan; fi\n\
                   mkdir -p target/nextest/quick && echo '<testsuites/>' >target/nextest/quick/junit.xml\n\
                   if [ -r /proc/self/status ]; then grep '^SigIgn:' /proc/self/status >\"{signals}\"; fi\n\
                   echo $$ >\"{reached}\"\n\
                   while [ -f \"{hold}\" ]; do sleep 0.02; done\n\
                   echo 'the steps ran'\n\
                   if [ -f \"{failing}\" ]; then echo 'a test failed'; exit 3; fi\n\
                 fi\n",
                calls = calls.display(),
                no_plan = folder.join("no-plan").display(),
                signals = folder.join("signals").display(),
                reached = folder.join("reached").display(),
                hold = folder.join("hold").display(),
                failing = folder.join("failing").display(),
            ),
        ),
        (
            "date",
            format!(
                "#!/bin/sh\nif [ \"$*\" = '-u +%s' ]; then echo {STARTED}; else exec /bin/date \"$@\"; fi\n"
            ),
        ),
    ];
    for (name, script) in scripts {
        let path = folder.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    calls: Vec<String>,
}

/// A repository whose `origin` is itself, with pull request #7 at a commit
/// on `change` that contains `main`; returns it with that head.
fn pull_request() -> (Workspace, String) {
    let workspace = repository();
    let path = workspace.path().display().to_string();
    workspace.git(&["remote", "add", "origin", &path]);
    workspace.write("change.txt", b"a change\n");
    commit_all(&workspace, "A change");
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.git(&["update-ref", &format!("refs/pull/{NUMBER}/head"), &head]);
    (workspace, head)
}

/// The repository's `ci/merge-pr.sh` on #7, the stand-ins first on `PATH`,
/// with these variables added.
fn merge_command(workspace: &Workspace, stand_in: &Path, variables: &[(&str, &str)]) -> Command {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/merge-pr.sh");
    let mut command = workspace.command(script);
    command
        .arg(NUMBER)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env(
            "GOODFETCH_GATE_BACKSTOP_LOG",
            workspace.path().join("no-backstop"),
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .envs(variables.iter().copied());
    command
}

fn calls(stand_in: &Path) -> Vec<String> {
    std::fs::read_to_string(stand_in.join("calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Runs `merge_command` to its end.
fn merge(workspace: &Workspace, stand_in: &Path, variables: &[(&str, &str)]) -> Run {
    let output = merge_command(workspace, stand_in, variables)
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        calls: calls(stand_in),
    }
}

/// Starts `merge_command` with its output in pipes, the leader of a process
/// group of its own, as a session's shell is, and waits until the gate's
/// steps run and wait on `hold`.
fn merge_up_to_its_steps(workspace: &Workspace, stand_in: &Path) -> Child {
    std::fs::write(stand_in.join("hold"), "").unwrap();
    let caller = merge_command(workspace, stand_in, &[("GOODFETCH_GATE_MIN_FREE_GB", "0")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    until("the gate's steps to start", || {
        std::fs::read_to_string(stand_in.join("reached")).is_ok_and(|pid| pid.ends_with('\n'))
    });
    caller
}

/// Waits, on the real clock, for something a process outside the test does.
fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "no sign of {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Sends `signal` to `target`, a process or, as `-<group>`, a process group.
fn signal(signal: &str, target: &str) {
    let sent = Command::new("kill")
        .args([signal, "--", target])
        .status()
        .unwrap();
    assert!(sent.success(), "kill {signal} {target}");
}

fn runs(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn gate_lock(workspace: &Workspace) -> PathBuf {
    workspace.path().join(".claude/worktrees/merge-gate.lock")
}

/// Waits until the gate has merged, then let its lock go.
fn until_the_gate_merged(workspace: &Workspace, stand_in: &Path) {
    until("the gate's merge", || {
        calls(stand_in)
            .iter()
            .any(|call| call.starts_with("gh pr merge"))
    });
    until("the gate's lock to go", || !gate_lock(workspace).exists());
}

fn gate_worktree(workspace: &Workspace) -> PathBuf {
    workspace.path().join(".claude/worktrees/merge-gate")
}

/// The index of the first call that starts with `prefix`.
fn call(run: &Run, prefix: &str) -> usize {
    run.calls
        .iter()
        .position(|call| call.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` call in {:?}", run.calls))
}

#[test]
fn a_gate_refuses_to_start_while_its_target_file_system_is_short_of_space() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "1000000000")],
    );

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("GB free, under the 1000000000 GB a gate's builds need"),
        "{}",
        run.stdout
    );
    assert!(
        !run.calls.iter().any(|call| call.starts_with("just ")),
        "{:?}",
        run.calls
    );
    assert!(
        !run.calls.iter().any(|call| call.contains("sweep-target")),
        "{:?}",
        run.calls
    );
    assert!(!gate_lock(&workspace).exists());
}

/// A green gate merges, then sweeps the gate's `target/` before it lets
/// the lock go, keeping every unit read since the gate started, however
/// long it ran.
#[test]
fn after_a_green_gate_the_target_is_swept_under_the_gates_lock() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let sweep = call(
        &run,
        "cargo run --quiet --locked --bin xtask -- sweep-target",
    );
    assert!(
        run.calls[sweep].ends_with(&format!(
            "sweep-target --read-since {STARTED} in {}",
            gate_worktree(&workspace).display()
        )),
        "{}",
        run.calls[sweep]
    );
    assert!(call(&run, "just pre-merge quick") < sweep);
    assert!(call(&run, "gh pr merge") < sweep);
    assert!(!gate_lock(&workspace).exists());
}

#[test]
fn a_failing_gate_keeps_its_log_and_test_results_and_is_swept_too() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("failing"), "").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(3), "{}{}", run.stdout, run.stderr);
    assert!(
        !run.calls.iter().any(|call| call.starts_with("gh pr merge")),
        "{:?}",
        run.calls
    );
    call(
        &run,
        "cargo run --quiet --locked --bin xtask -- sweep-target",
    );
    let failures = workspace.path().join(".git/goodfetch-gate-failures");
    let kept: Vec<PathBuf> = std::fs::read_dir(&failures)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    let name = kept[0].file_name().unwrap().to_str().unwrap();
    assert!(
        name.ends_with(&format!("-pr{NUMBER}-{}", &head[..12])),
        "{name}"
    );
    assert!(
        std::fs::read_to_string(kept[0].join("gate.log"))
            .unwrap()
            .contains("a test failed"),
        "{name}"
    );
    assert!(kept[0].join("nextest/quick/junit.xml").is_file());
    assert!(
        run.stdout
            .contains(&format!("kept in {}", kept[0].display())),
        "{}",
        run.stdout
    );
}

/// A session restart stops the shell that hosts `just merge`, with its
/// process group (2026-11-08: SIGTERM killed a gate 1 h 47 in). The gate is
/// past that reach and still merges.
#[test]
fn a_gate_merges_after_the_shell_that_started_it_dies() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let mut caller = merge_up_to_its_steps(&workspace, &stand_in);

    signal("-TERM", &format!("-{}", caller.id()));
    caller.wait().unwrap();
    std::fs::remove_file(stand_in.join("hold")).unwrap();

    until_the_gate_merged(&workspace, &stand_in);
}

/// Output the caller stops reading stops nothing: the gate's output goes to
/// its log.
#[test]
fn a_gate_merges_after_its_callers_output_closes() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let mut caller = merge_up_to_its_steps(&workspace, &stand_in);

    drop(caller.stdout.take());
    drop(caller.stderr.take());
    std::fs::remove_file(stand_in.join("hold")).unwrap();

    until_the_gate_merged(&workspace, &stand_in);
    caller.wait().unwrap();
}

/// While the caller lives it follows the gate: it shows the gate's output,
/// says where its log is, and ends with the gate's exit status.
#[test]
fn a_caller_shows_the_gates_output_and_log_and_ends_with_its_status() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("failing"), "").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(3), "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("a test failed"), "{}", run.stdout);
    let logs: Vec<PathBuf> = std::fs::read_dir(workspace.path().join(".git/goodfetch-gate-logs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect();
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert!(
        std::fs::read_to_string(&logs[0])
            .unwrap()
            .contains("a test failed")
    );
    assert!(
        run.stdout.contains(&logs[0].display().to_string()),
        "{}",
        run.stdout
    );
}

/// A stop signal to the process the gate's lock names stops the gate's steps
/// and lets the lock go; nothing merges, and the caller ends with 143.
#[rstest]
#[case::sigterm("-TERM")]
#[case::sighup("-HUP")]
#[case::sigint("-INT")]
fn a_stop_signal_to_the_gate_stops_its_steps_and_releases_its_lock(#[case] stop: &str) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let mut caller = merge_up_to_its_steps(&workspace, &stand_in);
    let gate = std::fs::read_to_string(gate_lock(&workspace).join("pid")).unwrap();
    let step = std::fs::read_to_string(stand_in.join("reached")).unwrap();

    signal(stop, gate.trim());

    until("the gate's lock to go", || !gate_lock(&workspace).exists());
    let code = caller.wait().unwrap().code();
    assert!(!runs(step.trim()), "the gate's step still runs");
    assert_eq!(code, Some(143));
    assert!(
        !calls(&stand_in)
            .iter()
            .any(|call| call.starts_with("gh pr merge")),
        "{:?}",
        calls(&stand_in)
    );
}

/// A gate that has merged has nothing left to stop: SIGTERM during its sweep
/// neither cuts the sweep nor keeps the lock, and the caller ends with the
/// merge's status.
#[test]
fn sigterm_during_the_sweep_leaves_the_gates_end_whole() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("sweep-hold"), "").unwrap();
    let mut caller = merge_command(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    until("the gate's sweep", || stand_in.join("sweeping").exists());
    let gate = std::fs::read_to_string(gate_lock(&workspace).join("pid")).unwrap();

    signal("-TERM", gate.trim());
    std::fs::remove_file(stand_in.join("sweep-hold")).unwrap();

    until("the gate's lock to go", || !gate_lock(&workspace).exists());
    assert_eq!(caller.wait().unwrap().code(), Some(0));
}

/// The gate starts as a background command, which ignores SIGINT and SIGQUIT,
/// from a caller that may ignore more; its steps, and so the tests' processes,
/// still get what a foreground command gets.
#[cfg(target_os = "linux")]
#[test]
fn a_gates_steps_ignore_no_signal_the_gates_start_would_have_them_ignore() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let signals = std::fs::read_to_string(stand_in.join("signals")).unwrap();
    let ignored = u64::from_str_radix(signals.trim_start_matches("SigIgn:").trim(), 16).unwrap();
    // SIGHUP, SIGINT, SIGQUIT, SIGPIPE and SIGTERM, bits 0, 1, 2, 12 and 14.
    assert_eq!(ignored & 0x5007, 0, "{signals}");
}

/// The stress run comes after the whole Pre-merge suite, and refuses a
/// selected LazyFS power cut unless `GOODFETCH_LAZYFS` names a LazyFS build
/// (two gates of 2026-11-08 died there, late). A full gate that lacks one
/// fails before any step, naming the variable. Only Linux has LazyFS.
#[cfg(target_os = "linux")]
#[rstest]
#[case::unset(None)]
#[case::no_such_file(Some("no-such-lazyfs"))]
#[case::not_executable(Some("stand-in/calls"))]
#[case::a_folder(Some("stand-in"))]
fn a_full_gate_without_a_lazyfs_build_fails_before_any_step(#[case] lazyfs: Option<&str>) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("gate"), "full").unwrap();
    let named = lazyfs.map(|path| workspace.path().join(path).display().to_string());
    let mut variables = vec![("GOODFETCH_GATE_MIN_FREE_GB", "0")];
    if let Some(named) = &named {
        variables.push(("GOODFETCH_LAZYFS", named));
    }

    let run = merge(&workspace, &stand_in, &variables);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("GOODFETCH_LAZYFS"), "{}", run.stdout);
    assert!(
        !run.calls.iter().any(|call| call.starts_with("just ")),
        "{:?}",
        run.calls
    );
    assert!(!gate_lock(&workspace).exists());
}

#[test]
fn a_full_gate_with_a_lazyfs_build_runs_its_steps() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("gate"), "full").unwrap();
    let lazyfs = stand_in.join("lazyfs");
    std::fs::write(&lazyfs, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&lazyfs, std::fs::Permissions::from_mode(0o755)).unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    call(&run, "just pre-merge all");
}

/// A stand-in LazyFS build, and the variables of a full gate that uses it.
fn full_gate_with_lazyfs(stand_in: &Path) -> PathBuf {
    std::fs::write(stand_in.join("gate"), "full").unwrap();
    let lazyfs = stand_in.join("lazyfs");
    std::fs::write(&lazyfs, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&lazyfs, std::fs::Permissions::from_mode(0o755)).unwrap();
    lazyfs
}

/// The read only marks figures accepted in the table, and no figure past its
/// threshold fails (ADR 0134): GitHub's API being down leaves the verdict to
/// the gate's steps.
#[test]
fn a_full_gate_runs_its_steps_when_the_pull_requests_acceptance_cannot_be_read() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);
    std::fs::write(stand_in.join("acceptance-unread"), "").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    call(&run, "just pre-merge all");
    call(&run, "gh pr merge");
}

/// The plan only names the earlier run the merge's message credits, and a
/// dirty tree's gate leaves none.
#[test]
fn a_green_full_gate_merges_without_a_plan_to_read() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);
    std::fs::write(stand_in.join("no-plan"), "").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    call(&run, "gh pr merge");
}

/// No step of a quick or docs gate needs LazyFS, so neither asks for it.
#[rstest]
#[case::quick("quick")]
#[case::docs("docs")]
fn a_gate_without_the_stress_run_runs_without_a_lazyfs_build(#[case] gate: &str) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("gate"), gate).unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    call(&run, &format!("just pre-merge {gate}"));
}
