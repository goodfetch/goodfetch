//! The nightly fuzzing (`just fuzz-nightly`, `ci/fuzz-nightly.sh`; TESTING.md,
//! Fuzzing): every `cargo-fuzz` target of the fuzz crates' manifests, run on
//! `main`'s head from the gate host's cron, which gives way to a gate. The
//! script runs against stand-ins for `cargo` and `gh` that record each call, in
//! a throwaway repository holding `origin/main`'s script; the private fuzzing
//! repository is a local one. The local gate's tooling declares no Feature.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::affected::repository;
use crate::merge_pr::until;
use crate::pre_merge::runs;
use crate::workspace::{Started, Workspace, write_executable};

/// The toolchain of the repository's `ci/sanitizer-toolchain.toml`, which the
/// fixture holds.
fn pinned_toolchain() -> String {
    let pins = repository_file("ci/sanitizer-toolchain.toml");
    let pins = String::from_utf8(pins).unwrap();
    pins.lines()
        .find_map(|line| line.strip_prefix("toolchain = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap()
        .to_owned()
}

/// A file of the repository, as the commit under test holds it.
fn repository_file(path: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(path)).unwrap()
}

/// A fuzz crate's manifest with these targets, a `cargo-fuzz` crate or not.
fn manifest(package: &str, fuzz: bool, targets: &[&str]) -> String {
    use std::fmt::Write as _;
    let mut text = format!(
        "[package]\nname = \"{package}\"\nversion = \"0.0.0\"\n\n[package.metadata]\ncargo-fuzz = {fuzz}\n\n[workspace]\n"
    );
    for target in targets {
        write!(
            text,
            "\n[[bin]]\nname = \"{target}\"\npath = \"fuzz_targets/{target}.rs\"\ntest = false\n"
        )
        .unwrap();
    }
    text
}

/// The fixture as a repository whose `origin` is itself, `main` holding the
/// script and two fuzz crates, `a` with targets `one` (with a dictionary) and
/// `two`, `b` with `three`, and a crate `c` whose `fuzz/` is no `cargo-fuzz`
/// crate.
fn fuzz_repository() -> Workspace {
    let workspace = repository();
    let path = workspace.path().display().to_string();
    workspace.git(&["remote", "add", "origin", &path]);
    workspace.git(&["switch", "--quiet", "main"]);
    let files: Vec<(String, Vec<u8>)> = vec![
        (
            "ci/fuzz-nightly.sh".into(),
            repository_file("ci/fuzz-nightly.sh"),
        ),
        (
            "ci/gate-worktree.sh".into(),
            repository_file("ci/gate-worktree.sh"),
        ),
        (
            "ci/sanitizer-toolchain.toml".into(),
            repository_file("ci/sanitizer-toolchain.toml"),
        ),
        (
            "crates/a/fuzz/Cargo.toml".into(),
            manifest("a-fuzz", true, &["one", "two"]).into_bytes(),
        ),
        ("crates/a/fuzz/one/seeds/seed".into(), b"1".to_vec()),
        (
            "crates/a/fuzz/one/dictionary.dict".into(),
            b"\"{\"\n".to_vec(),
        ),
        ("crates/a/fuzz/two/seeds/seed".into(), b"2".to_vec()),
        (
            "crates/b/fuzz/Cargo.toml".into(),
            manifest("b-fuzz", true, &["three"]).into_bytes(),
        ),
        ("crates/b/fuzz/three/seeds/seed".into(), b"3".to_vec()),
        (
            "crates/c/fuzz/Cargo.toml".into(),
            manifest("c-fuzz", false, &["not_a_target"]).into_bytes(),
        ),
    ];
    for (file, contents) in &files {
        workspace.write(file, contents);
        workspace.git(&["add", file]);
    }
    workspace.git(&["commit", "--quiet", "--message", "Add the fuzz crates"]);
    workspace.git(&["switch", "--quiet", "change"]);
    workspace
}

/// The input `cargo fuzz tmin`'s stand-in leaves, and its base64.
const SHRUNK: &str = "shrunk-input-9c2e";
const SHRUNK_IN_BASE64: &str = "c2hydW5rLWlucHV0LTljMmU=";

/// Stand-ins for `cargo` and `gh` that append each call to `calls` as
/// `<program> <arguments>|<token>|<HOME>|<folder>`, `<token>` being `token`
/// when `GITHUB_TOKEN` or `GH_TOKEN` is set and `none` otherwise.
///
/// `cargo +<toolchain> fuzz run` grows its first corpus by two inputs; for a
/// target `crashing` lists, it leaves a crash in its artifact prefix and fails;
/// for a target `holding` lists, it notes its process in `held` and waits while
/// `hold` exists, and told to stop meanwhile it notes in `stopped` whether the
/// job's lock is still `locked` or already `free`, and ends. `cargo fuzz tmin`
/// writes [`SHRUNK`] to its exact artifact path. `gh issue list` prints the
/// content of `open`, the open ticket's number its `--jq` would print, or
/// nothing; `gh issue create` notes its process in `gh-held` and waits while
/// `gh-hold` exists, then keeps its body as `body-<n>` and prints its issue's
/// address, `<n>` counting from 1; `gh issue comment` keeps its body as
/// `comment` and prints the comment's address.
fn stand_ins(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let lock = workspace.path().join(".claude/worktrees/fuzz-nightly.lock");
    let cargo = format!(
        r#"#!/bin/sh
token=none
if [ -n "${{GITHUB_TOKEN:-}}${{GH_TOKEN:-}}" ]; then token=token; fi
echo "cargo $*|$token|$HOME|$PWD" >>"{calls}"
[ "$2" = fuzz ] || exit 0
command=$3
shift 3
target=""
corpus=""
artifacts=""
exact=""
while [ $# -gt 0 ]; do
  case $1 in
    --fuzz-dir | --target-dir) shift ;;
    -artifact_prefix=*) artifacts=${{1#-artifact_prefix=}} ;;
    -exact_artifact_path=*) exact=${{1#-exact_artifact_path=}} ;;
    -*) ;;
    *) if [ -z "$target" ]; then target=$1; elif [ -z "$corpus" ]; then corpus=$1; fi ;;
  esac
  shift
done
case $command in
  run)
    mkdir -p "$corpus"
    : >"$corpus/grown-1"
    : >"$corpus/grown-2"
    if [ -f "{holding}" ] && grep -qxF "$target" "{holding}"; then
      trap 'held=free; if [ -f "{lock}/pid" ]; then held=locked; fi; echo $held >"{stopped}"; exit 143' TERM
      echo $$ >"{held}"
      while [ -f "{hold}" ]; do sleep 0.02; done
    fi
    if [ -f "{crashing}" ] && grep -qxF "$target" "{crashing}"; then
      mkdir -p "$artifacts"
      printf 'the crashing input' >"${{artifacts}}crash-0a1b"
      exit 1
    fi
    ;;
  tmin) printf '{shrunk}' >"$exact" ;;
esac
"#,
        calls = folder.join("calls").display(),
        holding = folder.join("holding").display(),
        held = folder.join("held").display(),
        hold = folder.join("hold").display(),
        stopped = folder.join("stopped").display(),
        crashing = folder.join("crashing").display(),
        lock = lock.display(),
        shrunk = SHRUNK,
    );
    write_executable(&folder.join("cargo"), &cargo);
    let gh = format!(
        r#"#!/bin/sh
token=none
if [ -n "${{GITHUB_TOKEN:-}}${{GH_TOKEN:-}}" ]; then token=token; fi
echo "gh $*|$token|$HOME|$PWD" >>"{calls}"
command="$1 $2"
issue=${{3:-}}
body=""
while [ $# -gt 0 ]; do
  if [ "$1" = --body-file ]; then body=$2; fi
  shift
done
case $command in
  "issue list") cat "{open}" 2>/dev/null || true ;;
  "issue create")
    if [ -f "{folder}/gh-hold" ]; then
      echo $$ >"{folder}/gh-held"
      while [ -f "{folder}/gh-hold" ]; do sleep 0.02; done
    fi
    n=$(( $(cat "{count}" 2>/dev/null || echo 0) + 1 ))
    echo $n >"{count}"
    cp "$body" "{folder}/body-$n"
    echo "https://github.com/example/issues/$n"
    ;;
  "issue comment")
    cp "$body" "{folder}/comment"
    echo "https://github.com/example/issues/$issue#issuecomment-1"
    ;;
esac
"#,
        calls = folder.join("calls").display(),
        open = folder.join("open").display(),
        count = folder.join("count").display(),
        folder = folder.display(),
    );
    write_executable(&folder.join("gh"), &gh);
    folder
}

/// A clone of a private fuzzing repository, a local bare one, with one commit.
fn fuzzing_clone(workspace: &Workspace) -> (PathBuf, PathBuf) {
    let bare = workspace.path().join("fuzzing.git");
    let clone = workspace.path().join("fuzzing");
    workspace.git(&[
        "init",
        "--quiet",
        "--bare",
        "--initial-branch=main",
        "fuzzing.git",
    ]);
    workspace.git(&["clone", "--quiet", "fuzzing.git", "fuzzing"]);
    workspace.write("fuzzing/README.md", b"Fuzzing's inputs\n");
    workspace.git(&["-C", "fuzzing", "add", "README.md"]);
    workspace.git(&["-C", "fuzzing", "commit", "--quiet", "--message", "Start"]);
    workspace.git(&["-C", "fuzzing", "push", "--quiet", "origin", "main"]);
    (bare, clone)
}

struct Night {
    code: Option<i32>,
    calls: Vec<String>,
    results: String,
    log: String,
}

impl Night {
    /// The calls that start with `start`: a program, `cargo` or `gh`, and
    /// the first of its arguments.
    fn calls_of(&self, start: &str) -> Vec<&str> {
        self.calls
            .iter()
            .map(String::as_str)
            .filter(|call| call.starts_with(start))
            .collect()
    }
}

/// The job's command as cron runs it, with the stand-ins first on `PATH`, the
/// logs in the workspace, both tokens set, and these variables added.
fn job(workspace: &Workspace, stand_in: &Path, variables: &[(&str, &str)]) -> Command {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/fuzz-nightly.sh");
    let mut command = workspace.command(script);
    command
        .arg("origin/main")
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env("GOODFETCH_FUZZ_LOG", workspace.path().join("logs"))
        .env(
            "GOODFETCH_FUZZ_REPOSITORY",
            workspace.path().join("fuzzing"),
        )
        .env("GOODFETCH_FUZZ_SECONDS", "60")
        .env("GOODFETCH_FUZZ_CORES", "2")
        .env("GITHUB_TOKEN", "a-token")
        .env("GH_TOKEN", "a-token")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Gate host")
        .env("GIT_AUTHOR_EMAIL", "gate@example.invalid")
        .env("GIT_COMMITTER_NAME", "Gate host")
        .env("GIT_COMMITTER_EMAIL", "gate@example.invalid")
        .envs(variables.iter().copied());
    command
}

/// Runs the job to its end.
fn night(workspace: &Workspace, stand_in: &Path) -> Night {
    let output = job(workspace, stand_in, &[]).output().unwrap();
    night_ended(workspace, stand_in, output.status.code())
}

/// What a night that ended with `code` left: its calls, its line and its log.
fn night_ended(workspace: &Workspace, stand_in: &Path, code: Option<i32>) -> Night {
    let logs = workspace.path().join("logs");
    let read = |path: PathBuf| std::fs::read_to_string(path).unwrap_or_default();
    let log = std::fs::read_dir(&logs)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
                .map(|entry| read(entry.path()))
                .collect()
        })
        .unwrap_or_default();
    Night {
        code,
        calls: read(stand_in.join("calls"))
            .lines()
            .map(str::to_owned)
            .collect(),
        results: read(logs.join("results.txt")),
        log,
    }
}

fn job_lock(workspace: &Workspace) -> PathBuf {
    workspace.path().join(".claude/worktrees/fuzz-nightly.lock")
}

/// The target and the time limit of a `cargo fuzz run` call.
fn target_and_limit(call: &str) -> (String, String) {
    let arguments: Vec<&str> = call.split('|').next().unwrap().split(' ').collect();
    let after_target_dir = arguments
        .iter()
        .position(|argument| *argument == "--target-dir")
        .unwrap()
        + 2;
    let limit = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("-max_total_time="))
        .unwrap();
    (arguments[after_target_dir].to_owned(), limit.to_owned())
}

/// Every target of every `cargo-fuzz` manifest runs once, on the pinned
/// toolchain, each for the same share of the night; the night's line counts
/// the targets and those whose corpus grew, and the job lets its lock go.
#[test]
fn each_target_of_the_manifests_runs_once_with_one_time_limit() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);

    let night = night(&workspace, &stand_in);

    assert_eq!(night.code, Some(0), "{}", night.log);
    let toolchain = pinned_toolchain();
    let runs = night.calls_of(&format!("cargo +{toolchain} fuzz run "));
    let mut ran: Vec<(String, String)> = runs.iter().map(|call| target_and_limit(call)).collect();
    ran.sort();
    // Three targets on two cores: two rounds of the night's 60 seconds.
    assert_eq!(
        ran,
        [("one", "30"), ("three", "30"), ("two", "30")]
            .map(|(target, limit)| (target.to_owned(), limit.to_owned())),
        "{:?}",
        night.calls
    );
    let commit = workspace.git(&["rev-parse", "main"]);
    assert!(
        night.results.ends_with(&format!(
            " {} fuzz=pass targets=3 seconds=30 grew=3/3 corpora=local (no clone of the fuzzing repository)\n",
            commit.trim()
        )),
        "{}",
        night.results
    );
    assert!(!job_lock(&workspace).exists());
}

/// A run lists the target's private corpus before its committed seeds, so it
/// grows the corpus and never the seeds, and passes the target's dictionary.
#[test]
fn a_run_lists_the_private_corpus_before_the_committed_seeds() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);

    let night = night(&workspace, &stand_in);

    let toolchain = pinned_toolchain();
    let runs = night.calls_of(&format!("cargo +{toolchain} fuzz run "));
    let one = runs.iter().find(|call| call.contains(" one ")).unwrap();
    let corpus = workspace.path().join("logs/corpus/a/one");
    assert!(
        one.contains(&format!(" one {} fuzz/one/seeds -- ", corpus.display())),
        "{one}"
    );
    assert!(one.contains(" -dict=fuzz/one/dictionary.dict"), "{one}");
}

/// The build, the runs and the pruning get no token, and a home of the
/// night's own, so they find no configuration of the user's that holds one.
#[test]
fn no_token_and_not_the_users_home_reach_what_builds_and_runs_the_targets() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);

    let night = night(&workspace, &stand_in);

    let logs = workspace.path().join("logs").display().to_string();
    let cargo = night.calls_of("cargo ");
    assert!(!cargo.is_empty(), "{}", night.log);
    for call in cargo {
        let fields: Vec<&str> = call.split('|').collect();
        assert_eq!(fields[1], "none", "a token reached {call}");
        assert!(
            fields[2].starts_with(&logs) && fields[2].ends_with("/home"),
            "the user's home reached {call}"
        );
    }
}

/// After the runs, each target's corpus is pruned with `cargo fuzz cmin`.
#[test]
fn each_corpus_is_pruned_after_the_runs() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);

    let night = night(&workspace, &stand_in);

    let toolchain = pinned_toolchain();
    let corpora = workspace.path().join("logs/corpus");
    let prunes = night.calls_of(&format!("cargo +{toolchain} fuzz cmin "));
    for target in ["a/one", "a/two", "b/three"] {
        let (_, name) = target.split_once('/').unwrap();
        let corpus = format!(" {name} {}|", corpora.join(target).display());
        assert!(
            prunes.iter().any(|call| call.contains(&corpus)),
            "{target}: {:?}",
            night.calls
        );
    }
}

/// Who holds a lock the job gives way to.
#[derive(Clone, Copy)]
enum Busy {
    /// `just merge`'s gate.
    MergeGate,
    /// A test-health run, in the backstop's worktree.
    TestHealth,
    /// The backstop's full run.
    Backstop,
}

/// A night that finds a gate running, or a scheduled heavy run in the
/// backstop's worktree, starts nothing, writes its skipped line and lets its
/// own lock go: the job gives way, it never makes a gate wait.
#[rstest::rstest]
#[case::the_merge_gate(Busy::MergeGate, "the merge gate is running")]
#[case::a_test_health_run(Busy::TestHealth, "the backstop's worktree is busy")]
#[case::the_backstop(Busy::Backstop, "the backstop's worktree is busy")]
fn with_a_gate_or_a_heavy_run_holding_its_lock_the_night_starts_nothing(
    #[case] busy: Busy,
    #[case] said: &str,
) {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);
    let mut holder = Command::new("sleep")
        .arg("600")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let lock = match busy {
        Busy::MergeGate => ".claude/worktrees/merge-gate.lock",
        Busy::TestHealth | Busy::Backstop => ".claude/worktrees/gate-backstop.lock",
    };
    workspace.write(&format!("{lock}/pid"), holder.id().to_string().as_bytes());
    if let Busy::TestHealth = busy {
        workspace.write(&format!("{lock}/test-health"), b"");
    }

    let night = night(&workspace, &stand_in);

    holder.kill().unwrap();
    holder.wait().unwrap();
    assert_eq!(night.code, Some(1), "{}", night.log);
    assert!(night.calls_of("cargo ").is_empty(), "{:?}", night.calls);
    assert!(
        night
            .results
            .ends_with(&format!(" fuzz=skipped ({said})\n")),
        "{}",
        night.results
    );
    assert!(!job_lock(&workspace).exists());
}

/// Starts the job and leaves it running, until the test ends at the latest.
fn started_job(workspace: &Workspace, stand_in: &Path) -> Started {
    Started::spawn(
        job(workspace, stand_in, &[])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
}

/// A gate that starts during a night stops the job (`ci/merge-pr.sh`): the
/// job stops the target that runs and waits for it, writes its line, which
/// says stopped, and only then lets its lock go, which ends the gate's wait.
#[rstest::rstest]
#[case::sigterm("-TERM")]
#[case::sighup("-HUP")]
#[case::sigint("-INT")]
fn a_stop_ends_the_running_target_before_the_jobs_lock_goes(#[case] stop: &str) {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);
    std::fs::write(stand_in.join("hold"), "").unwrap();
    std::fs::write(stand_in.join("holding"), "one\n").unwrap();
    let mut job = started_job(&workspace, &stand_in);
    let held = stand_in.join("held");
    until("the target's run", || {
        std::fs::read_to_string(&held).is_ok_and(|process| process.ends_with('\n'))
    });
    let target = std::fs::read_to_string(&held).unwrap().trim().to_owned();
    // The process a gate signals: the one the job's lock names.
    let process = std::fs::read_to_string(job_lock(&workspace).join("pid")).unwrap();

    let sent = Command::new("kill")
        .args([stop, process.trim()])
        .status()
        .unwrap();
    let code = job.wait().code();
    // A target the stop did not reach would wait on for ever.
    std::fs::remove_file(stand_in.join("hold")).unwrap();

    assert!(sent.success());
    assert_eq!(code, Some(143));
    assert_eq!(
        std::fs::read_to_string(stand_in.join("stopped")).ok(),
        Some("locked\n".to_owned())
    );
    assert!(!runs(&target));
    assert!(!job_lock(&workspace).exists());
    let night = night_ended(&workspace, &stand_in, code);
    assert!(
        night.results.ends_with(" fuzz=stopped\n"),
        "{}",
        night.results
    );
    assert!(night.calls_of("gh ").is_empty(), "{:?}", night.calls);
}

/// A crash is shrunk with `cargo fuzz tmin`, filed with its input as an issue of
/// the private fuzzing repository, and named in a ticket of the main tracker
/// that links that issue without the input; the night's pruned corpora and the
/// input are pushed to the private repository. Filing and pushing hold the
/// token the run did not have.
#[test]
fn a_crash_is_shrunk_and_filed_privately_and_the_ticket_holds_no_input() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);
    let (bare, clone) = fuzzing_clone(&workspace);
    std::fs::write(stand_in.join("crashing"), "two\n").unwrap();

    let night = night(&workspace, &stand_in);

    assert_eq!(night.code, Some(1), "{}", night.log);
    let toolchain = pinned_toolchain();
    let shrinks = night.calls_of(&format!("cargo +{toolchain} fuzz tmin "));
    assert_eq!(shrinks.len(), 1, "{:?}", night.calls);
    assert!(shrinks[0].contains(" two "), "{}", shrinks[0]);
    let created = night.calls_of("gh issue create ");
    assert_eq!(created.len(), 2, "{:?}", night.calls);
    for call in &created {
        assert!(call.contains("|token|"), "{call}");
    }
    assert!(
        created[0].ends_with(&format!("|{}", clone.display())),
        "{}",
        created[0]
    );
    let private = std::fs::read_to_string(stand_in.join("body-1")).unwrap();
    assert!(private.contains(SHRUNK_IN_BASE64), "{private}");
    assert!(!created[1].ends_with(&format!("|{}", clone.display())));
    let ticket = std::fs::read_to_string(stand_in.join("body-2")).unwrap();
    assert!(ticket.contains("`two`"), "{ticket}");
    assert!(
        ticket.contains("https://github.com/example/issues/1"),
        "{ticket}"
    );
    assert!(
        !ticket.contains(SHRUNK_IN_BASE64) && !ticket.contains(SHRUNK),
        "{ticket}"
    );
    assert!(
        night.results.contains(
            " fuzz=crash (two: https://github.com/example/issues/1, https://github.com/example/issues/2) "
        ),
        "{}",
        night.results
    );
    assert!(
        night.results.ends_with(" corpora=pushed\n"),
        "{}",
        night.results
    );
    let pushed = |path: &str| {
        Command::new("git")
            .args(["--git-dir", bare.to_str().unwrap(), "cat-file", "-e"])
            .arg(format!("main:{path}"))
            .status()
            .unwrap()
            .success()
    };
    assert!(pushed("corpus/a/one/grown-1"), "{}", night.log);
    assert!(pushed("corpus/b/three/grown-2"), "{}", night.log);
    let input = Command::new("git")
        .args([
            "--git-dir",
            bare.to_str().unwrap(),
            "ls-tree",
            "-r",
            "--name-only",
            "main",
            "crashes/a/two",
        ])
        .output()
        .unwrap();
    assert!(!input.stdout.is_empty(), "{}", night.log);
}

/// A new crash of a target whose ticket is still open gets its own private
/// issue, and a comment on that ticket links it in place of a second ticket.
#[test]
fn a_crash_of_a_target_whose_ticket_is_open_is_noted_on_that_ticket() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);
    fuzzing_clone(&workspace);
    std::fs::write(stand_in.join("crashing"), "two\n").unwrap();
    std::fs::write(stand_in.join("open"), "7\n").unwrap();

    let night = night(&workspace, &stand_in);

    assert_eq!(night.code, Some(1), "{}", night.log);
    assert_eq!(
        night.calls_of("gh issue create ").len(),
        1,
        "{:?}",
        night.calls
    );
    assert_eq!(
        night.calls_of("gh issue comment 7 ").len(),
        1,
        "{:?}",
        night.calls
    );
    let comment = std::fs::read_to_string(stand_in.join("comment")).unwrap();
    assert!(
        comment.contains("https://github.com/example/issues/1"),
        "{comment}"
    );
    assert!(!comment.contains(SHRUNK), "{comment}");
    assert!(
        night.results.contains(
            " fuzz=crash (two: https://github.com/example/issues/1, https://github.com/example/issues/7#issuecomment-1) "
        ),
        "{}",
        night.results
    );
}

/// A stop that comes while the night files a crash waits for the filing: the
/// private issue gets its ticket, and the line says what was filed.
#[test]
fn a_stop_while_the_night_files_waits_for_the_filing() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);
    fuzzing_clone(&workspace);
    std::fs::write(stand_in.join("crashing"), "two\n").unwrap();
    std::fs::write(stand_in.join("gh-hold"), "").unwrap();
    let mut job = started_job(&workspace, &stand_in);
    let held = stand_in.join("gh-held");
    until("the private issue's filing", || {
        std::fs::read_to_string(&held).is_ok_and(|process| process.ends_with('\n'))
    });
    let process = std::fs::read_to_string(job_lock(&workspace).join("pid")).unwrap();

    let sent = Command::new("kill")
        .args(["-TERM", process.trim()])
        .status()
        .unwrap();
    std::fs::remove_file(stand_in.join("gh-hold")).unwrap();
    let code = job.wait().code();

    assert!(sent.success());
    assert_eq!(code, Some(1));
    let night = night_ended(&workspace, &stand_in, code);
    assert_eq!(
        night.calls_of("gh issue create ").len(),
        2,
        "{:?}",
        night.calls
    );
    assert!(
        night.results.contains(
            " fuzz=crash (two: https://github.com/example/issues/1, https://github.com/example/issues/2) "
        ),
        "{}",
        night.results
    );
    assert!(!job_lock(&workspace).exists());
}

/// Without a clone of the private repository, a crash's shrunk input stays on
/// the host, named by path in the main tracker's ticket, and no private issue
/// is filed.
#[test]
fn without_a_clone_a_crash_stays_on_the_host_and_the_ticket_says_where() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);
    std::fs::write(stand_in.join("crashing"), "three\n").unwrap();

    let night = night(&workspace, &stand_in);

    assert_eq!(night.code, Some(1), "{}", night.log);
    let created = night.calls_of("gh issue create ");
    assert_eq!(created.len(), 1, "{:?}", night.calls);
    let ticket = std::fs::read_to_string(stand_in.join("body-1")).unwrap();
    assert!(ticket.contains("`three`"), "{ticket}");
    assert!(ticket.contains("on the gate host"), "{ticket}");
    assert!(!ticket.contains(SHRUNK), "{ticket}");
    let kept = workspace.path().join("logs/crashes/b/three");
    let inputs: Vec<_> = std::fs::read_dir(&kept).unwrap().collect();
    assert_eq!(inputs.len(), 1);
    assert!(
        night
            .results
            .ends_with(" corpora=local (no clone of the fuzzing repository)\n"),
        "{}",
        night.results
    );
}

/// Cron's line starts from `origin/main`'s script, not the checkout's, which
/// nothing fast-forwards.
#[test]
fn the_crontab_line_runs_the_script_of_origin_main() {
    let workspace = fuzz_repository();
    let stand_in = stand_ins(&workspace);
    let marker = workspace.path().join("the-commits-script-ran");
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.write(
        "ci/fuzz-nightly.sh",
        format!("#!/bin/bash\necho \"$*\" >>\"{}\"\n", marker.display()).as_bytes(),
    );
    workspace.git(&[
        "commit",
        "--quiet",
        "--all",
        "--message",
        "Change the script",
    ]);
    workspace.git(&["switch", "--quiet", "change"]);
    let crontab = String::from_utf8(repository_file("ci/fuzz-nightly.crontab")).unwrap();
    let line = crontab
        .lines()
        .filter(|line| !line.starts_with('#') && line.contains(" bash -lc '"))
        .map(|entry| {
            entry
                .split_once(" bash -lc '")
                .and_then(|(_, line)| line.strip_suffix('\''))
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(line.len(), 1, "{crontab}");
    let command = job(
        &workspace,
        &stand_in,
        &[("GOODFETCH_CHECKOUT", workspace.path().to_str().unwrap())],
    );
    let mut cron = Command::new("bash");
    cron.args(["-c", line[0]])
        .current_dir("/")
        .env_clear()
        .envs(
            command
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        );

    let output = cron.output().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(marker).unwrap().trim(),
        "origin/main"
    );
}
