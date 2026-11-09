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
use crate::workspace::Workspace;

/// A stand-in `just` that appends its arguments, with `GOODFETCH_GATE_FULL`,
/// to `calls`, prints `compare`'s content as `just _gate-compare`, and fails
/// with status 3 for the calls `failing` lists, one a line.
fn stand_in_just(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let script = format!(
        r#"#!/bin/sh
echo "$*|${{GOODFETCH_GATE_FULL:-}}" >>"{calls}"
if [ "$1" = _gate-compare ] && [ -f "{compare}" ]; then
  cat "{compare}"
fi
if [ -f "{failing}" ] && grep -qxF -- "$*" "{failing}"; then
  exit 3
fi
"#,
        calls = folder.join("calls").display(),
        compare = folder.join("compare").display(),
        failing = folder.join("failing").display(),
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
fn backstop_repository() -> Workspace {
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
    let logs = workspace.path().join("logs");
    let output = command
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .env("RUNNER_TEMP", workspace.path().join("runner-temp"))
        .env("GOODFETCH_GATE_BACKSTOP_LOG", &logs)
        .env("GOODFETCH_LAZYFS", stand_in.join("lazyfs"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .envs(variables.iter().copied())
        .output()
        .unwrap();
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
        code: output.status.code(),
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
    let (green, off_main) = (green.trim(), off_main_head.trim());
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", "A merge"]);
    workspace.git(&["commit", "--quiet", "--allow-empty", "--message", "Another"]);
    let logs = workspace.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let results: String = days
        .iter()
        .map(|day| match *day {
            "green" => format!("d {green} gate=pass misses=0 suspects=0 unchecked=0 earned=0\n"),
            "fail" => format!("d {off_main} gate=fail misses=0 suspects=0 unchecked=0 earned=0\n"),
            "miss" => format!("d {off_main} gate=pass misses=1 suspects=0 unchecked=0 earned=0\n"),
            "skipped" => format!("d {off_main} gate=skipped (free disk 1 GB < 2 GB)\n"),
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
            b"backstop_greens() { :; }\nlatest_green() { git rev-parse main~1; }\n",
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

/// With a green full run and no miss, the backstop passes.
#[test]
fn a_backstop_with_no_miss_passes() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(0), "{}", run.log);
    assert!(
        !run.calls.iter().any(|call| call.starts_with("repeat ")),
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

/// A red full run fails the backstop.
#[test]
fn a_red_full_run_fails_the_backstop() {
    let workspace = backstop_repository();
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), "pre-merge all\n").unwrap();

    let run = backstop(&workspace, &stand_in);

    assert_eq!(run.code, Some(1), "{}", run.log);
    assert!(run.results.contains(" gate=fail "), "{}", run.results);
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
}

/// A gate worktree keeps its npm packages from one gate to the next, but
/// never Vite's pre-bundled dependencies (Vitest's and Storybook's, at any
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
        "node_modules/.cache/storybook/10.6.3/0c2d/sb-vitest/vitest/0e3f/deps",
        "console/collie/node_modules/.vite/deps",
    ];
    for deps in prebundled {
        workspace.write(&format!("{worktree}/{deps}/_metadata.json"), b"{}");
        workspace.write(&format!("{worktree}/{deps}/react.js"), b"");
    }
    let kept = [
        "node_modules/react/index.js",
        "console/collie/node_modules/.bin/vite",
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
