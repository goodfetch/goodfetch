//! `just merge` (`ci/merge-pr.sh`, AGENTS.md's Merging; TESTING.md's
//! Commands) around its gate: it refuses to start one while the file system
//! of the gate's `target/` is short of space (#1822), keeps a failing gate's
//! log and test results in the repository's common folder (#2087), and
//! sweeps the gate's `target/` after each gate, still holding its lock; it
//! gates a head without its base's tip merged with that tip, and refuses one
//! that conflicts with it, and one whose merge would close other issues
//! than its `Closes` lines name (#2249); a
//! gate waits for the run in the backstop's worktree, a test-health run or
//! the backstop's own, unless that one waits for it; it files the merge's stress
//! listings; it reads the hosted tier's records of a same-repository pull
//! request, waits for a hosted run still going, cancels one past its cap,
//! ends red on a red hosted run, and reads no fork's runs (#3377); and the
//! gate outlives the shell that started it, whose output it no longer
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
use crate::pre_merge::runs;
use crate::workspace::{Started, Workspace};

const NUMBER: &str = "7";

/// The Unix time the `date` stand-in gives for `date -u +%s`.
const STARTED: &str = "1800000000";

/// A process ID above every system's limit, so no process has it.
pub(crate) const NO_PROCESS: &str = "2147483646";

/// The `gh` stand-in of [`stand_ins`]: it reads the pull request from the
/// folder's files and answers the query of a spec's sub-issues and the paged
/// query of what the merge closes.
fn gh_stand_in(folder: &Path, calls: &Path, head: &str) -> String {
    let viewed = format!(
        "#!/bin/sh\necho \"gh $*\" >>\"{calls}\"\n\
         if [ \"$1 $2\" = \"pr view\" ]; then printf '%s\\t%s\\t%s\\tmain\\t%s\\tA change\\n' \"$(cat \"{state}\" 2>/dev/null || printf 'OPEN\\tfalse')\" \"$(cat \"{fork}\" 2>/dev/null || echo false)\" \"$(cat \"{pushed}\" 2>/dev/null || echo {head})\" \"$(cat \"{branch}\" 2>/dev/null || echo change)\"; fi\n",
        calls = calls.display(),
        state = folder.join("state").display(),
        fork = folder.join("fork").display(),
        pushed = folder.join("pushed").display(),
        branch = folder.join("branch").display(),
    );
    viewed
        + &hosted_stand_in(&folder.join("hosted"))
        + &format!(
            "if [ \"$1 $2\" = \"api graphql\" ]; then\n\
           case \" $* \" in\n\
             *\" --paginate \"*)\n\
               if [ -f \"{closing_unreadable}\" ]; then exit 1; fi\n\
               printf 'default\\t%s\\n' \"$(cat \"{default_branch}\" 2>/dev/null || echo main)\"\n\
               if [ -f \"{closes}\" ]; then awk '{{ print \"closes\\t\" $0 }}' \"{closes}\"; fi\n\
               if [ -f \"{body}\" ]; then awk '{{ print \"body\\t\" $0 }}' \"{body}\"; fi\n\
               ;;\n\
             *)\n\
               if [ -f \"{unreadable}\" ]; then exit 1; fi\n\
               cat \"{left}\" 2>/dev/null || echo 0\n\
               ;;\n\
           esac\n\
         fi\n\
         if [ \"$1 $2\" = \"pr merge\" ] && [ -f \"{merges_as}\" ]; then\n\
           merged=$(git -c user.name=GitHub -c user.email=noreply@example.test commit-tree \"$(cat \"{merges_as}\")^{{tree}}\" -p refs/heads/main -p {head} -m merged)\n\
           git update-ref refs/heads/main \"$merged\"\n\
         fi\n",
            merges_as = folder.join("merges-as").display(),
            left = folder.join("sub-issues-left").display(),
            unreadable = folder.join("sub-issues-unreadable").display(),
            closing_unreadable = folder.join("closing-unreadable").display(),
            default_branch = folder.join("default-branch").display(),
            closes = folder.join("closes").display(),
            body = folder.join("body").display(),
        )
}

/// The part of the `gh` stand-in that answers `ci/hosted-records.sh` from
/// `hosted`, a scripted Actions API: the repository is
/// `goodfetch/goodfetch`; the pull-request workflow is active while `active`
/// exists; the n-th listing of its runs filters `runs.<n>.json` when it
/// exists, else `runs.json`, through the script's own filter; a run's jobs
/// are `jobs.json`'s; `gh run cancel` fails while `cancel-refused` exists,
/// and else makes `runs.cancelled.json` the listing when it exists; `gh run
/// download <id>` copies `artifacts/<id>/` into its `--dir`, and fails
/// without it.
fn hosted_stand_in(hosted: &Path) -> String {
    format!(
        "if [ \"$1\" = api ]; then\n\
           case $2 in\n\
             'repos/{{owner}}/{{repo}}') echo {REPOSITORY} ;;\n\
             'repos/{{owner}}/{{repo}}/actions/workflows/pull-request.yml')\n\
               if [ -f \"{hosted}/active\" ]; then echo active; else echo disabled_manually; fi ;;\n\
             'repos/{{owner}}/{{repo}}/actions/workflows/pull-request.yml/runs?'*)\n\
               echo listed >>\"{hosted}/listings\"\n\
               listing=\"{hosted}/runs.$(wc -l <\"{hosted}/listings\" | tr -d ' ').json\"\n\
               [ -f \"$listing\" ] || listing=\"{hosted}/runs.json\"\n\
               jq -r \"$4\" \"$listing\" ;;\n\
             'repos/{{owner}}/{{repo}}/actions/runs/'*) jq -r \"$4\" \"{hosted}/jobs.json\" ;;\n\
           esac\n\
         fi\n\
         if [ \"$1 $2\" = \"run cancel\" ]; then\n\
           if [ -f \"{hosted}/cancel-refused\" ]; then exit 1; fi\n\
           if [ -f \"{hosted}/runs.cancelled.json\" ]; then\n\
             rm -f \"{hosted}\"/runs.[0-9]*.json\n\
             cp \"{hosted}/runs.cancelled.json\" \"{hosted}/runs.json\"\n\
           fi\n\
         fi\n\
         if [ \"$1 $2\" = \"run download\" ]; then\n\
           artifacts=\"{hosted}/artifacts/$3\"\n\
           while [ $# -gt 0 ]; do if [ \"$1\" = --dir ]; then into=$2; fi; shift; done\n\
           [ -d \"$artifacts\" ] || exit 1\n\
           mkdir -p \"$into\" && cp -R \"$artifacts/.\" \"$into/\"\n\
         fi\n",
        hosted = hosted.display(),
    )
}

/// The repository the `gh` stand-in names.
const REPOSITORY: &str = "goodfetch/goodfetch";

/// Stand-ins for `gh`, `cargo` and `just` that append each call to `calls`,
/// and for `date`, which gives [`STARTED`] for `-u +%s` and leaves every
/// other format to the system's `date`.
/// `gh pr view` shows an open pull request at the head of `change`, or at the
/// commit `pushed` names once a test pushed another, in the state and draft
/// state `state` gives (`<state>`, a tab, `<true|false>`) when it exists, from
/// the head branch `branch` names, `change` without it, and from a fork while
/// `fork` holds `true`; the Actions API answers from `hosted/`
/// ([`hosted_stand_in`]); `gh api graphql`, the
/// query of the spec's sub-issues left open after the pull request, prints
/// `sub-issues-left`'s content, `0` without it, and fails while
/// `sub-issues-unreadable` exists; `gh api graphql --paginate`, the query of
/// what the merge closes, answers as its filter does, a line `default`, the
/// repository's default branch (`default-branch`'s content, `main` without
/// it), a line `closes` for each line of `closes`, the issues GitHub would
/// close, and a line `body` for each line of `body`, the pull request's body,
/// each kind and its value parted by a tab, and fails while
/// `closing-unreadable` exists; and `gh pr
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
/// notes itself in `together`; and each `just` call given hosted records
/// notes their repository and files.
fn stand_ins(workspace: &Workspace, head: &str) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let calls = folder.join("calls");
    let scripts = [
        ("gh", gh_stand_in(&folder, &calls, head)),
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
                "#!/bin/sh\n\
                 echo \"just $*\" >>\"{calls}\"\n\
                 if [ -n \"$GOODFETCH_PERF_PULL_REQUEST\" ]; then\n\
                   echo \"acceptance $GOODFETCH_PERF_PULL_REQUEST\" >>\"{calls}\"\n\
                 fi\n\
                 if [ -n \"$GOODFETCH_GATE_HOSTED_RECORDS\" ]; then\n\
                   echo \"hosted records of $GOODFETCH_GATE_REPOSITORY:$(cd \"$GOODFETCH_GATE_HOSTED_RECORDS\" && find . -type f | sort | tr '\\n' ' ')\" >>\"{calls}\"\n\
                 fi\n\
                 if [ \"$1\" = pre-merge ]; then\n\
                   alone=\"\"\n\
                   if mkdir \"{running}\" 2>/dev/null; then alone=1; else echo \"$*\" >>\"{together}\"; fi\n\
                   if [ -f \"{base_listings}\" ]; then\n\
                     tip=$(git rev-parse origin/main)\n\
                     mkdir -p \"target/stress-listings/$tip\" && : >\"target/stress-listings/$tip/listed\"\n\
                   fi\n\
                   records=\"$(git rev-parse --path-format=absolute --git-common-dir)/goodfetch-gate-runs\"\n\
                   mkdir -p \"$records\"\n\
                   if [ ! -f \"{no_plan}\" ]; then echo previous=none >\"$records/{head}.plan\"; fi\n\
                   mkdir -p target/nextest/quick && echo '<testsuites/>' >target/nextest/quick/junit.xml\n\
                   if [ -r /proc/self/status ]; then grep '^SigIgn:' /proc/self/status >\"{signals}\"; fi\n\
                   echo $$ >\"{reached}\"\n\
                   while [ -f \"{hold}\" ]; do sleep 0.02; done\n\
                   echo 'the steps ran'\n\
                   if [ -f \"{lengthy}\" ]; then sleep 1; fi\n\
                   [ -z \"$alone\" ] || rmdir \"{running}\"\n\
                   if [ -f \"{failing}\" ]; then\n\
                     mkdir -p target/red-steps/quick/nextest/quick && echo '<testsuites/>' >target/red-steps/quick/nextest/quick/junit.xml\n\
                     mkdir -p target/steps/quick && echo 'a lint failed' >target/steps/quick/lint.log\n\
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

/// What a quick merge of a change a test reads says of its gate, in the
/// gate's log and in the merge's message.
const A_TEST_READS_THE_CHANGE: &str = "`just pre-merge quick` (a test reads the change, and the full gate runs only once per finished spec, on the pull request of its `spec/<n>` branch that leaves none of its sub-issues open; the Pre-merge suite, perf, the stress run, the visual tests, the Linux-only tests and the full gate's other steps did not run)";

/// Lands a pull request on `main` as `just merge` does when a test reads its
/// change and it merged on the quick gate: a merge commit whose message says
/// so.
fn land_on_the_quick_gate(workspace: &Workspace) {
    let main = "refs/heads/main";
    let tree = format!("{main}^{{tree}}");
    let change = workspace.git(&["commit-tree", &tree, "-p", main, "-m", "A change"]);
    let change = change.trim();
    let message = format!(
        "A change (#1)\n\nMerged on the local gate: {A_TEST_READS_THE_CHANGE} passed at {change}.\n\nSigned-off-by: Fixture <fixture@example.invalid>"
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

/// A repository whose `origin` is itself, with pull request #7 at a commit
/// on `change` that contains `main`; returns it with that head.
fn pull_request() -> (Workspace, String) {
    pull_request_after(0)
}

/// [`pull_request`], on a `main` that first took this many quick merges of
/// changes a test reads.
fn pull_request_after(quick_merges: usize) -> (Workspace, String) {
    let workspace = repository();
    let path = workspace.path().display().to_string();
    workspace.git(&["remote", "add", "origin", &path]);
    for _ in 0..quick_merges {
        land_on_the_quick_gate(&workspace);
    }
    workspace.git(&["merge", "--quiet", "--ff-only", "main"]);
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

pub(crate) fn gate_lock(workspace: &Workspace) -> PathBuf {
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

/// The log of the workspace's one gate.
fn gate_log(workspace: &Workspace) -> String {
    let logs: Vec<PathBuf> = std::fs::read_dir(workspace.path().join(".git/goodfetch-gate-logs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect();
    assert_eq!(logs.len(), 1, "{logs:?}");
    std::fs::read_to_string(&logs[0]).unwrap()
}

/// No gate starts a run as it ends, merged or red: what a quick merge skips
/// runs on the finished spec's full gate alone (the owner's decision of
/// 2026-11-18, ADR 0135).
#[rstest]
#[case::merged(false, 0)]
#[case::red(true, 3)]
fn a_gate_starts_no_post_merge_run_as_it_ends(#[case] failing: bool, #[case] code: i32) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    if failing {
        std::fs::write(stand_in.join("failing"), "").unwrap();
    }

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(code), "{}{}", run.stdout, run.stderr);
    let said = gate_log(&workspace);
    assert!(!said.contains("post-merge run"), "{said}");
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
    // Each red step's own log, which the gate's summary names.
    assert_eq!(
        std::fs::read_to_string(kept[0].join("steps/quick/lint.log")).unwrap(),
        "a lint failed\n"
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
    let said = gate_log(&workspace);
    assert!(said.contains("The gate was told to stop"), "{said}");
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
    assert!(run.stdout.contains("gave the gate later"), "{}", run.stdout);
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("just ") || call.starts_with("gh pr merge")),
        "{:?}",
        run.calls
    );
    assert!(!gate_lock(&workspace).exists());
}

/// Which gate `just merge` runs (ADR 0135's amendment of 2026-11-10, the
/// owner's decision for the refactor: quick per merge, the full gate once per
/// finished spec; its note of 2026-11-13: only the pull request that finishes
/// the spec; and its note of 2026-11-14: no other trigger). `xtask affected`
/// says what the change is; a change a test reads takes the full gate only on
/// a spec's pull request, whose head branch is `spec/<n>`, when none of the
/// spec's sub-issues stays open once the issues it closes are closed, and the
/// quick gate on every other. A change of documents alone keeps the docs
/// gate, and one no test reads the quick gate, on any branch. Only the full
/// gate's stress run needs a LazyFS build, so the quick and docs gates run
/// without one.
#[rstest]
#[case::a_change_a_test_reads("full", "change", "0", "quick")]
#[case::a_finished_specs_pull_request("full", "spec/130", "0", "all")]
#[case::an_unfinished_specs_pull_request("full", "spec/130", "2", "quick")]
#[case::a_branch_that_only_starts_like_a_specs("full", "spec/notes", "0", "quick")]
#[case::a_branch_that_only_names_a_spec("full", "claude/spec/130", "0", "quick")]
#[case::a_specs_documents("docs", "spec/130", "0", "docs")]
#[case::a_specs_change_no_test_reads("quick", "spec/130", "0", "quick")]
fn a_change_a_test_reads_takes_the_full_gate_only_on_a_finished_specs_pull_request(
    #[case] affected: &str,
    #[case] branch: &str,
    #[case] sub_issues_left: &str,
    #[case] form: &str,
) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);
    std::fs::write(stand_in.join("gate"), affected).unwrap();
    std::fs::write(stand_in.join("branch"), branch).unwrap();
    std::fs::write(stand_in.join("sub-issues-left"), sub_issues_left).unwrap();
    let mut variables = vec![("GOODFETCH_GATE_MIN_FREE_GB", "0")];
    if form == "all" {
        variables.push(("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()));
    }

    let run = merge(&workspace, &stand_in, &variables);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(forms(&run), [form], "{}", run.stdout);
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
    let said = A_TEST_READS_THE_CHANGE;
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

/// A gate says in its log and in the merge's message which gate ran and why.
/// The backstop tells a merge a full gate passed, which leaves a record, from a
/// quick gate's, which leaves none, by the message's first words (`xtask gate
/// compare`, as `gate::FULL_GATE_MERGE` quotes them), so the reason comes
/// after them.
#[rstest]
#[case::a_finished_specs_pull_request(
    "full",
    "spec/130",
    "`just pre-merge all` (the pull request that finishes spec #130, from `spec/130`)"
)]
#[case::a_change_no_test_reads(
    "quick",
    "change",
    "`just pre-merge quick` (no test, benchmark, model, container or Evaluation reads the change)"
)]
fn a_gate_says_which_gate_ran_and_why(
    #[case] affected: &str,
    #[case] branch: &str,
    #[case] said: &str,
) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let lazyfs = full_gate_with_lazyfs(&stand_in);
    std::fs::write(stand_in.join("gate"), affected).unwrap();
    std::fs::write(stand_in.join("branch"), branch).unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
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

/// A spec's pull request that leaves sub-issues of its spec open takes the
/// quick gate (the owner's rule of 2026-11-13, after three full gates on an
/// unfinished spec's pull request), and its gate's log and its merge's
/// message say why, after the words a quick merge of a change a test reads
/// opens with. The one query names the spec and the pull request.
#[test]
fn an_unfinished_specs_pull_request_says_why_it_took_the_quick_gate() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    a_specs_pull_request_a_test_reads(&stand_in);
    std::fs::write(stand_in.join("sub-issues-left"), "1\n").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(forms(&run), ["quick"], "{}", run.stdout);
    let why = "spec #130 is not finished: 1 sub-issue stays open after this pull request";
    let merged = run
        .calls
        .iter()
        .find(|call| call.starts_with("gh pr merge "))
        .unwrap();
    assert!(
        merged.contains(
            "--body Merged on the local gate: `just pre-merge quick` (a test reads the change"
        ) && merged.contains(why),
        "{merged}"
    );
    assert!(run.stdout.contains(why), "{}", run.stdout);
    let queries: Vec<&String> = run
        .calls
        .iter()
        .filter(|call| call.starts_with("gh api graphql ") && !call.contains("--paginate"))
        .collect();
    assert_eq!(queries.len(), 1, "{:?}", run.calls);
    assert!(
        queries[0].contains("-F spec=130") && queries[0].contains(&format!("-F pr={NUMBER}")),
        "{}",
        queries[0]
    );
}

/// Whether a spec's pull request finishes the spec decides whether it takes
/// the full gate, so a query that fails, or answers with no whole number,
/// merges nothing: taken for none left, it would run a full gate on an
/// unfinished spec; taken for some, it would skip a finished spec's.
#[rstest]
#[case::the_query_fails(None)]
#[case::it_gives_nothing(Some(""))]
#[case::it_gives_no_whole_number(Some("more than 100"))]
fn a_specs_sub_issues_that_cannot_be_read_merge_nothing(#[case] answer: Option<&str>) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    a_specs_pull_request_a_test_reads(&stand_in);
    match answer {
        Some(answer) => std::fs::write(stand_in.join("sub-issues-left"), answer).unwrap(),
        None => std::fs::write(stand_in.join("sub-issues-unreadable"), "").unwrap(),
    }

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("sub-issues of spec #130")
            && run.stdout.contains("run the command again")
            && !run.stdout.contains("--full"),
        "{}",
        run.stdout
    );
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("just ") || call.starts_with("gh pr merge")),
        "{:?}",
        run.calls
    );
    assert!(!gate_lock(&workspace).exists());
}

/// No count sends a change to the full gate (ADR 0135's note of 2026-11-14,
/// the owner's rule « FULL GATE seulement en fermant une spec entière », which
/// ended the tenth change's full gate of its amendment of 2026-11-10): however
/// many changes a test reads merged on the quick gate before it, a change a
/// test reads that finishes no spec takes the quick gate, from a branch that
/// is no spec's or from an unfinished spec's, and its gate's log and its
/// merge's message announce no full gate to come.
#[rstest]
#[case::after_nine_such_merges(9, "change", "0")]
#[case::after_twelve_such_merges(12, "change", "0")]
#[case::an_unfinished_specs_after_nine(9, "spec/130", "2")]
fn no_count_of_quick_merges_sends_a_change_to_the_full_gate(
    #[case] merged_on_the_quick_gate: usize,
    #[case] branch: &str,
    #[case] sub_issues_left: &str,
) {
    let (workspace, head) = pull_request_after(merged_on_the_quick_gate);
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("gate"), "full").unwrap();
    std::fs::write(stand_in.join("branch"), branch).unwrap();
    std::fs::write(stand_in.join("sub-issues-left"), sub_issues_left).unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(forms(&run), ["quick"], "{}", run.stdout);
    let merged = run
        .calls
        .iter()
        .find(|call| call.starts_with("gh pr merge "))
        .unwrap();
    assert!(!merged.contains("tenth"), "{merged}");
    assert!(!run.stdout.contains("tenth"), "{}", run.stdout);
}

/// The forms of `just pre-merge` a gate ran, in order.
fn forms(run: &Run) -> Vec<&str> {
    run.calls
        .iter()
        .filter_map(|call| call.strip_prefix("just pre-merge "))
        .collect()
}

/// `--past-cap` is the one option, and any other is refused before the gate
/// starts: `--full` too, since only a finished spec's pull request takes the
/// full gate (ADR 0135's note of 2026-11-14).
#[rstest]
#[case::past_cap(&["--past-cap"], Some(0), "quick")]
#[case::full(&["--full"], Some(2), "")]
#[case::full_after_past_cap(&["--past-cap", "--full"], Some(2), "")]
#[case::unknown(&["--fast"], Some(2), "")]
#[case::unknown_after_a_known_one(&["--past-cap", "--fast"], Some(2), "")]
fn past_cap_is_the_gates_one_option(
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

/// Makes the pull request a spec's, from its integration branch `spec/<n>`
/// (`docs/agents/issue-tracker.md`), with a change a test reads: the one kind
/// that takes the full gate unasked, when it finishes the spec, as the `gh`
/// stand-in answers by default.
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
    // The gate builds the head's lockfile as committed, never one Cargo
    // rewrote.
    let unlocked: Vec<&String> = run
        .calls
        .iter()
        .filter(|call| call.starts_with("cargo ") && !call.contains(" --locked "))
        .collect();
    assert!(unlocked.is_empty(), "{unlocked:?}");
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

/// The nightly fuzzing gives way to a gate (ci/fuzz-nightly.sh): the gate
/// tells the job holding its lock to stop and starts no step until it ended,
/// however long its stop takes.
#[test]
fn a_gate_stops_the_nightly_fuzzing_and_waits_for_its_stop() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let hold = stand_in.join("fuzzing-hold");
    let stopping = stand_in.join("fuzzing-stopping");
    std::fs::write(&hold, "").unwrap();
    let mut fuzzing = Started::spawn(
        Command::new("bash")
            .arg("-c")
        .arg(format!(
            r#"trap 'echo >"{stopping}"; while [ -f "{hold}" ]; do sleep 0.02; done; exit 143' TERM
while :; do sleep 0.02; done"#,
            stopping = stopping.display(),
            hold = hold.display(),
        ))
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
    let lock = workspace.path().join(".claude/worktrees/fuzz-nightly.lock");
    std::fs::create_dir_all(&lock).unwrap();
    std::fs::write(lock.join("pid"), fuzzing.id().to_string()).unwrap();
    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);

    until("the fuzzing's stop", || stopping.exists());
    until("the gate's word that it stops the fuzzing", || {
        std::fs::read_to_string(&output).is_ok_and(|text| {
            text.contains(&format!(
                "Stopping the nightly fuzzing, process {}",
                fuzzing.id()
            ))
        })
    });
    let before_the_end = calls(&stand_in);
    std::fs::remove_file(&hold).unwrap();
    let ended = fuzzing.wait();

    until_the_gate_merged(&workspace, &stand_in);
    assert_eq!(ended.code(), Some(143));
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
#[case::closed("state", "CLOSED\tfalse", "is CLOSED, not open")]
#[case::a_draft("state", "OPEN\ttrue", "is a draft")]
#[case::closing_an_issue_no_line_names("closes", "12\n", "#12: GitHub closes it")]
fn a_pull_request_the_gate_refuses_is_refused_before_any_wait(
    #[case] file: &str,
    #[case] content: &str,
    #[case] refusal: &str,
) {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join(file), content).unwrap();
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
    assert!(!said.contains("Stopped"), "{said}");
}

/// [`pull_request`] whose `main` then took a commit that writes `content` to
/// `file`, so the head no longer holds the base's tip; the checkout is left
/// on no branch, as the gate host's never has the pull request's branch out,
/// and with an identity of its own, as the gate host's has. Returns the
/// repository, the head and `main`'s tip.
fn pull_request_behind_main(file: &str, content: &[u8]) -> (Workspace, String, String) {
    let (workspace, head) = pull_request();
    workspace.git(&["config", "user.name", "Gate host"]);
    workspace.git(&["config", "user.email", "gate@example.invalid"]);
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.write(file, content);
    commit_all(&workspace, "Another change");
    let tip = workspace.git(&["rev-parse", "main"]).trim().to_owned();
    workspace.git(&["switch", "--quiet", "--detach"]);
    (workspace, head, tip)
}

/// A pull request whose head lacks its base's tip is gated on, and merged
/// at, the merge of its head and that tip, which the gate makes and pushes
/// to the pull request's branch: no one has to merge `main` into it and wait
/// for its checks again first (the owner's decision of 2026-11-17, ADR 0135).
#[test]
fn a_head_without_its_bases_tip_is_gated_and_merged_with_it() {
    let (workspace, head, tip) = pull_request_behind_main("main.txt", b"on main\n");
    let stand_in = stand_ins(&workspace, &head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let merged = workspace
        .git(&["rev-parse", "refs/heads/change"])
        .trim()
        .to_owned();
    let parents = workspace.git(&["rev-list", "--parents", "-n", "1", &merged]);
    assert_eq!(parents.trim(), format!("{merged} {head} {tip}"));
    let files = workspace.git(&["ls-tree", "--name-only", &merged]);
    assert!(files.lines().any(|file| file == "change.txt"), "{files}");
    assert!(files.lines().any(|file| file == "main.txt"), "{files}");
    let affected = &run.calls[call(&run, "cargo run --quiet --locked --bin xtask -- affected")];
    assert!(affected.contains(&format!("--head {merged}")), "{affected}");
    let gh_merge = &run.calls[call(&run, "gh pr merge ")];
    assert!(
        gh_merge.contains(&format!("--match-head-commit {merged}")),
        "{gh_merge}"
    );
}

/// A head that conflicts with its base's tip merges nothing and runs no
/// step: the gate cannot resolve a conflict, and says what to do.
#[test]
fn a_head_that_conflicts_with_its_bases_tip_is_refused_before_any_step() {
    let (workspace, head, _) = pull_request_behind_main("change.txt", b"another change\n");
    let stand_in = stand_ins(&workspace, &head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains(&format!(
            "error: pull request #{NUMBER} conflicts with the tip of main; merge main into it and resolve the conflict"
        )),
        "{}",
        run.stdout
    );
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("just ") || call.starts_with("gh pr merge")),
        "{:?}",
        run.calls
    );
    assert_eq!(
        workspace.git(&["rev-parse", "refs/heads/change"]).trim(),
        head
    );
    assert!(!gate_lock(&workspace).exists());
}

/// [`pull_request`] whose body is `body` and whose merge, as GitHub reads
/// it, closes `closes`; with a commit on the base whose message is
/// `on_the_base`, which the head then merges, and a last commit on the head
/// whose message is `on_the_head`, when given. Returns the repository, its
/// stand-ins and the head.
fn pull_request_closing(
    body: &str,
    closes: &[u32],
    on_the_base: Option<&str>,
    on_the_head: Option<&str>,
) -> (Workspace, PathBuf, String) {
    let (workspace, _) = pull_request();
    if let Some(message) = on_the_base {
        workspace.git(&["switch", "--quiet", "main"]);
        workspace.write("main.txt", b"on main\n");
        commit_all(&workspace, message);
        workspace.git(&["update-ref", "refs/remotes/origin/main", "main"]);
        workspace.git(&["switch", "--quiet", "change"]);
        workspace.git(&["merge", "--quiet", "--no-edit", "main"]);
    }
    if let Some(message) = on_the_head {
        workspace.write("change.txt", b"a closing change\n");
        commit_all(&workspace, message);
    }
    let head = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.git(&["update-ref", &format!("refs/pull/{NUMBER}/head"), &head]);
    let stand_in = stand_ins(&workspace, &head);
    std::fs::write(stand_in.join("body"), body).unwrap();
    let listed: Vec<String> = closes.iter().map(u32::to_string).collect();
    std::fs::write(stand_in.join("closes"), listed.join("\n")).unwrap();
    (workspace, stand_in, head)
}

/// Asserts that `run` was refused before the gate held anything: status 1,
/// no step, no merge and no lock.
fn assert_refused_before_the_lock(workspace: &Workspace, run: &Run) {
    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert_nothing_ran_or_held(workspace, &run.calls);
}

/// Asserts that these calls ran no step and merged nothing, and that no lock
/// is left.
fn assert_nothing_ran_or_held(workspace: &Workspace, calls: &[String]) {
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("just ") || call.starts_with("gh pr merge")),
        "{calls:?}"
    );
    assert!(!gate_lock(workspace).exists());
}

/// GitHub closes every issue a closing keyword of the body names, prose
/// included, and its list holds none a commit message names (#2241 closed an
/// unbuilt ticket named in a sentence; #2223's commits closed nothing, for a
/// reason not established): the merge must close
/// what the body's `Closes` lines name, no more and no less, or the gate
/// refuses before it holds anything, with the three lists and the rule each
/// issue at fault breaks.
#[rstest]
#[case::a_keyword_in_prose(
    "A change that closes #12 along the way.\n",
    &[12],
    None,
    ["GitHub closes on merge: #12", "`Closes` lines of the body: none", "Closing keywords of its commits: none"],
    "#12: GitHub closes it, but no `Closes #12` line names it"
)]
#[case::a_line_github_does_not_read(
    "Closes #12\n",
    &[],
    None,
    ["GitHub closes on merge: none", "`Closes` lines of the body: #12", "Closing keywords of its commits: none"],
    "#12: a `Closes #12` line names it, but GitHub does not close it"
)]
#[case::a_keyword_in_a_commit_message(
    "",
    &[],
    Some("fix: #12 the change"),
    ["GitHub closes on merge: none", "`Closes` lines of the body: none", "Closing keywords of its commits: #12"],
    "#12: a closing keyword of a commit message names it, but no `Closes #12` line does"
)]
fn a_pull_request_whose_closing_lines_disagree_with_its_merge_is_refused(
    #[case] body: &str,
    #[case] closes: &[u32],
    #[case] on_the_head: Option<&str>,
    #[case] lists: [&str; 3],
    #[case] fault: &str,
) {
    let (workspace, stand_in, _) = pull_request_closing(body, closes, None, on_the_head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_refused_before_the_lock(&workspace, &run);
    for line in lists.into_iter().chain([fault]) {
        assert!(run.stdout.contains(line), "{line:?} in {}", run.stdout);
    }
}

/// Lists that agree merge as before: a batch's body names its members with
/// `Part of` and closes only what it finishes, and a commit message may name
/// an issue a `Closes` line names too. A closing keyword in a commit the base
/// already holds is not the pull request's.
#[rstest]
#[case::none_named_and_none_to_close("", &[], None, None)]
#[case::several_lines_github_holds_one_for_one(
    "A batch.\n\nCloses #12\n- Closes #13\r\n\nPart of #14\n- **PR 15**, part of #14: a member.\n",
    &[12, 13],
    Some("Fixes #99 on main"),
    Some("Fixes #13 in its last commit")
)]
fn a_pull_request_whose_closing_lines_agree_with_its_merge_merges(
    #[case] body: &str,
    #[case] closes: &[u32],
    #[case] on_the_base: Option<&str>,
    #[case] on_the_head: Option<&str>,
) {
    let (workspace, stand_in, head) = pull_request_closing(body, closes, on_the_base, on_the_head);

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let gh_merge = &run.calls[call(&run, "gh pr merge ")];
    assert!(
        gh_merge.contains(&format!("--match-head-commit {head}")),
        "{gh_merge}"
    );
}

/// What a merge closes that cannot be read is refused: read as nothing, it
/// would let any closing keyword through.
#[test]
fn what_a_merge_closes_that_cannot_be_read_merges_nothing() {
    let (workspace, stand_in, _) = pull_request_closing("", &[], None, None);
    std::fs::write(stand_in.join("closing-unreadable"), "").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_refused_before_the_lock(&workspace, &run);
    assert!(
        run.stdout.contains(&format!(
            "error: what pull request #{NUMBER}'s merge closes could not be read from GitHub"
        )),
        "{}",
        run.stdout
    );
}

/// A body edited while the gate waited is judged as it is once the wait
/// ends, as the head is.
#[test]
fn a_gate_judges_the_closing_lines_its_pull_request_has_once_its_wait_ends() {
    let (workspace, head) = pull_request();
    let stand_in = stand_ins(&workspace, &head);
    let mut backstop = holder();
    hold_the_backstops_worktree(&workspace, &backstop.id().to_string(), HeldBy::Backstop);
    let (mut caller, output) = merge_writing_to_a_file(&workspace, &stand_in);
    until("the gate's word that it waits", || {
        std::fs::read_to_string(&output).is_ok_and(|text| text.contains("Waiting for the backstop"))
    });
    std::fs::write(stand_in.join("body"), "A change that closes #12 too.\n").unwrap();
    std::fs::write(stand_in.join("closes"), "12\n").unwrap();

    backstop.kill().unwrap();
    backstop.wait().unwrap();
    let code = caller.wait().unwrap().code();

    assert_eq!(code, Some(1));
    let said = std::fs::read_to_string(&output).unwrap();
    assert!(
        said.contains("#12: GitHub closes it, but no `Closes #12` line names it"),
        "{said}"
    );
    assert_nothing_ran_or_held(&workspace, &calls(&stand_in));
}

/// GitHub acts on no closing keyword of a pull request whose base is not the
/// repository's default branch: the gate then refuses nothing for them, and
/// says so in one line.
#[test]
fn on_a_base_other_than_the_default_branch_no_closing_line_is_refused() {
    let (workspace, stand_in, head) = pull_request_closing("Closes #13\n", &[12], None, None);
    std::fs::write(stand_in.join("default-branch"), "trunk").unwrap();

    let run = merge(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_MIN_FREE_GB", "0")],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let said = "base, main, is not the repository's default branch, trunk";
    assert_eq!(run.stdout.matches(said).count(), 1, "{}", run.stdout);
    let gh_merge = &run.calls[call(&run, "gh pr merge ")];
    assert!(
        gh_merge.contains(&format!("--match-head-commit {head}")),
        "{gh_merge}"
    );
}

/// A batch's head branch, which the hosted tier serves.
const BATCH: &str = "batch/2026-11-23a";

/// When a run began, 60 s before [`STARTED`], and 1801 s before it, past
/// the 1800 s cap.
const JUST_STARTED: &str = "2027-01-15T07:59:00Z";
const PAST_THE_CAP: &str = "2027-01-15T07:29:59Z";

/// The stand-ins for pull request #7 from [`BATCH`] in this repository, with
/// the pull-request workflow active; returns them and their `hosted/`
/// folder, which [`hosted_stand_in`] reads.
fn a_batch_served_by_the_hosted_tier(workspace: &Workspace, head: &str) -> (PathBuf, PathBuf) {
    let stand_in = stand_ins(workspace, head);
    std::fs::write(stand_in.join("branch"), BATCH).unwrap();
    let hosted = stand_in.join("hosted");
    std::fs::create_dir_all(&hosted).unwrap();
    std::fs::write(hosted.join("active"), "").unwrap();
    (stand_in, hosted)
}

/// A run of `.github/workflows/pull-request.yml` for a `pull_request` event
/// as the Actions API lists it, from `repository`, begun at `started`.
fn hosted_run(
    id: u64,
    (status, conclusion): (&str, Option<&str>),
    head: &str,
    started: &str,
    repository: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "status": status,
        "conclusion": conclusion,
        "head_sha": head,
        "run_started_at": started,
        "created_at": started,
        "html_url": page(id),
        "path": ".github/workflows/pull-request.yml",
        "event": "pull_request",
        "head_repository": { "full_name": repository },
    })
}

/// A run's page.
fn page(id: u64) -> String {
    format!("https://github.com/{REPOSITORY}/actions/runs/{id}")
}

/// Writes `runs` as the listing `name` (`runs.json`, `runs.<n>.json` or
/// `runs.cancelled.json`).
fn list_hosted_runs(hosted: &Path, name: &str, runs: &[serde_json::Value]) {
    let listing = serde_json::json!({ "total_count": runs.len(), "workflow_runs": runs });
    std::fs::write(hosted.join(name), listing.to_string()).unwrap();
}

/// The artifact `gate-record-<step>` of run `id`: its record of `head`.
fn upload_hosted_record(hosted: &Path, id: u64, step: &str, head: &str) {
    let artifact = hosted.join(format!("artifacts/{id}/gate-record-{step}"));
    std::fs::create_dir_all(&artifact).unwrap();
    std::fs::write(artifact.join(format!("{head}.tsv")), "a record\n").unwrap();
}

/// `ci/merge-pr.sh` with no time spent in the hosted waits.
fn merge_without_waits(workspace: &Workspace, stand_in: &Path) -> Run {
    merge(
        workspace,
        stand_in,
        &[
            ("GOODFETCH_GATE_MIN_FREE_GB", "0"),
            ("GOODFETCH_HOSTED_APPEAR", "0"),
            ("GOODFETCH_HOSTED_POLL", "0"),
        ],
    )
}

fn never_cancelled(run: &Run) -> bool {
    !run.calls
        .iter()
        .any(|call| call.starts_with("gh run cancel"))
}

/// A batch's gate reads the records of the hosted run that passed on its
/// head, names that run, and gives them to its plan with the repository
/// they must name (ADR 0135's amendment of 2026-11-22); a run on a commit
/// outside the batch is never read.
#[test]
fn a_batchs_gate_reuses_the_records_of_the_hosted_run_on_its_head() {
    let (workspace, head) = pull_request();
    let (stand_in, hosted) = a_batch_served_by_the_hosted_tier(&workspace, &head);
    let outside = workspace.git(&["rev-parse", "main"]).trim().to_owned();
    let passed = ("completed", Some("success"));
    list_hosted_runs(
        &hosted,
        "runs.json",
        &[
            hosted_run(101, passed, &head, JUST_STARTED, REPOSITORY),
            hosted_run(99, passed, &outside, JUST_STARTED, REPOSITORY),
        ],
    );
    upload_hosted_record(&hosted, 101, "lint", &head);
    upload_hosted_record(&hosted, 99, "lint", &outside);

    let run = merge_without_waits(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let read = format!(
        "The hosted run 101 on {head} ({}), success: its gate records are read",
        page(101)
    );
    assert!(run.stdout.contains(&read), "{}", run.stdout);
    let given = format!("hosted records of {REPOSITORY}:./101/gate-record-lint/{head}.tsv ");
    let given = run.calls.iter().position(|call| *call == given);
    assert!(
        given.is_some_and(|given| given < call(&run, "just pre-merge quick")),
        "{:?}",
        run.calls
    );
    assert!(never_cancelled(&run));
}

/// The gate waits for the hosted run still going on its head, so it runs
/// none of that run's units beside it, then reads its records.
#[test]
fn a_batchs_gate_waits_for_the_hosted_run_going_on_its_head() {
    let (workspace, head) = pull_request();
    let (stand_in, hosted) = a_batch_served_by_the_hosted_tier(&workspace, &head);
    let going = hosted_run(101, ("in_progress", None), &head, JUST_STARTED, REPOSITORY);
    list_hosted_runs(&hosted, "runs.1.json", std::slice::from_ref(&going));
    list_hosted_runs(&hosted, "runs.2.json", &[going]);
    let passed = hosted_run(
        101,
        ("completed", Some("success")),
        &head,
        JUST_STARTED,
        REPOSITORY,
    );
    list_hosted_runs(&hosted, "runs.json", &[passed]);
    upload_hosted_record(&hosted, 101, "quick", &head);

    let run = merge_without_waits(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let waited = format!(
        "Waiting for the hosted run 101 on {head} ({}), in_progress",
        page(101)
    );
    assert_eq!(run.stdout.matches(&waited).count(), 1, "{}", run.stdout);
    assert!(
        run.stdout.contains("success: its gate records are read"),
        "{}",
        run.stdout
    );
    assert!(never_cancelled(&run));
    call(&run, "just pre-merge quick");
}

/// A hosted run still going past its 1800 s cap is cancelled before the gate
/// runs a step: the gate reads what it recorded and runs the rest itself.
#[test]
fn a_hosted_run_past_its_cap_is_cancelled_before_the_gate_runs_its_units() {
    let (workspace, head) = pull_request();
    let (stand_in, hosted) = a_batch_served_by_the_hosted_tier(&workspace, &head);
    let going = hosted_run(101, ("in_progress", None), &head, PAST_THE_CAP, REPOSITORY);
    list_hosted_runs(&hosted, "runs.json", &[going]);
    let cancelled = ("completed", Some("cancelled"));
    let cancelled = hosted_run(101, cancelled, &head, PAST_THE_CAP, REPOSITORY);
    list_hosted_runs(&hosted, "runs.cancelled.json", &[cancelled]);
    upload_hosted_record(&hosted, 101, "lint", &head);

    let run = merge_without_waits(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let said = format!(
        "The hosted run 101 on {head} ({}) is past its 1800 s cap: the gate cancelled it",
        page(101)
    );
    assert!(run.stdout.contains(&said), "{}", run.stdout);
    assert!(
        run.stdout.contains("cancelled: its gate records are read"),
        "{}",
        run.stdout
    );
    assert!(call(&run, "gh run cancel 101") < call(&run, "just pre-merge quick"));
}

/// A gate whose token may not cancel a run past its cap waits for that run's
/// end, and runs no unit beside it.
#[test]
fn a_hosted_run_the_gate_may_not_cancel_is_waited_for_to_its_end() {
    let (workspace, head) = pull_request();
    let (stand_in, hosted) = a_batch_served_by_the_hosted_tier(&workspace, &head);
    std::fs::write(hosted.join("cancel-refused"), "").unwrap();
    let going = hosted_run(101, ("in_progress", None), &head, PAST_THE_CAP, REPOSITORY);
    for listing in ["runs.1.json", "runs.2.json", "runs.3.json"] {
        list_hosted_runs(&hosted, listing, std::slice::from_ref(&going));
    }
    let passed = hosted_run(
        101,
        ("completed", Some("success")),
        &head,
        PAST_THE_CAP,
        REPOSITORY,
    );
    list_hosted_runs(&hosted, "runs.json", &[passed]);

    let run = merge_without_waits(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let refused =
        "is past its 1800 s cap, and the gate may not cancel it: the gate waits for its end";
    assert_eq!(run.stdout.matches(refused).count(), 1, "{}", run.stdout);
    let listings = std::fs::read_to_string(hosted.join("listings")).unwrap();
    assert!(listings.lines().count() >= 4, "{listings}");
    let ended = run
        .calls
        .iter()
        .rposition(|call| call.contains("/actions/workflows/pull-request.yml/runs?"));
    assert!(ended.is_some_and(|ended| ended < call(&run, "just pre-merge quick")));
}

/// A hosted run that ended red on the gate's head ends the gate red, naming
/// each failed job and its page, and the gate runs none of it again (ADR
/// 0064).
#[test]
fn a_red_hosted_run_on_the_head_ends_the_gate_red() {
    let (workspace, head) = pull_request();
    let (stand_in, hosted) = a_batch_served_by_the_hosted_tier(&workspace, &head);
    let failed = hosted_run(
        101,
        ("completed", Some("failure")),
        &head,
        JUST_STARTED,
        REPOSITORY,
    );
    list_hosted_runs(&hosted, "runs.json", &[failed]);
    let job = format!("{}/job/5", page(101));
    let jobs = serde_json::json!({ "jobs": [
        { "name": "quick (goodfetch-engine)", "conclusion": "failure", "html_url": job },
        { "name": "lint", "conclusion": "success", "html_url": format!("{}/job/4", page(101)) },
    ] });
    std::fs::write(hosted.join("jobs.json"), jobs.to_string()).unwrap();

    let run = merge_without_waits(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    let said = format!(
        "error: the hosted run 101 on {head} ended failure ({}), so the gate is red",
        page(101)
    );
    assert!(run.stdout.contains(&said), "{}", run.stdout);
    let named = format!("  quick (goodfetch-engine): {job}\n");
    assert!(run.stdout.contains(&named), "{}", run.stdout);
    assert!(!run.stdout.contains("  lint:"), "{}", run.stdout);
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("just pre-merge") || call.starts_with("gh pr merge")),
        "{:?}",
        run.calls
    );
}

/// A run whose head repository is another, a fork's pull request on the
/// same branch name and head, is never read: its red counts for nothing and
/// its records are not downloaded.
#[test]
fn a_hosted_run_from_another_repository_is_never_read() {
    let (workspace, head) = pull_request();
    let (stand_in, hosted) = a_batch_served_by_the_hosted_tier(&workspace, &head);
    let failed = ("completed", Some("failure"));
    let forks = hosted_run(101, failed, &head, JUST_STARTED, "someone/goodfetch");
    list_hosted_runs(&hosted, "runs.json", &[forks]);
    upload_hosted_record(&hosted, 101, "lint", &head);

    let run = merge_without_waits(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("hosted run 101"), "{}", run.stdout);
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("gh run download")),
        "{:?}",
        run.calls
    );
    let none = format!("hosted records of {REPOSITORY}:");
    assert!(run.calls.contains(&none), "{:?}", run.calls);
}

/// Only a batch's or a spec's pull request from this repository waits for a
/// hosted run: a fork's reads none, and a ticket's pull request has none.
#[rstest]
#[case::a_forks_batch(
    BATCH,
    "true",
    "pull request #7 comes from a fork, whose hosted runs the gate never reads"
)]
#[case::a_tickets(
    "gate/3377-hosted",
    "false",
    "the hosted tier runs a batch's or a spec's pull request"
)]
fn only_a_batch_or_spec_of_this_repository_reads_hosted_runs(
    #[case] branch: &str,
    #[case] fork: &str,
    #[case] said: &str,
) {
    let (workspace, head) = pull_request();
    let (stand_in, hosted) = a_batch_served_by_the_hosted_tier(&workspace, &head);
    std::fs::write(stand_in.join("branch"), branch).unwrap();
    std::fs::write(stand_in.join("fork"), fork).unwrap();
    let passed = hosted_run(
        101,
        ("completed", Some("success")),
        &head,
        JUST_STARTED,
        REPOSITORY,
    );
    list_hosted_runs(&hosted, "runs.json", &[passed]);

    let run = merge_without_waits(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let said = format!("No hosted run serves this gate: {said}");
    assert!(run.stdout.contains(&said), "{}", run.stdout);
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("gh api repos/{owner}/{repo}")
                || call.starts_with("hosted records")),
        "{:?}",
        run.calls
    );
}
