//! `just perf` and `just counted-rows` on a Linux host (`ci/perf.sh`, ADRs
//! 0071 and 0140): perf builds the bench targets (`xtask perf build`) and
//! measures them (`xtask perf run`), and the counted rows (`xtask counts`),
//! which read neither, are a run of their own, so the gate records them as a
//! step apart and `just preflight` runs them alone (ADR 0135's amendment of
//! 2026-11-22). A stand-in Cargo plays xtask and records each call.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::workspace::Workspace;

/// The counted rows' calls, in the order the script makes them.
const COUNTS: [&str; 6] = [
    "counts",
    "counts --prove",
    "counts --strace",
    "counts --strace --prove",
    "counts --valgrind",
    "counts --valgrind --prove",
];

/// A folder of stand-ins: Valgrind and gungraun-runner, which the script
/// only looks for, and a Cargo whose `run … -- <xtask arguments>` appends
/// those arguments to `calls`, and its niceness and them to `niceness`; a
/// call that `failing` names fails with status 3.
fn stand_ins(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let cargo = format!(
        r#"#!/bin/sh
while [ $# -gt 0 ] && [ "$1" != -- ]; do shift; done
shift
echo "$*" >>"{folder}/calls"
echo "$(nice) $*" >>"{folder}/niceness"
case "$*" in
  affected*) echo "bench-targets=goodfetch-frontier/instructions" ;;
esac
if [ -f "{folder}/failing" ] && [ "$*" = "$(cat "{folder}/failing")" ]; then exit 3; fi
exit 0
"#,
        folder = folder.display(),
    );
    for (name, script) in [
        ("cargo", cargo.as_str()),
        ("valgrind", "#!/bin/sh\nexit 0\n"),
        ("gungraun-runner", "#!/bin/sh\nexit 0\n"),
    ] {
        let path = folder.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

struct Run {
    code: Option<i32>,
    calls: Vec<String>,
    /// Each call's niceness, beside its xtask arguments.
    niceness: Vec<(i32, String)>,
    stderr: String,
    /// `target/skipped.txt`, the parts a local run listed as skipped.
    skipped: String,
}

/// Runs a copy of the repository's `ci/perf.sh` in the workspace with
/// `argument`, its own head for `just perf`, with the stand-ins first on
/// `PATH`.
fn perf(workspace: &Workspace, stand_ins: &Path, argument: &str) -> Run {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for script in ["ci/perf.sh", "ci/container.sh", "ci/output-value.sh"] {
        workspace.write(script, &std::fs::read(repository.join(script)).unwrap());
        std::fs::set_permissions(
            workspace.path().join(script),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    workspace.commit_all();
    let output = workspace
        .command(workspace.path().join("ci/perf.sh"))
        .arg(argument)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_ins.display()))
        .env("CARGO", stand_ins.join("cargo"))
        .env_remove("CI")
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        calls: std::fs::read_to_string(stand_ins.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
        niceness: std::fs::read_to_string(stand_ins.join("niceness"))
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let (niceness, call) = line.split_once(' ').unwrap();
                (niceness.parse().unwrap(), call.to_owned())
            })
            .collect(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        skipped: std::fs::read_to_string(workspace.path().join("target/skipped.txt"))
            .unwrap_or_default(),
    }
}

#[test]
fn perf_builds_the_bench_targets_then_measures_them_and_counts_no_row() {
    let workspace = Workspace::from_fixture("empty");
    let stand_ins = stand_ins(&workspace);

    let run = perf(&workspace, &stand_ins, "HEAD");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.calls,
        [
            "affected --base HEAD --head HEAD",
            "perf build --base HEAD --bench-targets goodfetch-frontier/instructions",
            "perf run",
        ],
        "{}",
        run.stderr
    );
}

/// The rows alone, in their order, each at `nice -n 10` (TESTING.md's
/// priority for load beside other work), so they yield the CPU to the gate's
/// other steps.
#[test]
fn the_counted_rows_run_alone_at_low_priority() {
    let workspace = Workspace::from_fixture("empty");
    let stand_ins = stand_ins(&workspace);
    let own: i32 = String::from_utf8(std::process::Command::new("nice").output().unwrap().stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    let run = perf(&workspace, &stand_ins, "--counted-rows");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls, COUNTS, "{}", run.stderr);
    for (niceness, call) in &run.niceness {
        assert_eq!(*niceness, (own + 10).min(19), "{call}: {:?}", run.niceness);
    }
}

#[test]
fn a_red_counted_row_fails_the_counted_rows_with_its_status() {
    let workspace = Workspace::from_fixture("empty");
    let stand_ins = stand_ins(&workspace);
    std::fs::write(stand_ins.join("failing"), "counts --strace").unwrap();

    let run = perf(&workspace, &stand_ins, "--counted-rows");

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert_eq!(run.calls, COUNTS[..3], "{}", run.stderr);
}

/// The rows read under Valgrind run through gungraun-runner: a local host
/// without it runs the others and lists the rest as skipped.
#[test]
fn without_gungraun_the_counted_rows_list_the_valgrind_rows_as_skipped() {
    let workspace = Workspace::from_fixture("empty");
    let stand_ins = stand_ins(&workspace);
    std::fs::remove_file(stand_ins.join("gungraun-runner")).unwrap();

    let run = perf(&workspace, &stand_ins, "--counted-rows");

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls, COUNTS[..4], "{}", run.stderr);
    assert!(
        run.skipped.starts_with("just counted-rows: skipped"),
        "{}",
        run.skipped
    );
}

#[test]
fn a_red_bench_build_fails_perf_before_it_measures() {
    let workspace = Workspace::from_fixture("empty");
    let stand_ins = stand_ins(&workspace);
    std::fs::write(
        stand_ins.join("failing"),
        "perf build --base HEAD --bench-targets goodfetch-frontier/instructions",
    )
    .unwrap();

    let run = perf(&workspace, &stand_ins, "HEAD");

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        !run.calls.iter().any(|call| call == "perf run"),
        "{:?}",
        run.calls
    );
}
