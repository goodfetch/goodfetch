//! The fingerprint of the host a gate run ran on (ADR 0135): a recorded
//! result counts only for a run on the same host, with the same tools and in
//! the same environment.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command as Process;

use sha2::{Digest as _, Sha256};

/// The variables only `ci/merge-pr.sh` reads, before any step starts: they
/// decide whether a gate begins, never what a step runs or how.
const LAUNCHER_VARIABLES: [&str; 1] = ["GOODFETCH_GATE_MIN_FREE_GB"];

/// The `NAME=value` lines of `variables` a step can read: the run's
/// without the launcher's own, so a gate started under another disk floor
/// still reuses the results of one started without it.
pub(super) fn step_variables(variables: &str) -> String {
    variables
        .lines()
        .filter(|line| {
            let name = line.split_once('=').map_or(*line, |(name, _)| name);
            !LAUNCHER_VARIABLES.contains(&name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The fingerprint of this host and the tools the gate's steps run: the
/// kernel, the system's packages, the runtimes, the test runners, the
/// container engines, the toolchain and the toolchains rustup holds, the
/// tools no package of the system holds ([`TOOLS`], [`CARGO_TOOLS`]), and the
/// run's environment `variables` a step can read ([`step_variables`]). A
/// command that is missing counts as such. Java is asked without its
/// performance data file: when `/tmp` is full, the JVM warns on stderr that
/// it cannot write it, naming its own process ID, and each probe would then
/// fingerprint another host (FQ50).
pub(super) fn fingerprint(root: &Path, cargo: &OsStr, variables: &str) -> String {
    const PROBES: [&[&str]; 12] = [
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
        // The fuzz-target check lists itself as skipped, and passes, on a
        // host without its pinned toolchain (ci/fuzz-check.sh).
        &["rustup", "toolchain", "list"],
    ];
    let mut digest = Sha256::new();
    let mut probe = |command: &mut Process| match command.current_dir(root).output() {
        Ok(output) => {
            digest.update(output.status.code().unwrap_or(-1).to_le_bytes());
            digest.update(&output.stdout);
            digest.update(&output.stderr);
        }
        Err(_) => digest.update(b"missing"),
    };
    for arguments in PROBES.into_iter().chain(TOOLS) {
        let (program, rest) = arguments.split_first().unwrap_or((&"true", &[]));
        probe(Process::new(program).args(rest));
    }
    for subcommand in ["nextest"].into_iter().chain(CARGO_TOOLS) {
        probe(crate::child_cargo::command(cargo).args([subcommand, "--version"]));
    }
    digest.update(variables.as_bytes());
    format!("{:x}", digest.finalize())
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
