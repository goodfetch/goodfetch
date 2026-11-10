//! `just merge` (`ci/merge-pr.sh`, AGENTS.md's Merging; TESTING.md's
//! Commands) around its gate: it refuses to start one while the file system
//! of the gate's `target/` is short of space (#1822), keeps a failing gate's
//! log and test results in the repository's common folder (#2087), and
//! sweeps the gate's `target/` after each gate, still holding its lock; a
//! gate waits for the run in the backstop's worktree, a test-health run or
//! the backstop's own, unless that one waits for it; it files the merge's stress
//! listings; and the gate outlives the shell that started it, whose output it
//! no longer needs. `gh`, `cargo` and `just` are stand-ins that record each call, and
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
use crate::pre_merge::runs;
use crate::workspace::Workspace;

const NUMBER: &str = "7";

/// The Unix time the `date` stand-in gives for `date -u +%s`.
const STARTED: &str = "1800000000";

/// A process ID above every system's limit, so no process has it.
pub(crate) const NO_PROCESS: &str = "2147483646";

/// Stand-ins for `gh`, `cargo` and `just` that append each call to `calls`,
/// and for `date`, which gives [`STARTED`] for `-u +%s` and leaves every
/// other format to the system's `date`.
/// `gh pr view` shows an open pull request at the head of `change`, or at the
/// commit `pushed` names once a test pushed another, in the state and draft
/// state `state` gives (`<state>`, a tab, `<true|false>`) when it exists, from
/// the head branch `branch` names, `change` without it, and `gh pr
/// merge` makes `main` a merge with the tree of the revision `merges-as` names,
/// when it exists; `cargo run … affected` prints `selection`'s content, else
/// `gate=quick` or the gate `gate` names, its sweep notes `sweeping` and waits
/// while `sweep-hold` exists, its `perf pull-request` fails while
/// `acceptance-unread` exists, and its `stress listings` writes the head's
/// listing unless `listing-fails` exists; `just pre-merge` writes the listing of
/// the base's tip while `base-listings` exists, the plan a full gate reads,
/// unless `no-plan` exists, and a JUnit file,
/// notes the signals it ignores in `signals` where the system lists them,
/// notes its process in `reached`, waits while `hold` exists, writes a line,
/// takes a second more while `lengthy` exists and, while `failing` exists,
/// leaves a red step's kept result file and fails with status 3; a `just
/// pre-merge` that starts while another runs
/// notes itself in `together`.
fn stand_ins(workspace: &Workspace, head: &str) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let calls = folder.join("calls");
    let scripts = [
        (
            "gh",
            format!(
                "#!/bin/sh\necho \"gh $*\" >>\"{calls}\"\n\
                 if [ \"$1 $2\" = \"pr view\" ]; then printf '%s\\t%s\\tmain\\t%s\\tA change\\n' \"$(cat \"{state}\" 2>/dev/null || printf 'OPEN\\tfalse')\" \"$(cat \"{pushed}\" 2>/dev/null || echo {head})\" \"$(cat \"{branch}\" 2>/dev/null || echo change)\"; fi\n\
                 if [ \"$1 $2\" = \"pr merge\" ] && [ -f \"{merges_as}\" ]; then\n\
                   merged=$(git -c user.name=GitHub -c user.email=noreply@example.test commit-tree \"$(cat \"{merges_as}\")^{{tree}}\" -p refs/heads/main -p {head} -m merged)\n\
                   git update-ref refs/heads/main \"$merged\"\n\
                 fi\n",
                calls = calls.display(),
                pushed = folder.join("pushed").display(),
                state = folder.join("state").display(),
                branch = folder.join("branch").display(),
                merges_as = folder.join("merges-as").display(),
            ),
        ),
        (
            "cargo",
            format!(
                "#!/bin/sh\necho \"cargo $* in $PWD\" >>\"{calls}\"\n\
                 case \" $* \" in\n\
                   *\" affected \"*) if [ -f \"{selection}\" ]; then cat \"{selection}\"; elif [ -f \"{gate}\" ]; then echo gate=$(cat \"{gate}\"); else echo gate=quick; fi ;;\n\
                   *\" sweep-target \"*)\n\
                     : >\"{sweeping}\"\n\
                     while [ -f \"{sweep_hold}\" ]; do sleep 0.02; done\n\
                     ;;\n\
                   *\" perf pull-request \"*) if [ -f \"{acceptance_unread}\" ]; then exit 1; fi ;;\n\
                   *\" stress listings \"*)\n\
                     if [ -f \"{listing_fails}\" ]; then exit 1; fi\n\
                     while [ $# -gt 0 ]; do if [ \"$1\" = --out ]; then out=$2; fi; shift; done\n\
                     mkdir -p \"$out/{head}\" && : >\"$out/{head}/listed\"\n\
                     ;;\n\
                 esac\n",
                calls = calls.display(),
                listing_fails = folder.join("listing-fails").display(),
                selection = folder.join("selection").display(),
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
                 if [ -n \"$GOODFETCH_PERF_PULL_REQUEST\" ]; then\n\
                   echo \"acceptance $GOODFETCH_PERF_PULL_REQUEST\" >>\"{calls}\"\n\
                 fi\n\
                 if [ \"$1\" = pre-merge ]; then\n\
                   alone=\"\"\n\
                   if mkdir \"{running}\" 2>/dev/null; then alone=1; else echo \"$*\" >>\"{together}\"; fi\n\
                   if [ -f \"{base_listings}\" ]; then\n\
                     tip=$(git rev-parse origin/main)\n\
                     mkdir -p \"target/stress-listings/$tip\" && : >\"target/stress-listings/$tip/listed\"\n\
                   fi\n\
                   mkdir -p target/gate-runs\n\
                   if [ ! -f \"{no_plan}\" ]; then echo previous=none >target/gate-runs/{head}.plan; fi\n\
                   mkdir -p target/nextest/quick && echo '<testsuites/>' >target/nextest/quick/junit.xml\n\
                   if [ -r /proc/self/status ]; then grep '^SigIgn:' /proc/self/status >\"{signals}\"; fi\n\
                   echo $$ >\"{reached}\"\n\
                   while [ -f \"{hold}\" ]; do sleep 0.02; done\n\
                   echo 'the steps ran'\n\
                   if [ -f \"{lengthy}\" ]; then sleep 1; fi\n\
                   [ -z \"$alone\" ] || rmdir \"{running}\"\n\
                   if [ -f \"{failing}\" ]; then\n\
                     mkdir -p target/red-steps/quick/nextest/quick && echo '<testsuites/>' >target/red-steps/quick/nextest/quick/junit.xml\n\
                     echo 'a test failed'; exit 3\n\
                   fi\n\
                 fi\n",
                calls = calls.display(),
                running = folder.join("running").display(),
                together = folder.join("together").display(),
                lengthy = folder.join("lengthy").display(),
                base_listings = folder.join("base-listings").display(),
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
    merge_with(workspace, stand_in, &[], variables)
}

/// Runs `merge_command` to its end with these options after the number, as
/// `just merge <number> <options>` passes them.
fn merge_with(
    workspace: &Workspace,
    stand_in: &Path,
    options: &[&str],
    variables: &[(&str, &str)],
) -> Run {
    let output = merge_command(workspace, stand_in, variables)
        .args(options)
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
pub(crate) fn until(what: &str, condition: impl Fn() -> bool) {
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
    // What each red step left when it ended, which the steps after it clear.
    assert!(
        kept[0]
            .join("red-steps/quick/nextest/quick/junit.xml")
            .is_file()
    );
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
    a_specs_pull_request_a_test_reads(&stand_in);
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

/// `xtask affected`'s selection decides which gate runs, so a selection the
/// script cannot read fails before any step: taking it for some gate would
/// merge on a gate nobody chose.
#[rstest]
#[case::no_gate("crates=a\n")]
#[case::its_gate_twice("gate=full\ngate=quick\n")]
fn a_selection_without_its_gate_or_with_it_twice_merges_nothing(#[case] selection: &str) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("selection"), selection).unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("`gate=`"), "{}", run.stdout);
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("just ") || call.starts_with("gh pr merge")),
        "{:?}",
        run.calls
    );
    assert!(!gate_lock(&workspace).exists());
}

#[test]
fn a_gate_the_script_does_not_know_merges_nothing() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("gate"), "later").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("just ") || call.starts_with("gh pr merge")),
        "{:?}",
        run.calls
    );
    assert!(!gate_lock(&workspace).exists());
}

/// The backstop tells a merge a full gate passed, which leaves a record, from a
/// quick gate's, which leaves none, by these words (`xtask gate compare`): it
/// holds them to the merge's message, as `gate::FULL_GATE_MERGE` quotes them.
#[rstest]
#[case::full(true, "Merged on the local gate: `just pre-merge all`")]
#[case::quick(false, "Merged on the local gate: `just pre-merge quick`")]
fn a_merge_says_which_gate_passed_it(#[case] full: bool, #[case] words: &str) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);
    if !full {
        std::fs::remove_file(stand_in.join("gate")).unwrap();
    }

    let run = merge(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert!(
        run.calls
            .iter()
            .any(|call| call.starts_with("gh pr merge ")
                && call.contains(&format!("--body {words}"))),
        "{:?}",
        run.calls
    );
}

/// Which gate `just merge` runs (ADR 0135's amendment of 2026-11-10, the
/// owner's decision for the refactor: quick per merge, the full gate once per
/// finished spec). `xtask affected` says what the change is; a change a test
/// reads takes the full gate only on a spec's pull request, whose head branch
/// is `spec/<n>`, or when `--full` asks for it, and the quick gate otherwise.
/// A change of documents alone keeps the docs gate, and one no test reads the
/// quick gate, on any branch.
#[rstest]
#[case::a_change_a_test_reads("full", "change", &[], "quick")]
#[case::a_specs_pull_request("full", "spec/130", &[], "all")]
#[case::a_branch_that_only_starts_like_a_specs("full", "spec/notes", &[], "quick")]
#[case::a_branch_that_only_names_a_spec("full", "claude/spec/130", &[], "quick")]
#[case::asked_for("full", "change", &["--full"], "all")]
#[case::asked_for_on_documents("docs", "change", &["--full"], "all")]
#[case::a_specs_documents("docs", "spec/130", &[], "docs")]
#[case::a_specs_change_no_test_reads("quick", "spec/130", &[], "quick")]
fn a_change_a_test_reads_takes_the_full_gate_only_on_a_specs_pull_request_or_when_asked(
    #[case] affected: &str,
    #[case] branch: &str,
    #[case] options: &[&str],
    #[case] form: &str,
) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);
    std::fs::write(stand_in.join("gate"), affected).unwrap();
    std::fs::write(stand_in.join("branch"), branch).unwrap();

    let run = merge_with(
        &workspace,
        &stand_in,
        options,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let gates: Vec<&str> = run
        .calls
        .iter()
        .filter_map(|call| call.strip_prefix("just pre-merge "))
        .collect();
    assert_eq!(gates, [form]);
}

/// A quick merge of a change a test reads is never recorded as a full one:
/// the gate's log and the merge's message name the quick gate, say why it
/// ran, and say what did not run.
#[test]
fn a_quick_merge_of_a_change_a_test_reads_says_what_it_did_not_run() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("gate"), "full").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let said = "`just pre-merge quick` (a test reads the change, and the full gate runs once per finished spec, on the pull request of its `spec/<n>` branch, or with `--full`; the Pre-merge suite, perf, the stress run, the visual tests, the Linux-only tests and the full gate's other steps did not run)";
    let merged = run
        .calls
        .iter()
        .find(|call| call.starts_with("gh pr merge "))
        .unwrap();
    assert!(
        merged.contains(&format!(
            "--body Merged on the local gate: {said} passed at {head}."
        )),
        "{merged}"
    );
    assert!(
        run.stdout.contains(&format!("The gate is {said}")),
        "{}",
        run.stdout
    );
}

/// A full gate says in its log and in the merge's message why it ran, after
/// the words the merge's record is found by.
#[rstest]
#[case::a_specs_pull_request("spec/130", &[], "(the pull request of a spec's branch, `spec/130`)")]
#[case::asked_for("change", &["--full"], "(asked for with `--full`)")]
fn a_full_gate_says_why_it_ran(#[case] branch: &str, #[case] options: &[&str], #[case] why: &str) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);
    std::fs::write(stand_in.join("branch"), branch).unwrap();

    let run = merge_with(
        &workspace,
        &stand_in,
        options,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let said = format!("`just pre-merge all` {why}");
    let merged = run
        .calls
        .iter()
        .find(|call| call.starts_with("gh pr merge "))
        .unwrap();
    assert!(
        merged.contains(&format!("--body Merged on the local gate: {said}")),
        "{merged}"
    );
    assert!(
        run.stdout.contains(&format!("The gate is {said}")),
        "{}",
        run.stdout
    );
}

/// `--full` and `--past-cap` go together in either order, and any other
/// option is refused before the gate starts.
#[rstest]
#[case::full_first(&["--full", "--past-cap"], Some(0), "all")]
#[case::past_cap_first(&["--past-cap", "--full"], Some(0), "all")]
#[case::past_cap_alone(&["--past-cap"], Some(0), "quick")]
#[case::unknown(&["--fast"], Some(2), "")]
#[case::unknown_after_a_known_one(&["--full", "--fast"], Some(2), "")]
fn the_gates_options_go_in_any_order_and_no_other_is_taken(
    #[case] options: &[&str],
    #[case] code: Option<i32>,
    #[case] form: &str,
) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);
    std::fs::write(stand_in.join("branch"), "change").unwrap();

    let run = merge_with(
        &workspace,
        &stand_in,
        options,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, code, "{}{}", run.stdout, run.stderr);
    let gates: Vec<&str> = run
        .calls
        .iter()
        .filter_map(|call| call.strip_prefix("just pre-merge "))
        .collect();
    let expected: Vec<&str> = form.split(' ').filter(|form| !form.is_empty()).collect();
    assert_eq!(gates, expected);
}

/// A quick gate's hand-over of its base's listings rests on no test reading
/// the change. A quick merge of a change a test reads may add, rename or
/// remove tests, so the base's listings are not the merge's: none is kept,
/// and the next stress run lists its base from a checkout.
#[test]
fn a_quick_merge_of_a_change_a_test_reads_hands_no_listings_to_the_merge() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("gate"), "full").unwrap();
    std::fs::write(stand_in.join("merges-as"), &head).unwrap();
    std::fs::write(stand_in.join("base-listings"), "").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert!(!listings(&workspace).exists(), "{}", run.stdout);
    assert!(
        run.stdout
            .contains("the next stress run lists its base from a checkout"),
        "{}",
        run.stdout
    );
    assert!(
        !run.calls
            .iter()
            .any(|call| call.contains("stress listings")),
        "{:?}",
        run.calls
    );
}

#[test]
fn a_full_gate_with_a_lazyfs_build_runs_its_steps() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    a_specs_pull_request_a_test_reads(&stand_in);
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

/// Makes the pull request a spec's, from its integration branch `spec/<n>`
/// (`docs/agents/issue-tracker.md`), with a change a test reads: the one kind
/// that takes the full gate unasked.
fn a_specs_pull_request_a_test_reads(stand_in: &Path) {
    std::fs::write(stand_in.join("gate"), "full").unwrap();
    std::fs::write(stand_in.join("branch"), "spec/130").unwrap();
}

/// A stand-in LazyFS build, and the variables of a full gate that uses it.
fn full_gate_with_lazyfs(stand_in: &Path) -> PathBuf {
    a_specs_pull_request_a_test_reads(stand_in);
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
    // `just perf` would fail on a file that was never written, after every
    // other host step: the gate gets no such file to read.
    assert!(
        !run.calls.iter().any(|call| call.starts_with("acceptance ")),
        "{:?}",
        run.calls
    );
}

/// A read that worked hands `just perf` the file it wrote.
#[test]
fn a_full_gate_hands_its_steps_the_pull_requests_acceptance() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);

    let run = merge(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert!(
        run.calls[call(&run, "acceptance ")].ends_with("/target/perf-pull-request.json"),
        "{:?}",
        run.calls
    );
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

/// Runs a full gate on `head` to its end, its merge a commit with `merged_as`'s
/// tree, and answers what the merge left in `main`.
fn full_gate_merging_as(workspace: &Workspace, stand_in: &Path, merged_as: &str) -> (Run, String) {
    let lazyfs = full_gate_with_lazyfs(stand_in);
    std::fs::write(stand_in.join("merges-as"), merged_as).unwrap();
    let run = merge(
        workspace,
        stand_in,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );
    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let merged = workspace.git(&["rev-parse", "refs/heads/main"]);
    (run, merged.trim().to_owned())
}

fn listings(workspace: &Workspace) -> PathBuf {
    gate_worktree(workspace).join("target/stress-listings")
}

/// The next stress run calls a test new when its base's listing lacks it, so
/// the listing of the merge's tree is filed under the merge, the base the next
/// gate reads.
#[test]
fn a_full_gate_files_its_listings_under_the_merge_it_made() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);

    let (run, merged) = full_gate_merging_as(&workspace, &stand_in, &head);

    assert_ne!(merged, head);
    assert!(
        listings(&workspace).join(&merged).join("listed").is_file(),
        "{}",
        run.stdout
    );
    assert!(!listings(&workspace).join(&head).exists());
}

/// A base that moved during the gate gives the merge a tree the listing never
/// saw: filed under it, a test only the moved base holds would read as old, and
/// no stress run would repeat it.
#[test]
fn a_merge_with_another_tree_than_the_head_gets_no_listings() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);

    let (run, _) = full_gate_merging_as(&workspace, &stand_in, "refs/heads/main");

    assert!(!listings(&workspace).exists());
    assert!(
        !run.calls
            .iter()
            .any(|call| call.contains("stress listings")),
        "{:?}",
        run.calls
    );
    assert!(
        run.stdout
            .contains("the next stress run lists its base from a checkout"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_listing_that_fails_after_the_merge_leaves_none_and_only_warns() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("listing-fails"), "").unwrap();

    let (run, _) = full_gate_merging_as(&workspace, &stand_in, &head);

    assert!(!listings(&workspace).exists());
    assert!(
        run.stdout
            .contains("the next stress run lists its base from a checkout"),
        "{}",
        run.stdout
    );
}

/// No test reads a quick gate's change, so the listing of its base is the
/// merge's too. A gate that holds none says so and files nothing.
#[rstest]
#[case::with_its_bases(true)]
#[case::without_them(false)]
fn a_quick_gate_hands_its_bases_listings_to_the_merge(#[case] saved: bool) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("merges-as"), &head).unwrap();
    if saved {
        std::fs::write(stand_in.join("base-listings"), "").unwrap();
    }

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let merged = workspace.git(&["rev-parse", "refs/heads/main"]);
    assert_eq!(
        listings(&workspace)
            .join(merged.trim())
            .join("listed")
            .is_file(),
        saved,
        "{}{}",
        run.stdout,
        run.stderr
    );
    assert_eq!(
        run.stdout
            .contains("the next stress run lists its base from a checkout"),
        !saved,
        "{}",
        run.stdout
    );
}

/// Starts `merge_command`, its output in the file it answers with.
fn merge_writing_to_a_file(workspace: &Workspace, stand_in: &Path) -> (Child, PathBuf) {
    let output = stand_in.join("caller-output");
    let caller = merge_command(workspace, stand_in, &[("GOODFETCH_GATE_MIN_FREE_GB", "0")])
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&output).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    (caller, output)
}

/// Who holds the backstop's worktree, as its lock folder says it.
#[derive(Clone, Copy)]
enum HeldBy {
    /// A test-health run, which marks the lock it takes.
    TestHealth,
    /// The backstop's own run, in its full run.
    Backstop,
    /// The backstop's own run, which lets the gate of this process, running
    /// when it started, end first.
    BackstopWaitingFor(&'static str),
}

fn backstop_lock(workspace: &Workspace) -> PathBuf {
    workspace
        .path()
        .join(".claude/worktrees/gate-backstop.lock")
}

/// Leaves the backstop's worktree held by process `holder`, as `held` says.
fn hold_the_backstops_worktree(workspace: &Workspace, holder: &str, held: HeldBy) {
    let lock = backstop_lock(workspace);
    std::fs::create_dir_all(&lock).unwrap();
    std::fs::write(lock.join("pid"), holder).unwrap();
    match held {
        HeldBy::TestHealth => std::fs::write(lock.join("test-health"), "").unwrap(),
        HeldBy::Backstop => {}
        HeldBy::BackstopWaitingFor(gate) => {
            std::fs::write(lock.join("waits-for"), gate).unwrap();
        }
    }
}

/// A process that lives until the test ends it, to hold a lock.
fn holder() -> Child {
    Command::new("sleep")
        .arg("600")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// A test-health run takes the host's cores for hours, and on 2026-11-08 a
/// gate beside one failed on the load; the backstop's full run is a second
/// gate's worth of suites (ADR 0135's amendment of 2026-11-10). The gate says
/// which it waits for, starts no step until that run ends, then runs as
/// usual. A backstop that waits for an earlier gate was there first, so the
/// gate waits for it too.
#[rstest]
#[case::a_test_health_run(HeldBy::TestHealth, "the test-health run")]
#[case::the_backstop(HeldBy::Backstop, "the backstop")]
#[case::the_backstop_waiting_for_an_earlier_gate(
    HeldBy::BackstopWaitingFor(NO_PROCESS),
    "the backstop"
)]
fn a_gate_waits_for_the_run_in_the_backstops_worktree_and_says_so(
    #[case] held: HeldBy,
    #[case] named: &str,
) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let mut run = holder();
    hold_the_backstops_worktree(&workspace, &run.id().to_string(), held);
    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);
    let waiting = format!("Waiting for {named}, process {}", run.id());

    until("the gate's word that it waits", || {
        std::fs::read_to_string(&output).is_ok_and(|text| text.contains(&waiting))
    });

    let before_the_end = calls(&stand_in);
    run.kill().unwrap();
    run.wait().unwrap();
    until_the_gate_merged(&workspace, &stand_in);
    assert!(
        !before_the_end.iter().any(|call| call.starts_with("just ")),
        "{before_the_end:?}"
    );
    assert!(caller.wait().unwrap().success());
}

/// A backstop that found this gate running lets it end first and says so in
/// its lock, so the two never wait for each other: the gate goes on while the
/// backstop still holds its worktree.
#[test]
fn a_gate_goes_first_once_the_backstop_says_it_waits_for_it() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let mut backstop = holder();
    hold_the_backstops_worktree(&workspace, &backstop.id().to_string(), HeldBy::Backstop);
    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);
    until("the gate's word that it waits", || {
        std::fs::read_to_string(&output).is_ok_and(|text| text.contains("Waiting for the backstop"))
    });

    let gate = std::fs::read_to_string(gate_lock(&workspace).join("pid")).unwrap();
    std::fs::write(backstop_lock(&workspace).join("waits-for"), gate.trim()).unwrap();

    until_the_gate_merged(&workspace, &stand_in);
    assert!(caller.wait().unwrap().success());
    assert!(runs(&backstop.id().to_string()));
    backstop.kill().unwrap();
    backstop.wait().unwrap();
}

/// The text of every log in `folders`, each under its path.
fn logs_of(folders: &[PathBuf]) -> String {
    use std::fmt::Write as _;
    let logs = folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flat_map(|entries| entries.filter_map(Result::ok).map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"));
    let mut text = String::new();
    for path in logs {
        let log = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = writeln!(text, "== {}\n{log}", path.display());
    }
    text
}

/// A gate and the backstop started at the same moment never run their suites
/// together, whichever takes its lock first, and neither waits for the other
/// for ever: each has its own lock before it reads the other's, so at least
/// one sees the other, and a backstop that sees the gate lets it go first.
/// Both real scripts run here, with one or the other started first, each
/// `just pre-merge` taking a second, and each lock naming when its holder
/// started, as the scripts write it.
#[rstest]
#[case::the_backstop_first(true)]
#[case::the_gate_first(false)]
fn a_gate_and_the_backstop_started_together_never_run_their_suites_together(
    #[case] backstop_first: bool,
) {
    let workspace = crate::gate_backstop::backstop_repository();
    // Ignored, as the repository's `.gitignore` ignores it, so each
    // worktree is a clean checkout again at its next run.
    workspace.write(".git/info/exclude", b"/target/\n");
    workspace.git(&["merge", "--quiet", "--no-edit", "main"]);
    workspace.write("change.txt", b"a change\n");
    commit_all(&workspace, "A change");
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.git(&["update-ref", &format!("refs/pull/{NUMBER}/head"), &head]);
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("lengthy"), "").unwrap();
    workspace.write(
        "runner-temp/goodfetch-os-services/env",
        b"DBUS_SESSION_BUS_ADDRESS=unix:path=/stand-in/bus\n",
    );
    let backstop = || {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/gate-backstop.sh");
        let mut command = workspace.command(script);
        command
            .arg("origin/main")
            .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
            .env("RUNNER_TEMP", workspace.path().join("runner-temp"))
            .env("GOODFETCH_GATE_BACKSTOP_LOG", workspace.path().join("logs"))
            .env("GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB", "0")
            .env("GOODFETCH_LAZYFS", stand_in.join("just"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().unwrap()
    };
    let gate = || {
        merge_command(
            &workspace,
            &stand_in,
            &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
    };

    let (mut first, mut second) = if backstop_first {
        (backstop(), gate())
    } else {
        (gate(), backstop())
    };
    let statuses = [first.wait().unwrap(), second.wait().unwrap()];

    assert!(
        statuses.iter().all(std::process::ExitStatus::success),
        "{statuses:?}\n{}",
        logs_of(&[
            workspace.path().join("logs"),
            workspace.path().join(".git/goodfetch-gate-logs")
        ])
    );
    let mut suites: Vec<String> = calls(&stand_in)
        .into_iter()
        .filter(|call| call.starts_with("just pre-merge "))
        .collect();
    suites.sort();
    assert_eq!(suites, ["just pre-merge all", "just pre-merge quick"]);
    let together = std::fs::read_to_string(stand_in.join("together")).unwrap_or_default();
    assert_eq!(together, "", "a suite ran while the other ran");
}

/// A gate behind a backstop can wait for hours, and its pull request may be
/// pushed meanwhile. The gate reads the pull request once its wait has ended,
/// so it judges and merges the head the pull request then has, not one read
/// before the wait, which `gh pr merge` would refuse after a whole gate.
#[test]
fn a_gate_judges_the_head_its_pull_request_has_once_its_wait_ends() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let mut backstop = holder();
    hold_the_backstops_worktree(&workspace, &backstop.id().to_string(), HeldBy::Backstop);
    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);
    until("the gate's word that it waits", || {
        std::fs::read_to_string(&output).is_ok_and(|text| text.contains("Waiting for the backstop"))
    });
    workspace.write("change.txt", b"a later change\n");
    workspace.git(&["add", "change.txt"]);
    workspace.git(&["commit", "--quiet", "--message", "A later change"]);
    let pushed = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.git(&["update-ref", &format!("refs/pull/{NUMBER}/head"), &pushed]);
    std::fs::write(stand_in.join("pushed"), &pushed).unwrap();

    backstop.kill().unwrap();
    backstop.wait().unwrap();
    until_the_gate_merged(&workspace, &stand_in);

    assert!(caller.wait().unwrap().success());
    let merged: Vec<String> = calls(&stand_in)
        .into_iter()
        .filter(|call| call.starts_with("gh pr merge"))
        .collect();
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert!(
        merged[0].contains(&format!("--match-head-commit {pushed}")),
        "{merged:?}"
    );
    let said = std::fs::read_to_string(&output).unwrap();
    assert!(
        said.contains(&format!(
            "Running the local gate (quick) on #{NUMBER} at {pushed}"
        )),
        "{said}"
    );
}

/// A lock names when its holder started, beside its process ID. A process
/// that took a dead run's number, as one may after a reboot, started at
/// another time: it holds no gate up, where the number alone would have kept
/// the gate waiting for ever.
#[test]
fn a_process_that_took_a_dead_runs_number_holds_no_gate_up() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    hold_the_backstops_worktree(
        &workspace,
        &std::process::id().to_string(),
        HeldBy::Backstop,
    );
    std::fs::write(
        backstop_lock(&workspace).join("started"),
        "Thu Jan  1 00:00:00 1970\n",
    )
    .unwrap();

    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);

    until_the_gate_merged(&workspace, &stand_in);
    assert!(caller.wait().unwrap().success());
    let said = std::fs::read_to_string(&output).unwrap();
    assert!(!said.contains("Waiting for"), "{said}");
}

/// A lock that names its holder's true start, as the scripts write it, reads
/// as held: here the real `gate_worktree_lock` takes the backstop's worktree
/// for a process that then lives on, and the gate waits for that process.
#[test]
fn a_lock_naming_its_holders_true_start_holds_a_gate_up() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let take_the_lock = format!(
        "gate_worktree_name=gate-backstop; source '{}'; gate_worktree_lock; exec sleep 600",
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ci/gate-worktree.sh")
            .display()
    );
    let mut backstop = workspace
        .command("bash")
        .args(["-c", &take_the_lock])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    until("the backstop's lock", || {
        std::fs::read_to_string(backstop_lock(&workspace).join("pid"))
            .is_ok_and(|process| process.trim() == backstop.id().to_string())
    });
    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);
    let waiting = format!("Waiting for the backstop, process {}", backstop.id());

    until("the gate's word that it waits", || {
        std::fs::read_to_string(&output).is_ok_and(|text| text.contains(&waiting))
    });

    backstop.kill().unwrap();
    backstop.wait().unwrap();
    until_the_gate_merged(&workspace, &stand_in);
    assert!(caller.wait().unwrap().success());
}

/// A start that cannot be told, on either side, makes no lock stale: a live
/// lock read as stale would let two suites run together unseen, while a dead
/// one read as live makes a caller wait in sight, and its holder's number is
/// checked again each second. Only two readings that differ are another
/// process. Here the lock names no start, and the gate still waits.
#[test]
fn a_lock_whose_holders_start_cannot_be_told_is_judged_by_its_process() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let mut backstop = holder();
    hold_the_backstops_worktree(&workspace, &backstop.id().to_string(), HeldBy::Backstop);
    std::fs::write(backstop_lock(&workspace).join("started"), "").unwrap();
    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);
    let waiting = format!("Waiting for the backstop, process {}", backstop.id());

    until("the gate's word that it waits", || {
        std::fs::read_to_string(&output).is_ok_and(|text| text.contains(&waiting))
    });

    backstop.kill().unwrap();
    backstop.wait().unwrap();
    until_the_gate_merged(&workspace, &stand_in);
    assert!(caller.wait().unwrap().success());
}

/// A pull request the gate will refuse is refused at once, before the gate
/// takes its lock and waits: behind a backstop the wait can last hours, and a
/// refusal should cost nothing. The gate reads the pull request again once a
/// wait has ended.
#[rstest]
#[case::closed("CLOSED\tfalse", "is CLOSED, not open")]
#[case::a_draft("OPEN\ttrue", "is a draft")]
fn a_pull_request_the_gate_refuses_is_refused_before_any_wait(
    #[case] state: &str,
    #[case] refusal: &str,
) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("state"), state).unwrap();
    let mut backstop = holder();
    hold_the_backstops_worktree(&workspace, &backstop.id().to_string(), HeldBy::Backstop);

    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);
    until("the gate's refusal", || {
        std::fs::read_to_string(&output).is_ok_and(|text| text.contains(refusal))
    });
    let code = caller.wait().unwrap().code();
    let backstop_still_runs = runs(&backstop.id().to_string());
    backstop.kill().unwrap();
    backstop.wait().unwrap();

    assert_eq!(code, Some(1));
    assert!(backstop_still_runs);
    let said = std::fs::read_to_string(&output).unwrap();
    assert!(!said.contains("Waiting for"), "{said}");
    assert!(!gate_lock(&workspace).exists());
}

/// A run that died left its lock and its marks behind: it holds no gate up,
/// and the gate does not say it waits.
#[rstest]
#[case::a_test_health_run(HeldBy::TestHealth)]
#[case::the_backstop(HeldBy::Backstop)]
#[case::the_backstop_while_it_waited(HeldBy::BackstopWaitingFor(NO_PROCESS))]
fn a_gate_waits_for_no_run_that_died(#[case] held: HeldBy) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    hold_the_backstops_worktree(&workspace, NO_PROCESS, held);

    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);

    until_the_gate_merged(&workspace, &stand_in);
    assert!(caller.wait().unwrap().success());
    let said = std::fs::read_to_string(&output).unwrap();
    assert!(!said.contains("Waiting for"), "{said}");
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
