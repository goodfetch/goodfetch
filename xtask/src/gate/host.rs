//! The fingerprint of the host a gate run ran on (ADR 0135): a recorded
//! result counts only for a run on the same host, with the same tools and in
//! the same environment.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use sha2::{Digest as _, Sha256};

/// The variables only the gate's launchers read: `ci/merge-pr.sh`'s, before
/// any step starts, which decides whether a gate begins, those that say
/// where the record is and which layer runs (`ci/gate-records.sh`,
/// `ci/pre-merge.sh`), and the base `just quick` reads its change against,
/// which its run's unit names (`quick_runs`). None changes what a step runs
/// or how beyond what the key holds, so every layer fingerprints one host
/// alike.
const LAUNCHER_VARIABLES: [&str; 4] = [
    "GOODFETCH_GATE_MIN_FREE_GB",
    "GOODFETCH_GATE_RECORDS",
    "GOODFETCH_GATE_LAYER",
    "GOODFETCH_QUICK_BASE",
];

/// The variables of the keep-going steps (`ci/keep-going.sh`), which
/// `just quick` sets for its own steps: where each step's log goes, the step
/// a nested run is started from, and the listing of the steps in place of a
/// run. None changes a step's result, and the pre-push hook asks whether a
/// run is held outside them.
const STEP_RUNNER_VARIABLES: [&str; 3] = [
    "GOODFETCH_STEPS_DIR",
    "GOODFETCH_STEP",
    "GOODFETCH_STEPS_LIST",
];

/// The variables the gate removes from its environment before any step, so
/// each uses every thread whatever the shell that started it capped
/// (`ci/pre-merge.sh`, #2090's F1): no step of a planned run reads them.
pub(super) const REMOVED_BY_THE_GATE: [&str; 2] = ["NEXTEST_TEST_THREADS", "CARGO_BUILD_JOBS"];

/// The `NAME=value` lines of `variables` a step can read: the run's without
/// the launcher's own, the keep-going steps' and those the gate removes, so a
/// gate started under another disk floor still reuses the results of one
/// started without it, a run inside a step fingerprints the host as one
/// outside it does, and a plan made in a shell that caps threads or jobs
/// fingerprints the host as the gate does (#2090's F5).
pub(super) fn step_variables(variables: &str) -> String {
    variables
        .lines()
        .filter(|line| {
            let name = line.split_once('=').map_or(*line, |(name, _)| name);
            !LAUNCHER_VARIABLES.contains(&name)
                && !STEP_RUNNER_VARIABLES.contains(&name)
                && !REMOVED_BY_THE_GATE.contains(&name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The fingerprint of this host and the tools the gate's steps run: the
/// kernel, the system's packages, the runtimes, the test runners, the
/// container engines, the toolchain, the toolchains rustup holds with their
/// components, the tools no package of the system holds ([`TOOLS`],
/// [`CARGO_TOOLS`]), the Python of the project's environment, the ignore
/// files Git reads outside the tree ([`ignore_files`]), and the run's
/// environment `variables` a step can read ([`step_variables`]). A command
/// or a file that is missing counts as such. Java is asked without its
/// performance data file: when `/tmp` is full, the JVM warns on stderr that
/// it cannot write it, naming its own process ID, and each probe would then
/// fingerprint another host (FQ50).
pub(super) fn host_fingerprint(root: &Path, cargo: &OsStr, variables: &str) -> String {
    const PROBES: [&[&str]; 11] = [
        &["uname", "-srvm"],
        &["dpkg-query", "-W", "-f=${Package}=${Version}\n"],
        &["node", "--version"],
        &["pnpm", "--version"],
        &["uv", "--version"],
        &["python3", "--version"],
        &["valgrind", "--version"],
        &["podman", "--version"],
        &["docker", "--version"],
        &["java", "-XX:-UsePerfData", "-version"],
        &["rustc", "-Vv"],
    ];
    let mut digest = Sha256::new();
    let mut probe = |command: &mut Process| {
        let Ok(output) = command.current_dir(root).output() else {
            digest.update(b"missing");
            return String::new();
        };
        digest.update(output.status.code().unwrap_or(-1).to_le_bytes());
        digest.update(&output.stdout);
        digest.update(&output.stderr);
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    for arguments in PROBES.into_iter().chain(TOOLS) {
        let (program, rest) = arguments.split_first().unwrap_or((&"true", &[]));
        probe(Process::new(program).args(rest));
    }
    for subcommand in ["nextest"].into_iter().chain(CARGO_TOOLS) {
        probe(crate::child_cargo::command(cargo).args([subcommand, "--version"]));
    }
    // The fuzz-target check lists itself as skipped, and passes, on a host
    // without its pinned toolchain (ci/fuzz-check.sh), and the proofs' Miri
    // fails on one without its `miri` component (ci/proofs/run.sh).
    let toolchains = probe(Process::new("rustup").args(["toolchain", "list"]));
    for toolchain in toolchains
        .lines()
        .filter_map(|line| line.split_whitespace().next())
    {
        probe(Process::new("rustup").args([
            "component",
            "list",
            "--installed",
            "--toolchain",
            toolchain,
        ]));
    }
    // Pyright, mypy and pytest run on the environment's Python, which no
    // file of the tree pins and the system's `python3` need not be.
    for file in [root.join(".venv/pyvenv.cfg")]
        .into_iter()
        .chain(ignore_files(root))
    {
        match std::fs::read(&file) {
            Ok(contents) => digest.update(contents),
            Err(_) => digest.update(b"missing"),
        }
    }
    digest.update(variables.as_bytes());
    format!("{:x}", digest.finalize())
}

/// The files outside the tree that tell Git what else to ignore: the
/// repository's own `info/exclude`, and the user's (`core.excludesFile`,
/// or Git's default beside the user's configuration). Lint's check of
/// `.gitignore` (ci/ignored-sources.sh) and the tools that skip ignored
/// files read them with the tree's own.
fn ignore_files(root: &Path) -> Vec<PathBuf> {
    let git = |arguments: &[&str]| {
        Process::new("git")
            .args(arguments)
            .current_dir(root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .trim_end()
                    .to_owned()
            })
            .filter(|line| !line.is_empty())
    };
    let own = git(&["rev-parse", "--git-path", "info/exclude"]).map(|path| root.join(path));
    let users = git(&["config", "--path", "--get", "core.excludesFile"])
        .map(PathBuf::from)
        .or_else(|| {
            let configuration = std::env::var_os("XDG_CONFIG_HOME")
                .filter(|folder| !folder.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
                });
            configuration.map(|folder| folder.join("git/ignore"))
        });
    own.into_iter().chain(users).collect()
}

/// The tools the steps run that the host installs outside its package
/// manager, each asked for its version: `just`, which runs every recipe, and
/// those of lint and the proofs. A step whose tool is missing lists itself as
/// skipped and passes outside CI (ci/tool.sh), so its green result holds only
/// while the tool stays missing, and a tool's verdict only for its version.
const TOOLS: [&[&str]; 6] = [
    &["just", "--version"],
    &["typos", "--version"],
    &["zizmor", "--version"],
    &["actionlint", "-version"],
    &["gitleaks", "version"],
    &["osv-scanner", "--version"],
];

/// The Cargo subcommands the steps run besides nextest, found as Cargo finds
/// them: those of lint (cargo-deny, cargo-shear, cargo-hakari), the feature
/// combinations (cargo-hack), the fuzz-target check (cargo-fuzz) and the
/// proofs (cargo-audit, cargo-insta, and the others' again).
const CARGO_TOOLS: [&str; 7] = ["deny", "shear", "hakari", "hack", "fuzz", "audit", "insta"];
