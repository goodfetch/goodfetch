//! The incremental gate's backstop (`just gate-backstop`,
//! `ci/gate-backstop.sh`, ADR 0135): a full gate run in the gate's
//! worktree, then each result an incremental run on the same tree reused
//! and the full run found red, a selection miss that fails the backstop
//! (the test-gate audit's option 7). The steps run through a stand-in
//! `just` that records each call. The local gate is repository tooling, so
//! the module declares no Feature.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use crate::affected::repository;
use crate::merge_pr::{NO_PROCESS, until};
use crate::workspace::Workspace;

/// A stand-in `just` that appends its arguments, with `GOODFETCH_GATE_FULL`,
/// to `calls`, prints `compare`'s content as `just _gate-compare`, and fails
/// with status 3 for the calls `failing` lists, one a line. As `just
/// sweep-target` it appends to `sweeps` its arguments, `locked` when the
/// backstop's worktree is held, `written` when the day's line is in
/// `results.txt`, and the folder it runs in. For a call
/// `holding` lists, it notes its process in `held` and waits while `hold`
/// exists; told to stop meanwhile, it notes in `stopped` whether the
/// backstop's worktree is still `locked` or already `free`, and ends.
fn stand_in_just(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let script = format!(
        r#"#!/bin/sh
echo "$*|${{GOODFETCH_GATE_FULL:-}}" >>"{calls}"
if [ "$1" = _gate-compare ] && [ -f "{compare}" ]; then
  cat "{compare}"
fi
if [ "$1" = sweep-target ]; then
  held=free
  if [ -f "{lock}/pid" ]; then held=locked; fi
  line=unwritten
  if [ -s "{results}" ]; then line=written; fi
  echo "$*|$held|$line|$PWD" >>"{sweeps}"
fi
if [ -f "{failing}" ] && grep -qxF -- "$*" "{failing}"; then
  exit 3
fi
if [ -f "{holding}" ] && grep -qxF -- "$*" "{holding}"; then
  trap 'held=free; if [ -f "{lock}/pid" ]; then held=locked; fi; echo $held >"{stopped}"; exit 143' TERM
  echo $$ >"{held}"
  while [ -f "{hold}" ]; do sleep 0.02; done
fi
"#,
        holding = folder.join("holding").display(),
        held = folder.join("held").display(),
        hold = folder.join("hold").display(),
        stopped = folder.join("stopped").display(),
        calls = folder.join("calls").display(),
        compare = folder.join("compare").display(),
        failing = folder.join("failing").display(),
        sweeps = folder.join("sweeps").display(),
        results = workspace.path().join("logs/results.txt").display(),
        lock = workspace
            .path()
            .join(".claude/worktrees/gate-backstop.lock")
            .display(),
    );
    write_executable(&folder.join("just"), &script);
    // A stand-in LazyFS build, which the backstop asks for before any step.
    write_executable(&folder.join("lazyfs"), "#!/bin/sh\n");
    folder
}

fn write_executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Run {
    code: Option<i32>,
    calls: Vec<String>,
    results: String,
    misses: String,
    log: String,
}

/// A file of the repository's `ci/`, as the commit under test holds it.
fn repository_script(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ci")
            .join(name),
    )
    .unwrap()
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

/// The fixture as a repository whose `origin` is itself, so `origin/main`
/// is its `main`, holding the scripts the backstop runs from the commit it
/// tests.
pub(crate) fn backstop_repository() -> Workspace {
    let workspace = repository();
    let path = workspace.path().display().to_string();
    workspace.git(&["remote", "add", "origin", &path]);
    commit_to_main(
        &workspace,
        &[
            (
                "ci/gate-backstop.sh",
                &repository_script("gate-backstop.sh"),
            ),
            (
                "ci/gate-worktree.sh",
                &repository_script("gate-worktree.sh"),
            ),
            (
                "ci/backstop-green.sh",
                &repository_script("backstop-green.sh"),
            ),
        ],
    );
    workspace
}

/// What a commit's script does instead of the backstop, to tell that it ran.
fn script_noting_it_ran(workspace: &Workspace) -> (String, PathBuf) {
    let marker = workspace.path().join("the-commits-script-ran");
    (
        format!("#!/bin/bash\necho \"$*\" >>\"{}\"\n", marker.display()),
        marker,
    )
}

/// Runs the repository's `ci/gate-backstop.sh` in the workspace, with the
/// stand-in first on `PATH` and the logs in the workspace.
fn backstop(workspace: &Workspace, stand_in: &Path) -> Run {
    // Any disk serves the stand-in's steps.
    backstop_with_env(
        workspace,
        stand_in,
        &[("GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB", "0")],
    )
}

/// Runs the backstop as [`backstop`] does, with these variables added.
fn backstop_with_env(workspace: &Workspace, stand_in: &Path, variables: &[(&str, &str)]) -> Run {
    workspace.write(
        "runner-temp/goodfetch-os-services/env",
        b"DBUS_SESSION_BUS_ADDRESS=unix:path=/stand-in/bus\n",
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/gate-backstop.sh");
    let mut command = workspace.command(script);
    command.arg("origin/main");
    run_of(workspace, stand_in, variables, command)
}

/// Runs `command` as the backstop's cron line runs, with the stand-in first on
/// `PATH`, the logs in the workspace, and these variables added.
fn run_of(
    workspace: &Workspace,
    stand_in: &Path,
    variables: &[(&str, &str)],
    mut command: std::process::Command,
) -> Run {
    let output = as_cron_runs_it(workspace, stand_in, variables, &mut command)
        .output()
        .unwrap();
    run_ended(workspace, stand_in, output.status.code())
}

/// Gives `command` what the backstop's cron line gives it: the stand-in first
/// on `PATH`, the logs in the workspace, and these variables added.
fn as_cron_runs_it<'a>(
    workspace: &Workspace,
    stand_in: &Path,
    variables: &[(&str, &str)],
    command: &'a mut std::process::Command,
) -> &'a mut std::process::Command {
    command
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env("RUNNER_TEMP", workspace.path().join("runner-temp"))
        .env("GOODFETCH_GATE_BACKSTOP_LOG", workspace.path().join("logs"))
        .env("GOODFETCH_LAZYFS", stand_in.join("lazyfs"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .envs(variables.iter().copied())
}

/// What a run that ended with `code` left: its calls, its lines and its log.
fn run_ended(workspace: &Workspace, stand_in: &Path, code: Option<i32>) -> Run {
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
    Run {
        code,
        calls: read(stand_in.join("calls"))
            .lines()
            .map(str::to_owned)
            .collect(),
        results: read(logs.join("results.txt")),
        misses: read(logs.join("misses.txt")),
        log,
    }
}

/// A full run, then each kind of result the comparison finds: a reused
/// result the full run found red is a miss; a suspect crate of a red
/// per-member step reruns alone and is a miss when red there; a stress
/// run's suspect is reported; a result the full run never reached is
/// unchecked. A miss fails the backstop.
#[test]
fn each_reused_result_the_full_run_finds_red_is_a_miss_that_fails_the_backstop() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let found = |kind: &str, step: &str, member: Option<&str>| {
        let unit = member.map_or_else(|| step.to_owned(), |member| format!("{step}:{member}"));
        serde_json::json!({
            "kind": kind, "unit": unit, "step": step, "member": member,
            "run": "gated", "origin": "earlier",
        })
    };
    let compared = serde_json::json!([
        found("miss", "perf", None),
        found("suspect", "pre-merge", Some("goodfetch-engine")),
        found("suspect", "pre-merge", Some("goodfetch-frontier")),
        found("suspect", "stress", Some("goodfetch-engine")),
        found("unchecked", "miri", None),
    ]);
    std::fs::write(stand_in.join("compare"), compared.to_string()).unwrap();
    std::fs::write(
        stand_in.join("failing"),
        "test pre-merge goodfetch-frontier\n",
    )
    .unwrap();

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(
        run.calls.contains(&"pre-merge all|1".to_owned()),
        "{:?}",
        run.calls
    );
    // In a worktree of its own, never `just merge`'s, whose records it
    // reads.
    let worktrees = workspace.path().join(".claude/worktrees");
    assert!(worktrees.join("gate-backstop/.git").exists());
    assert!(!worktrees.join("merge-gate").exists());
    let merged_runs = worktrees.join("merge-gate/target/gate-runs");
    assert!(
        run.calls
            .iter()
            .any(|call| call.starts_with("_gate-compare ")
                && call.ends_with(&format!(" {}|", merged_runs.display()))),
        "{:?}",
        run.calls
    );
    for rerun in [
        "test pre-merge goodfetch-engine|",
        "test pre-merge goodfetch-frontier|",
    ] {
        assert!(
            run.calls.contains(&rerun.to_owned()),
            "{rerun}: {:?}",
            run.calls
        );
    }
    assert!(
        !run.calls.iter().any(|call| call.starts_with("test stress")),
        "{:?}",
        run.calls
    );
    assert!(
        run.results
            .ends_with("gate=pass misses=2 suspects=1 unchecked=1 earned=0\n"),
        "{}",
        run.results
    );
    for found in [
        "miss perf: the run on gated reused it from earlier",
        "miss pre-merge:goodfetch-frontier",
        "suspect stress:goodfetch-engine",
        "unchecked miri",
    ] {
        assert!(run.misses.contains(found), "{found}: {}", run.misses);
    }
    assert!(
        !run.misses.contains("pre-merge:goodfetch-engine:"),
        "{}",
        run.misses
    );
}

/// Each day compares from the backstop's last green run that `main` holds, so
/// earned passes merged since are stressed again however many merges, red days
/// or skipped days came between: a day that has not done them is no green run.
#[rstest::rstest]
#[case::no_green_run_yet(&[], false)]
#[case::green_on_main(&["green"], true)]
#[case::green_then_a_red_day(&["green", "fail"], true)]
#[case::green_then_a_day_with_a_miss(&["green", "miss"], true)]
#[case::green_then_a_skipped_day(&["green", "skipped"], true)]
#[case::green_off_main(&["off-main"], false)]
fn each_day_compares_since_the_last_green_run_main_holds(
    #[case] days: &[&str],
    #[case] since_green: bool,
) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let off_main = workspace.git(&["rev-parse", "change"]);
    workspace.git(&[
        "commit",
        "--quiet",
        "--allow-empty",
        "--message",
        "Off main",
    ]);
    let off_main_head = workspace.git(&["rev-parse", "HEAD"]);
    assert_ne!(off_main, off_main_head);
    workspace.git(&["switch", "--quiet", "main"]);
    let green = workspace.git(&["rev-parse", "HEAD"]);
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", "A merge"]);
    // A later day that is no green run tested a merge `main` holds, so only
    // what the day says, not where its commit is, keeps it from counting.
    let merged = workspace.git(&["rev-parse", "HEAD"]);
    let (green, merged, off_main) = (green.trim(), merged.trim(), off_main_head.trim());
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", "Another"]);
    let logs = workspace.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let results: String = days
        .iter()
        .map(|day| match *day {
            "green" => format!("d {green} gate=pass misses=0 suspects=0 unchecked=0 earned=0\n"),
            "fail" => format!("d {merged} gate=fail misses=0 suspects=0 unchecked=0 earned=0\n"),
            "miss" => format!("d {merged} gate=pass misses=1 suspects=0 unchecked=0 earned=0\n"),
            "skipped" => format!("d {merged} gate=skipped (free disk 1 GB < 2 GB)\n"),
            _ => format!("d {off_main} gate=pass misses=0 suspects=0 unchecked=0 earned=0\n"),
        })
        .collect();
    std::fs::write(logs.join("results.txt"), results).unwrap();

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    let merged_runs = workspace
        .path()
        .join(".claude/worktrees/merge-gate/target/gate-runs");
    let tail = if since_green {
        format!(" {} {green}|", merged_runs.display())
    } else {
        format!(" {}|", merged_runs.display())
    };
    assert!(
        run.calls
            .iter()
            .any(|call| call.starts_with("_gate-compare ") && call.ends_with(&tail)),
        "{tail}: {:?}",
        run.calls
    );
}

/// A record the comparison cannot read may hold earned passes, so it is a miss:
/// the day is no green run, and the next day's comparison reaches it again.
#[test]
fn a_merged_record_that_cannot_be_read_is_a_miss() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let compared = serde_json::json!([{
        "kind": "unreadable", "unit": "", "step": "", "member": null,
        "run": "gated", "origin": "", "base": "",
    }]);
    std::fs::write(stand_in.join("compare"), compared.to_string()).unwrap();

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(
        run.results.contains("gate=pass misses=1 "),
        "{}",
        run.results
    );
    assert!(run.misses.contains("miss record gated"), "{}", run.misses);
}

/// A record that is gone may have held earned passes too, as may a whole folder
/// of them: a line that read green over it would move the next day's comparison
/// past that merge for good.
#[rstest::rstest]
#[case::one_record("gated", "miss record gated: a full gate merged it")]
#[case::the_folder("", "miss records: ")]
fn a_merged_record_that_is_gone_is_a_miss(#[case] gone: &str, #[case] said: &str) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let compared = serde_json::json!([{
        "kind": "missing", "unit": "", "step": "", "member": null,
        "run": gone, "origin": "", "base": "",
    }]);
    std::fs::write(stand_in.join("compare"), compared.to_string()).unwrap();

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(
        run.results.contains("gate=pass misses=1 "),
        "{}",
        run.results
    );
    assert!(run.misses.contains(said), "{}", run.misses);
}

/// A checkout nothing fast-forwards holds an old script, which could compare,
/// skip what a later script stresses and write a green line. So whichever script
/// starts a run, the run is the commit's, its helpers included.
#[test]
fn a_run_uses_the_script_of_the_commit_it_tests() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let (script, marker) = script_noting_it_ran(&workspace);
    commit_to_main(&workspace, &[("ci/gate-backstop.sh", script.as_bytes())]);

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
    assert!(run.results.is_empty(), "{}", run.results);
    let commit = workspace.git(&["rev-parse", "main"]);
    assert_eq!(
        std::fs::read_to_string(marker).unwrap().trim(),
        commit.trim()
    );
}

/// The helpers come from the commit as well.
#[test]
fn a_run_uses_the_helpers_of_the_commit_it_tests() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    commit_to_main(
        &workspace,
        &[(
            "ci/backstop-green.sh",
            b"backstop_state() { echo on; }\nbackstop_greens() { :; }\nlatest_green() { git rev-parse main~1; }\n",
        )],
    );
    let earlier = workspace.git(&["rev-parse", "main~1"]);

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    assert!(
        run.calls
            .iter()
            .any(|call| call.starts_with("_gate-compare ")
                && call.ends_with(&format!(" {}|", earlier.trim()))),
        "{:?}",
        run.calls
    );
}

/// Cron's line starts from `origin/main`'s script, not the checkout's, which on
/// the day this came in may predate it.
#[test]
fn the_crontab_line_runs_the_script_of_origin_main() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let (script, marker) = script_noting_it_ran(&workspace);
    commit_to_main(&workspace, &[("ci/gate-backstop.sh", script.as_bytes())]);
    let crontab = repository_script("gate-backstop.crontab");
    let crontab = String::from_utf8(crontab).unwrap();
    let entry = crontab
        .lines()
        .find(|line| line.starts_with("37 3 "))
        .unwrap();
    let line = entry
        .split_once(" bash -lc '")
        .and_then(|(_, line)| line.strip_suffix('\''))
        .unwrap();
    let mut command = std::process::Command::new("bash");
    command.args(["-c", line]);
    command.current_dir("/");

    let run = run_of(
        &workspace,
        &stand_in,
        &[("GOODFETCH_CHECKOUT", workspace.path().to_str().unwrap())],
        command,
    );

    assert_eq!(run.code, Some(0), "{}", run.log);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
    assert!(marker.is_file(), "{}", run.log);
}

/// The re-stress refuses a `lazyfs::` test without a LazyFS build, and what it
/// selects takes a build to find, so a run asks up front, as `just merge`'s full
/// gate does: it fails plainly and first, never as a daily miss.
#[cfg(target_os = "linux")]
#[rstest::rstest]
#[case::unset("")]
#[case::not_a_build("/no/such/lazyfs")]
fn a_backstop_without_a_lazyfs_build_starts_no_step(#[case] lazyfs: &str) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);

    let run = backstop_with_env(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB", "0"),
            ("GOODFETCH_LAZYFS", lazyfs),
        ],
    );

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
    assert!(
        run.results.contains(" gate=skipped (no LazyFS build"),
        "{}",
        run.results
    );
    assert!(run.misses.is_empty(), "{}", run.misses);
    assert!(run.log.contains("GOODFETCH_LAZYFS"), "{}", run.log);
}

/// The way out of a miss nothing can clear, a base that can never run or a record
/// gone for good: the owner names the commit whose merges are done with, a whole
/// run still has to pass, and its line says it was told. The next day starts there,
/// and nobody edits `results.txt`.
#[test]
fn the_owner_can_name_where_a_run_starts_and_its_line_says_so() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let main = workspace.git(&["rev-parse", "main"]);
    let main = main.trim();
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", "A merge"]);
    workspace.git(&["switch", "--quiet", "change"]);

    let told = backstop_with_env(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB", "0"),
            ("GOODFETCH_GATE_BACKSTOP_SINCE", main),
        ],
    );

    assert_eq!(told.code, Some(0), "{}", told.log);
    assert!(
        told.calls
            .iter()
            .any(|call| call.starts_with("_gate-compare ") && call.ends_with(&format!(" {main}|"))),
        "{:?}",
        told.calls
    );
    assert!(
        told.results.ends_with(&format!(
            " misses=0 suspects=0 unchecked=0 earned=0 since={main}\n"
        )),
        "{}",
        told.results
    );
    let head = workspace.git(&["rev-parse", "main"]);
    let next = backstop(&workspace, &stand_in);
    assert!(
        next.calls
            .iter()
            .any(|call| call.starts_with("_gate-compare ")
                && call.ends_with(&format!(" {}|", head.trim()))),
        "{:?}",
        next.calls
    );
}

/// A commit `main` does not hold says nothing of the merges on it.
#[test]
fn a_named_start_main_does_not_hold_starts_no_step() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    workspace.git(&[
        "commit",
        "--quiet",
        "--allow-empty",
        "--message",
        "Off main",
    ]);
    let off_main = workspace.git(&["rev-parse", "HEAD"]);

    let run = backstop_with_env(
        &workspace,
        &stand_in,
        &[
            ("GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB", "0"),
            ("GOODFETCH_GATE_BACKSTOP_SINCE", off_main.trim()),
        ],
    );

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
    assert!(run.results.is_empty(), "{}", run.results);
}

/// A full run on `main`'s head stresses nothing, since its base is that head,
/// so a result that counted passes from an earlier head (#2233) is stressed
/// again here, at its whole count, against the base its run compared with: red
/// is a miss that fails the backstop. Each merge is on `main` already, so the
/// selection only notes a test or a member gone since (`merged`).
#[rstest::rstest]
#[case::green(false, Some(0), "misses=0")]
#[case::red(true, Some(1), "misses=1")]
fn each_result_that_counted_earlier_passes_is_stressed_again(
    #[case] red: bool,
    #[case] code: Option<i32>,
    #[case] misses: &str,
) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let found = |member: &str, base: &str| {
        serde_json::json!({
            "kind": "earned", "unit": format!("stress:{member}"), "step": "stress",
            "member": member, "run": "gated", "origin": "earlier", "base": base,
        })
    };
    let compared = serde_json::json!([
        found("goodfetch-engine", "the-base"),
        found("goodfetch-frontier", "the-base"),
        found("goodfetch-frontier", "the-base"),
        found("goodfetch-conformance", "another-base"),
    ]);
    std::fs::write(stand_in.join("compare"), compared.to_string()).unwrap();
    if red {
        std::fs::write(
            stand_in.join("failing"),
            "stress the-base   merged goodfetch-engine goodfetch-frontier\n",
        )
        .unwrap();
    }

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, code, "{}", run.log);
    for stressed in [
        "stress the-base   merged goodfetch-engine goodfetch-frontier|",
        "stress another-base   merged goodfetch-conformance|",
    ] {
        assert!(
            run.calls.contains(&stressed.to_owned()),
            "{stressed}: {:?}",
            run.calls
        );
    }
    assert!(
        run.results.ends_with(&format!(
            "gate=pass {misses} suspects=0 unchecked=0 earned=3\n"
        )),
        "{}",
        run.results
    );
    assert_eq!(
        run.misses
            .contains("miss stress: goodfetch-engine goodfetch-frontier"),
        red,
        "{}",
        run.misses
    );
}

/// With a green full run and no miss, the backstop passes, and reruns no
/// crate's tests alone and no stress run.
#[test]
fn a_backstop_with_no_miss_passes() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("test ") || call.starts_with("stress ")),
        "{:?}",
        run.calls
    );
    assert!(
        run.results
            .ends_with("gate=pass misses=0 suspects=0 unchecked=0 earned=0\n"),
        "{}",
        run.results
    );
    assert!(run.misses.is_empty(), "{}", run.misses);
}

/// The backstop has a switch, `ci/gate-backstop.state` of the commit it
/// tests (ADR 0135). Switched `off`, a run starts nothing, takes no lock,
/// writes no line and ends well, so the cron entry can stay; switched `on`,
/// it runs as it does for a commit without the file.
#[rstest::rstest]
#[case::off("off\n", &[], "")]
#[case::on("on\n", &["fetch|", "pre-merge all|1"], "gate=pass misses=0")]
fn the_backstop_runs_only_while_it_is_switched_on(
    #[case] state: &str,
    #[case] first_calls: &[&str],
    #[case] line: &str,
) {
    let workspace = backstop_repository();
    commit_to_main(&workspace, &[("ci/gate-backstop.state", state.as_bytes())]);
    let stand_in = stand_in_just(&workspace);

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    let calls: Vec<&str> = run
        .calls
        .iter()
        .map(String::as_str)
        .take(first_calls.len().max(1))
        .collect();
    assert_eq!(calls, first_calls);
    assert!(run.results.contains(line), "{}", run.results);
    assert_eq!(run.results.is_empty(), line.is_empty(), "{}", run.results);
    assert_eq!(
        workspace
            .path()
            .join(".claude/worktrees/gate-backstop")
            .exists(),
        !line.is_empty()
    );
}

/// A switch that says neither `on` nor `off` fails the run before any step.
#[test]
fn a_backstop_whose_switch_says_neither_on_nor_off_fails_before_any_step() {
    let workspace = backstop_repository();
    commit_to_main(&workspace, &[("ci/gate-backstop.state", b"paused\n")]);
    let stand_in = stand_in_just(&workspace);

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
    assert!(
        run.log
            .contains("says `paused`, which is neither `on` nor `off`"),
        "{}",
        run.log
    );
}

/// A backstop whose build could take the disk below what the host keeps
/// free does not start: it is listed as skipped, with the space it found,
/// and fails, so a missed day shows.
#[test]
fn a_backstop_without_the_disk_its_build_needs_is_skipped() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);

    let run = backstop_with_env(
        &workspace,
        &stand_in,
        &[("GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB", "100000000")],
    );

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(
        run.calls.iter().all(|call| !call.starts_with("pre-merge")),
        "{:?}",
        run.calls
    );
    assert!(
        run.results.contains(" gate=skipped (free disk "),
        "{}",
        run.results
    );
    assert!(sweeps(&stand_in).is_empty(), "{:?}", sweeps(&stand_in));
}

/// A reader of the results sees every day's run: one that ends on its setup,
/// before its full run's result, reads as failed, never as nothing, which
/// would look like a night the cron did not fire. Its lock goes with it.
#[cfg(target_os = "linux")]
#[rstest::rstest]
#[case::the_fetch("fetch", Some(3))]
#[case::the_worktrees_checkout("checkout", Some(128))]
#[case::the_os_services("_os-services", Some(3))]
fn a_backstop_whose_setup_fails_leaves_a_failed_results_line(
    #[case] setup: &str,
    #[case] code: Option<i32>,
) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    if setup == "checkout" {
        // A file where the worktree goes, which `git worktree add` refuses.
        workspace.write(".claude/worktrees/gate-backstop", b"");
    } else {
        std::fs::write(stand_in.join("failing"), format!("{setup}\n")).unwrap();
    }

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, code, "{}", run.log);
    assert!(run.results.contains(" gate=fail ("), "{}", run.results);
    assert_eq!(run.results.lines().count(), 1, "{}", run.results);
    assert!(
        !workspace
            .path()
            .join(".claude/worktrees/gate-backstop.lock")
            .exists()
    );
}

/// The backstop's sweeps of its worktree's `target/`, as the stand-in noted
/// them: `<arguments>|<locked or free>|<written or unwritten>|<folder>`.
fn sweeps(stand_in: &Path) -> Vec<String> {
    std::fs::read_to_string(stand_in.join("sweeps"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The two sweeps of a backstop that ran, in its worktree, under its lock and
/// once its day's line is written:
/// every profile but `ci-test` keeps what a build read in the last two days,
/// the daily runs' reach, and `ci-test`, which only the weekly constrained
/// test-health run builds, what one read in the last fifteen.
fn the_backstops_sweeps(workspace: &Workspace) -> Vec<String> {
    let worktree = workspace
        .path()
        .join(".claude/worktrees/gate-backstop")
        .canonicalize()
        .unwrap();
    [
        "sweep-target --leave ci-test --unused-hours 72",
        "sweep-target --target target/ci-test --unused-hours 384",
    ]
    .into_iter()
    .map(|sweep| format!("{sweep}|locked|written|{}", worktree.display()))
    .collect()
}

/// The backstop's worktree is on the host's own disk, and nothing removed
/// what its builds no longer use (62 GiB after its first runs, on
/// 2026-11-09). After its full run, green or red, and once its line is
/// written, the backstop sweeps its worktree's `target/`, still holding the
/// worktree's lock, so no test-health run builds there meanwhile; the sweep
/// keeps what the three runs that share the worktree still read. A red full
/// run fails the backstop, and its line says so.
#[rstest::rstest]
#[case::a_green_run(false, Some(0), " gate=pass ")]
#[case::a_red_run(true, Some(1), " gate=fail ")]
fn after_its_full_run_the_backstop_sweeps_its_target_under_its_lock(
    #[case] red: bool,
    #[case] code: Option<i32>,
    #[case] gate: &str,
) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    if red {
        std::fs::write(stand_in.join("failing"), "pre-merge all\n").unwrap();
    }

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, code, "{}", run.log);
    assert_eq!(sweeps(&stand_in), the_backstops_sweeps(&workspace));
    let full_run = run
        .calls
        .iter()
        .position(|call| call == "pre-merge all|1")
        .unwrap();
    let first_sweep = run
        .calls
        .iter()
        .position(|call| call.starts_with("sweep-target "))
        .unwrap();
    assert!(full_run < first_sweep, "{:?}", run.calls);
    assert_eq!(run.results.lines().count(), 1, "{}", run.results);
    assert!(run.results.contains(gate), "{}", run.results);
}

/// A sweep that fails changes no verdict: the run warns, sweeps the other
/// part and keeps its green line.
#[test]
fn a_sweep_that_fails_only_warns() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    std::fs::write(
        stand_in.join("failing"),
        "sweep-target --leave ci-test --unused-hours 72\n",
    )
    .unwrap();

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    assert!(
        run.log
            .contains("warning: the backstop's target/ was not swept"),
        "{}",
        run.log
    );
    assert_eq!(sweeps(&stand_in), the_backstops_sweeps(&workspace));
    assert!(
        run.results.contains(" gate=pass misses=0 "),
        "{}",
        run.results
    );
}

/// The merge gate's backstop runs ADR 0144's sweep of the tracker for real:
/// once the day's line is written, green or red, and before its worktree's
/// `target/` is swept, every `ready-for-agent` issue whose `Existing code` list
/// no longer holds is set aside. A sweep that fails, on a tracker or a token,
/// changes no verdict: the run warns and keeps its line.
#[rstest::rstest]
#[case::a_green_run("", Some(0), false)]
#[case::a_red_run("pre-merge all\n", Some(1), false)]
#[case::a_sweep_that_fails("existing-code-sweep --apply\n", Some(0), true)]
fn once_its_line_is_written_the_backstop_sweeps_the_tracker(
    #[case] failing: &str,
    #[case] code: Option<i32>,
    #[case] warned: bool,
) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), failing).unwrap();

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, code, "{}", run.log);
    let position = |prefix: &str| {
        run.calls
            .iter()
            .position(|call| call.starts_with(prefix))
            .unwrap_or_else(|| panic!("no call {prefix}: {:?}", run.calls))
    };
    let tracker_sweep = position("existing-code-sweep --apply|");
    assert!(
        position("pre-merge all|1") < tracker_sweep,
        "{:?}",
        run.calls
    );
    assert!(tracker_sweep < position("sweep-target "), "{:?}", run.calls);
    assert_eq!(
        run.log
            .contains("warning: the backstop did not sweep the tracker's `ready-for-agent` issues"),
        warned,
        "{}",
        run.log
    );
    assert_eq!(run.results.lines().count(), 1, "{}", run.results);
}

/// The backstop's full run is a second gate's worth of suites, and two of
/// them on one host fail tests on timing (ADR 0135's amendment of
/// 2026-11-10). A backstop that finds `just merge`'s gate running holds its
/// own worktree, says in its lock which gate it lets end first, so that gate
/// does not wait for it in turn, and starts no step until that gate ends;
/// then it runs in full, and a skipped day is not the price of a long gate.
#[test]
fn a_backstop_lets_a_running_merge_gate_end_before_it_starts_and_says_which() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let mut gate = std::process::Command::new("sleep")
        .arg("600")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let gate_process = gate.id().to_string();
    workspace.write(
        ".claude/worktrees/merge-gate.lock/pid",
        gate_process.as_bytes(),
    );
    let mut backstop = started_backstop(&workspace, &stand_in);
    let waits_for = workspace
        .path()
        .join(".claude/worktrees/gate-backstop.lock/waits-for");

    until("the backstop's word of the gate it waits for", || {
        std::fs::read_to_string(&waits_for).is_ok_and(|process| process.trim() == gate_process)
    });

    let before_the_gates_end = run_ended(&workspace, &stand_in, None);
    gate.kill().unwrap();
    gate.wait().unwrap();
    let code = backstop.wait().unwrap().code();
    let run = run_ended(&workspace, &stand_in, code);
    assert!(
        before_the_gates_end.calls.is_empty(),
        "{:?}",
        before_the_gates_end.calls
    );
    assert_eq!(run.code, Some(0), "{}", run.log);
    assert!(
        run.log.contains(&format!(
            "waiting for the merge gate, process {gate_process}, to end"
        )),
        "{}",
        run.log
    );
    assert!(
        run.calls.contains(&"pre-merge all|1".to_owned()),
        "{:?}",
        run.calls
    );
    assert!(
        run.results.contains(" gate=pass misses=0 "),
        "{}",
        run.results
    );
}

/// Starts the repository's `ci/gate-backstop.sh` as [`backstop`] runs it, and
/// leaves it running.
fn started_backstop(workspace: &Workspace, stand_in: &Path) -> std::process::Child {
    workspace.write(
        "runner-temp/goodfetch-os-services/env",
        b"DBUS_SESSION_BUS_ADDRESS=unix:path=/stand-in/bus\n",
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/gate-backstop.sh");
    let mut command = workspace.command(script);
    command.arg("origin/main");
    as_cron_runs_it(
        workspace,
        stand_in,
        &[("GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB", "0")],
        &mut command,
    )
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .unwrap()
}

/// A gate that died left its lock behind: it holds no backstop up, and the
/// backstop does not say it waits. A lock names when its holder started,
/// beside its process ID, so a process that took a dead gate's number, which
/// started at another time, holds no backstop up either, where the number
/// alone would have kept it waiting for ever. `pid` is the lock's process ID,
/// or, without one, this test's own, a live process.
#[rstest::rstest]
#[case::a_gate_that_died(Some(NO_PROCESS), None)]
#[case::a_process_that_took_a_dead_gates_number(None, Some("Thu Jan  1 00:00:00 1970\n"))]
fn a_backstop_waits_for_no_gate_that_died(
    #[case] pid: Option<&str>,
    #[case] started: Option<&str>,
) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let pid = pid.map_or_else(|| std::process::id().to_string(), str::to_owned);
    workspace.write(".claude/worktrees/merge-gate.lock/pid", pid.as_bytes());
    if let Some(started) = started {
        workspace.write(
            ".claude/worktrees/merge-gate.lock/started",
            started.as_bytes(),
        );
    }

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    assert!(!run.log.contains("waiting for"), "{}", run.log);
    assert!(
        run.calls.contains(&"pre-merge all|1".to_owned()),
        "{:?}",
        run.calls
    );
}

/// A backstop that finds its own worktree held, by a test-health run or by a
/// backstop still waiting for a gate, starts nothing, leaves that run its
/// lock, and lists its day as skipped: a night with no line in `results.txt`
/// would look like a night the cron did not fire.
#[test]
fn a_backstop_whose_worktree_is_held_lists_its_day_as_skipped() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    let holder = std::process::id().to_string();
    workspace.write(
        ".claude/worktrees/gate-backstop.lock/pid",
        holder.as_bytes(),
    );

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
    assert!(
        run.results.ends_with(&format!(
            " gate=skipped (the backstop's worktree is busy: process {holder})\n"
        )),
        "{}",
        run.results
    );
    assert_eq!(
        std::fs::read_to_string(
            workspace
                .path()
                .join(".claude/worktrees/gate-backstop.lock/pid")
        )
        .ok(),
        Some(holder)
    );
}

/// The backstop's lock is what keeps a gate from starting beside its suites,
/// so a stop must end those suites before the lock goes. Told to stop during
/// its full run, the backstop stops the step that runs, waits for it, and
/// only then releases its worktree; it writes no line for the day.
#[rstest::rstest]
#[case::sigterm("-TERM")]
#[case::sighup("-HUP")]
#[case::sigint("-INT")]
fn a_stop_ends_the_backstops_steps_before_its_lock_goes(#[case] stop: &str) {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("hold"), "").unwrap();
    std::fs::write(stand_in.join("holding"), "pre-merge all\n").unwrap();
    let mut backstop = started_backstop(&workspace, &stand_in);
    let held = stand_in.join("held");
    until("the backstop's full run", || {
        std::fs::read_to_string(&held).is_ok_and(|process| process.ends_with('\n'))
    });
    let full_run = std::fs::read_to_string(&held).unwrap().trim().to_owned();

    let sent = std::process::Command::new("kill")
        .args([stop, &backstop.id().to_string()])
        .status()
        .unwrap();
    let code = backstop.wait().unwrap().code();
    // A step the stop did not reach would wait on for ever.
    std::fs::remove_file(stand_in.join("hold")).unwrap();

    assert!(sent.success());
    assert_eq!(code, Some(143));
    assert_eq!(
        std::fs::read_to_string(stand_in.join("stopped")).ok(),
        Some("locked\n".to_owned())
    );
    assert!(!crate::pre_merge::runs(&full_run));
    assert!(
        !workspace
            .path()
            .join(".claude/worktrees/gate-backstop.lock")
            .exists()
    );
    let run = run_ended(&workspace, &stand_in, code);
    assert_eq!(run.results, "", "{}", run.log);
}

/// A gate worktree keeps its npm packages from one gate to the next, but
/// never Vite's pre-bundled dependencies (Vitest's, or another tool's at any
/// depth of `node_modules`): their cache is keyed by the lockfile and the
/// config only, so a head importing a package the previous head did not
/// would find it missing, have Vite bundle it mid-run and reload, and every
/// browser test file would fail to import.
#[test]
fn a_gate_worktree_keeps_its_npm_packages_but_not_vites_prebundled_dependencies() {
    let workspace = backstop_repository();
    // Ignored, as the repository's `.gitignore` ignores them.
    workspace.write(".git/info/exclude", b"node_modules/\n");
    let stand_in = stand_in_just(&workspace);
    let first = backstop(&workspace, &stand_in);
    assert_eq!(first.code, Some(0), "{}", first.log);
    let worktree = ".claude/worktrees/gate-backstop";
    let prebundled = [
        "node_modules/.vite/vitest/0a1b/deps",
        "node_modules/.cache/a-tool/10.6.3/0c2d/vitest/0e3f/deps",
        "website/node_modules/.vite/deps",
    ];
    for deps in prebundled {
        workspace.write(&format!("{worktree}/{deps}/_metadata.json"), b"{}");
        workspace.write(&format!("{worktree}/{deps}/react.js"), b"");
    }
    let kept = [
        "node_modules/react/index.js",
        "website/node_modules/.bin/vite",
        "node_modules/.vite/vitest/0a1b/results.json",
    ];
    for file in kept {
        workspace.write(&format!("{worktree}/{file}"), b"");
    }

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    for deps in prebundled {
        assert!(
            !workspace.path().join(worktree).join(deps).exists(),
            "{deps} was kept"
        );
    }
    for file in kept {
        assert!(
            workspace.path().join(worktree).join(file).exists(),
            "{file} was removed"
        );
    }
}

/// A gate worktree keeps perf's base build and the base's tree from one gate
/// to the next, as it keeps the head's build, so the next gate's perf
/// rebuilds only what the base changed; any other output of a step goes.
#[test]
fn a_gate_worktree_keeps_perfs_base_build_and_tree() {
    let workspace = backstop_repository();
    // Ignored, as the repository's `.gitignore` ignores it.
    workspace.write(".git/info/exclude", b"/target/\n");
    let stand_in = stand_in_just(&workspace);
    let first = backstop(&workspace, &stand_in);
    assert_eq!(first.code, Some(0), "{}", first.log);
    let worktree = ".claude/worktrees/gate-backstop";
    let kept = [
        "target/release/deps/bench",
        "target/perf-base/release/deps/bench",
        "target/perf-base-tree/Cargo.toml",
    ];
    for file in kept {
        workspace.write(&format!("{worktree}/{file}"), b"");
    }
    workspace.write(&format!("{worktree}/target/perf/plan.json"), b"{}");

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    for file in kept {
        assert!(
            workspace.path().join(worktree).join(file).exists(),
            "{file} was removed"
        );
    }
    assert!(
        !workspace
            .path()
            .join(worktree)
            .join("target/perf/plan.json")
            .exists(),
        "an earlier gate's perf plan was kept"
    );
}
