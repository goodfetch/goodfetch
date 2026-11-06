//! `just perf` on a Linux host (`ci/perf.sh`, ADRs 0071 and 0140): the
//! counted rows (`xtask counts`) run beside the bench targets' build
//! (`xtask perf build`), which needs neither their figures nor their build,
//! and the measurement (`xtask perf run`) starts once both have passed (the
//! test-gate audit's F2). A stand-in Cargo plays xtask and records each call.
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
/// those arguments to `calls`, and its niceness and them to `niceness`. The
/// first counted row waits, up to 10 s, until
/// the build has started, and the build until a counted row has, so each
/// fails when they run one after the other; a call that `failing` names
/// fails with status 3.
fn stand_ins(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let cargo = format!(
        r#"#!/bin/sh
while [ $# -gt 0 ] && [ "$1" != -- ]; do shift; done
shift
echo "$*" >>"{folder}/calls"
echo "$(nice) $*" >>"{folder}/niceness"
await() {{
  waited=0
  until [ -e "{folder}/$1" ]; do
    if [ $waited -ge 200 ]; then echo "$2" >&2; exit 1; fi
    sleep 0.05
    waited=$((waited + 1))
  done
}}
case "$*" in
  counts) touch "{folder}/counting"; await building "the counted rows ran before the build started" ;;
  "perf build"*) touch "{folder}/building"; await counting "the build ran before the counted rows started" ;;
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
}

/// Runs a copy of the repository's `ci/perf.sh` in the workspace against its
/// own head, with the stand-ins first on `PATH`.
fn perf(workspace: &Workspace, stand_ins: &Path) -> Run {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for script in ["ci/perf.sh", "ci/container.sh"] {
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
        .arg("HEAD")
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
    }
}

#[test]
fn the_counted_rows_run_beside_the_bench_build_then_perf_measures() {
    let workspace = Workspace::from_fixture("empty");
    let stand_ins = stand_ins(&workspace);

    let run = perf(&workspace, &stand_ins);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let counted: Vec<&str> = run
        .calls
        .iter()
        .map(String::as_str)
        .filter(|call| call.starts_with("counts"))
        .collect();
    assert_eq!(counted, COUNTS);
    assert_eq!(
        run.calls.last().map(String::as_str),
        Some("perf run"),
        "{:?}",
        run.calls
    );
}

/// The counted rows yield the CPU to the build and to other work on the host
/// (TESTING.md's `nice -n 10` for load beside other work), and the build and
/// measurement keep the script's own priority.
#[test]
fn the_counted_rows_run_at_low_priority_beside_the_build() {
    let workspace = Workspace::from_fixture("empty");
    let stand_ins = stand_ins(&workspace);

    let run = perf(&workspace, &stand_ins);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let own = run
        .niceness
        .iter()
        .find(|(_, call)| call.starts_with("perf build"))
        .map(|(niceness, _)| *niceness)
        .unwrap();
    for (niceness, call) in &run.niceness {
        let expected = if call.starts_with("counts") {
            (own + 10).min(19)
        } else {
            own
        };
        assert_eq!(*niceness, expected, "{call}: {:?}", run.niceness);
    }
}

#[test]
fn a_red_counted_row_fails_perf_before_it_measures() {
    let workspace = Workspace::from_fixture("empty");
    let stand_ins = stand_ins(&workspace);
    std::fs::write(stand_ins.join("failing"), "counts --strace").unwrap();

    let run = perf(&workspace, &stand_ins);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        run.calls.iter().any(|call| call.starts_with("perf build")),
        "{:?}",
        run.calls
    );
    assert!(
        !run.calls.iter().any(|call| call == "perf run"),
        "{:?}",
        run.calls
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

    let run = perf(&workspace, &stand_ins);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        !run.calls.iter().any(|call| call == "perf run"),
        "{:?}",
        run.calls
    );
}
