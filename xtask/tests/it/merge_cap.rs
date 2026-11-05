//! The merge cap (`ci/merge-cap.sh`, which `just merge` runs first; ADR
//! 0135, TESTING.md's Commands): `just merge` refuses once 10 merges have
//! landed on `main` since the last green run of the incremental gate's
//! backstop, whose record is `just gate-backstop`'s `results.txt`, unless
//! the owner passes `--past-cap`. Until the backstop has a green run, the
//! cap blocks nothing. The local gate is repository tooling, so the module
//! declares no Feature.
#![cfg(unix)]

use std::path::Path;

use rstest::rstest;

use crate::affected::repository;
use crate::workspace::Workspace;

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Commits `count` merges on `main`, as `just merge` lands them, moves
/// `origin/main` to the last and returns it.
fn land(workspace: &Workspace, count: usize) -> String {
    workspace.git(&["switch", "--quiet", "main"]);
    for merge in 1..=count {
        workspace.git(&[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            &format!("Merge #{merge}"),
        ]);
    }
    workspace.git(&["update-ref", "refs/remotes/origin/main", "main"]);
    workspace.git(&["rev-parse", "main"]).trim().to_owned()
}

/// Runs the repository's `ci/merge-cap.sh` in the workspace with `args`,
/// the backstop's record holding `results` when it is given.
fn merge_cap(workspace: &Workspace, results: Option<&str>, args: &[&str]) -> Run {
    let logs = workspace.path().join("gate-backstop");
    if let Some(results) = results {
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("results.txt"), results).unwrap();
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/merge-cap.sh");
    let output = workspace
        .command(script)
        .args(args)
        .env("PATH", "/usr/bin:/bin")
        .env("GOODFETCH_GATE_BACKSTOP_LOG", &logs)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn green(commit: &str) -> String {
    format!("2026-11-05 {commit} gate=pass misses=0 suspects=0 unchecked=0\n")
}

/// Before the backstop's first green run there is nothing to count from:
/// no record, or only red and skipped runs, block no merge.
#[rstest]
#[case::no_record(None)]
#[case::red_and_skipped_runs_only(Some(
    "2026-11-05 c0ffee gate=fail misses=0 suspects=0 unchecked=0\n\
     2026-11-06 c0ffee gate=pass misses=1 suspects=0 unchecked=0\n\
     2026-11-07 c0ffee gate=skipped (free disk 20 GB < 85 GB)\n"
))]
fn before_the_backstops_first_green_run_the_cap_blocks_nothing(#[case] results: Option<&str>) {
    let workspace = repository();
    land(&workspace, 12);

    let run = merge_cap(&workspace, results, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stdout.contains("no green run yet"), "{}", run.stdout);
}

/// Nine merges since the last green run pass; the tenth fills the cap, so
/// the next merge is refused, naming the run it counts from and the
/// override.
#[rstest]
#[case::under_the_cap(9, Some(0))]
#[case::at_the_cap(10, Some(1))]
fn the_cap_refuses_once_ten_merges_landed_since_the_last_green_run(
    #[case] merges: usize,
    #[case] code: Option<i32>,
) {
    let workspace = repository();
    let tested = land(&workspace, 0);
    land(&workspace, merges);

    let run = merge_cap(&workspace, Some(&green(&tested)), &[]);

    assert_eq!(run.code, code, "{}{}", run.stdout, run.stderr);
    let said = format!(
        "{merges} merges have landed on origin/main since the backstop's green run on {tested}"
    );
    assert!(
        run.stdout.contains(&said) || run.stderr.contains(&said),
        "{}{}",
        run.stdout,
        run.stderr
    );
    assert_eq!(
        run.stderr.contains("--past-cap"),
        code == Some(1),
        "{}",
        run.stderr
    );
}

#[test]
fn the_owner_merges_past_the_cap_with_the_override_and_it_says_so() {
    let workspace = repository();
    let tested = land(&workspace, 0);
    land(&workspace, 10);

    let run = merge_cap(&workspace, Some(&green(&tested)), &["--past-cap"]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stdout.contains("past the cap of 10"), "{}", run.stdout);
}

/// The cap counts from the latest green run on `main`'s history: a later
/// red run does not reset it, and a green run of a commit `main` does not
/// hold, such as a branch's, does not count.
#[test]
fn the_cap_counts_from_the_latest_green_run_on_mains_history() {
    let workspace = repository();
    let first = land(&workspace, 0);
    let second = land(&workspace, 3);
    land(&workspace, 9);
    let results = format!(
        "{}{}2026-11-07 {second} gate=fail misses=0 suspects=0 unchecked=0\n\
         2026-11-08 0123456789abcdef0123456789abcdef01234567 gate=pass misses=0 suspects=0 unchecked=0\n",
        green(&first),
        green(&second),
    );

    let run = merge_cap(&workspace, Some(&results), &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        run.stdout.contains(&format!(
            "9 merges have landed on origin/main since the backstop's green run on {second}"
        )),
        "{}",
        run.stdout
    );
}

/// Green runs that `main`'s history holds none of leave nothing to count
/// from, so the cap refuses rather than guess.
#[test]
fn green_runs_off_mains_history_alone_are_refused() {
    let workspace = repository();
    land(&workspace, 1);

    let run = merge_cap(
        &workspace,
        Some(&green("0123456789abcdef0123456789abcdef01234567")),
        &[],
    );

    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("--past-cap"), "{}", run.stderr);
}
