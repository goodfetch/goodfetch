//! Measuring the base, then the head, in one job on one runner (ADR 0071),
//! in two steps so the job can deny outbound traffic between them (ADR
//! 0043):
//!
//! - **build**, before the deny: checks the base out into its own worktree
//!   under the output folder, fetches what its lockfile needs, and builds the
//!   selected bench targets of the base and of the head, into the one target
//!   folder, so the base's dependencies come from the same cache;
//! - **run**, under the deny: runs each base target with
//!   `--save-baseline=base`, then each head target with `--baseline=base`,
//!   twenty more runs of each head target holding a benchmark the base lacks
//!   (its admission), and size-limit on the base and the head when the head
//!   has a `.size-limit.json`. It writes every report into the output folder
//!   and a copy into `target/junit/perf/`, which the job uploads with its
//!   JUnit files, and leaves the comparison to `compare`.
//!
//! gungraun keeps its baselines in its own folder under the output folder, so
//! no earlier run's baseline is ever compared with.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process and blocking-fs bans target async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::metadata;

use super::budget;
use super::gungraun;

/// The output folder, under the workspace's `target/`.
pub(crate) const OUT: &str = "target/perf";
/// Where the job's JUnit files are uploaded from; the reports join them.
const JUNIT: &str = "target/junit";
/// The size-limit configuration whose presence turns size-limit on.
const SIZE_LIMIT_CONFIG: &str = ".size-limit.json";
/// The base's target folder, inside the workspace's.
pub(crate) const BASE_TARGET: &str = "perf-base";
/// The gungraun baseline the base is saved as.
const BASELINE: &str = "base";

/// What `build` prepared, for `run`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    /// The base commit.
    base: String,
    /// `<crate>/<bench target>`, as `xtask affected` selects them.
    head_targets: Vec<String>,
    /// The selected targets the base has too.
    base_targets: Vec<String>,
    target_directory: PathBuf,
}

impl Plan {
    /// The base's own target folder. Cargo names a workspace member's
    /// artifacts by its path relative to the workspace root, so the base's
    /// and the head's would overwrite each other in one folder, and a head
    /// file older than the base's build would be taken as built. The
    /// cache-warming job builds the bench targets here too (`just warm`).
    fn base_target_directory(&self) -> PathBuf {
        self.target_directory.join(BASE_TARGET)
    }
}

/// The reports `run` wrote, relative to the workspace.
pub(crate) struct Reports {
    pub(crate) head: PathBuf,
    pub(crate) admission: Option<PathBuf>,
    pub(crate) size_base: Option<PathBuf>,
    pub(crate) size_head: Option<PathBuf>,
}

fn out(root: &Path) -> PathBuf {
    root.join(OUT)
}

fn base_tree(root: &Path) -> PathBuf {
    out(root).join("base")
}

/// Checks out `base` and builds the base's and the head's `targets`.
pub(crate) fn build(cargo: &OsStr, root: &Path, base: &str, targets: &[String]) -> Result<()> {
    let commit = git(
        root,
        &["rev-parse", "--verify", "--end-of-options"],
        &[&format!("{base}^{{commit}}")],
    )?;
    let commit = commit.trim().to_owned();
    remove_base_tree(root)?;
    let out = out(root);
    if out.exists() {
        std::fs::remove_dir_all(&out).with_context(|| format!("removing {}", out.display()))?;
    }
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let tree = base_tree(root);
    git(
        root,
        &["worktree", "add", "--detach", "--quiet"],
        &[&tree.to_string_lossy(), &commit],
    )?;
    let target_directory = metadata::target_directory(cargo, root)?;
    let base_target_directory = target_directory.join(BASE_TARGET);
    let base_targets: Vec<String> = if targets.is_empty() {
        Vec::new()
    } else {
        let present = bench_targets(cargo, &tree)?;
        targets
            .iter()
            .filter(|target| present.contains(*target))
            .cloned()
            .collect()
    };
    if !base_targets.is_empty() {
        cargo_command(cargo, &tree, &base_target_directory)
            .args(["fetch", "--locked"])
            .status_checked("cargo fetch in the base")?;
    }
    for target in &base_targets {
        cargo_bench(cargo, &tree, &base_target_directory, target)
            .arg("--no-run")
            .status_checked(&format!("building the base's {target}"))?;
    }
    for target in targets {
        cargo_bench(cargo, root, &target_directory, target)
            .arg("--no-run")
            .status_checked(&format!("building the head's {target}"))?;
    }
    let plan = Plan {
        base: commit,
        head_targets: targets.to_vec(),
        base_targets,
        target_directory,
    };
    std::fs::write(out.join("plan.json"), serde_json::to_vec_pretty(&plan)?)
        .context("writing the plan")?;
    Ok(())
}

/// Measures the base, then the head, as `build` planned.
pub(crate) fn run(cargo: &OsStr, root: &Path) -> Result<Reports> {
    let out = out(root);
    let plan: Plan = serde_json::from_slice(
        &std::fs::read(out.join("plan.json"))
            .context("reading target/perf/plan.json: run `xtask perf build` first")?,
    )
    .context("parsing target/perf/plan.json")?;
    let tree = base_tree(root);
    // Absolute, since the base's targets run in the base's tree.
    let gungraun_home =
        std::path::absolute(out.join("gungraun")).context("resolving gungraun's folder")?;
    let base_report = out.join("base.jsonl");
    let head_report = out.join("head.jsonl");
    File::create(&base_report).context("creating base.jsonl")?;
    File::create(&head_report).context("creating head.jsonl")?;
    for target in &plan.base_targets {
        measure(
            cargo_bench(cargo, &tree, &plan.base_target_directory(), target)
                .env("GUNGRAUN_HOME", &gungraun_home)
                .args(["--", &format!("--save-baseline={BASELINE}")]),
            &base_report,
            &format!("measuring the base's {target}"),
        )?;
    }
    for target in &plan.head_targets {
        measure(
            cargo_bench(cargo, root, &plan.target_directory, target)
                .env("GUNGRAUN_HOME", &gungraun_home)
                .args(["--", &format!("--baseline={BASELINE}")]),
            &head_report,
            &format!("measuring the head's {target}"),
        )?;
    }
    let admission = admit(cargo, root, &plan, &head_report)?;
    let (size_base, size_head) = if root.join(SIZE_LIMIT_CONFIG).is_file() {
        let size_base = tree
            .join(SIZE_LIMIT_CONFIG)
            .is_file()
            .then(|| size_limit(root, &tree, &out.join("size-base.json")))
            .transpose()?;
        let size_head = size_limit(root, root, &out.join("size-head.json"))?;
        (size_base, Some(size_head))
    } else {
        (None, None)
    };
    copy_reports(root)?;
    remove_base_tree(root)?;
    let relative = |path: &Path| path.strip_prefix(root).unwrap_or(path).to_path_buf();
    Ok(Reports {
        head: relative(&head_report),
        admission: admission.as_deref().map(relative),
        size_base: size_base.as_deref().map(relative),
        size_head: size_head.as_deref().map(relative),
    })
}

/// Runs each head target that holds a benchmark the base lacks 20 more
/// times, one report a run, and returns their folder, or `None` when no
/// benchmark is new.
fn admit(cargo: &OsStr, root: &Path, plan: &Plan, head_report: &Path) -> Result<Option<PathBuf>> {
    let new_targets: BTreeSet<String> = gungraun::read(&[head_report])?
        .into_iter()
        .filter(|measured| measured.base.is_none())
        .filter_map(|measured| {
            let (package, rest) = measured.subject.split_once('/')?;
            let file = rest.split("::").next()?;
            Some(format!("{package}/{file}"))
        })
        .filter(|target| plan.head_targets.contains(target))
        .collect();
    if new_targets.is_empty() {
        return Ok(None);
    }
    let folder = out(root).join("admission");
    std::fs::create_dir_all(&folder).context("creating the admission folder")?;
    let gungraun_home = std::path::absolute(out(root).join("admission-gungraun"))
        .context("resolving gungraun's admission folder")?;
    for run in 1..=budget::ADMISSION_RUNS {
        let report = folder.join(format!("run-{run:02}.jsonl"));
        File::create(&report).context("creating an admission report")?;
        for target in &new_targets {
            measure(
                cargo_bench(cargo, root, &plan.target_directory, target)
                    .env("GUNGRAUN_HOME", &gungraun_home)
                    .arg("--"),
                &report,
                &format!("admission run {run} of {target}"),
            )?;
        }
    }
    Ok(Some(folder))
}

/// Runs size-limit in `folder` (the head's workspace or the base's tree),
/// with the head's installed packages, and writes its JSON report.
fn size_limit(root: &Path, folder: &Path, report: &Path) -> Result<PathBuf> {
    let modules = folder.join("node_modules");
    if folder != root && !modules.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("node_modules"), &modules)
            .context("linking the head's node_modules into the base")?;
        #[cfg(not(unix))]
        bail!("size-limit on the base needs a Unix host to link node_modules");
    }
    let output = Command::new(root.join("node_modules/.bin/size-limit"))
        .arg("--json")
        .current_dir(folder)
        .stderr(Stdio::inherit())
        .output()
        .context("running size-limit")?;
    // size-limit exits non-zero when an entry passes its own `limit`; the
    // report is still whole, and `compare` holds the limits.
    let _: serde_json::Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("size-limit wrote no JSON report in {}", folder.display()))?;
    std::fs::write(report, &output.stdout)
        .with_context(|| format!("writing {}", report.display()))?;
    Ok(report.to_path_buf())
}

/// Copies every report into the JUnit folder the job uploads.
fn copy_reports(root: &Path) -> Result<()> {
    let out = out(root);
    let junit = root.join(JUNIT).join("perf");
    std::fs::create_dir_all(&junit).context("creating target/junit/perf")?;
    for name in [
        "base.jsonl",
        "head.jsonl",
        "size-base.json",
        "size-head.json",
    ] {
        let from = out.join(name);
        if from.is_file() {
            std::fs::copy(&from, junit.join(name))
                .with_context(|| format!("copying {}", from.display()))?;
        }
    }
    let admission = out.join("admission");
    if admission.is_dir() {
        let to = junit.join("admission");
        std::fs::create_dir_all(&to).context("creating target/junit/perf/admission")?;
        for entry in std::fs::read_dir(&admission).context("reading the admission folder")? {
            let entry = entry?;
            std::fs::copy(entry.path(), to.join(entry.file_name()))
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// The workspace's `<crate>/<bench target>`s at `root`.
fn bench_targets(cargo: &OsStr, root: &Path) -> Result<BTreeSet<String>> {
    // A base from before the workspace existed has no bench target.
    if !root.join("Cargo.toml").is_file() {
        return Ok(BTreeSet::new());
    }
    Ok(metadata::packages(cargo, root)?
        .into_iter()
        .flat_map(|package| {
            package
                .targets
                .into_iter()
                .filter(|target| target.kind.iter().any(|kind| kind == "bench"))
                .map(move |target| format!("{}/{}", package.name, target.name))
        })
        .collect())
}

fn remove_base_tree(root: &Path) -> Result<()> {
    let tree = base_tree(root);
    if tree.exists() {
        git(
            root,
            &["worktree", "remove", "--force"],
            &[&tree.to_string_lossy()],
        )?;
    }
    git(root, &["worktree", "prune"], &[])?;
    Ok(())
}

fn cargo_command(cargo: &OsStr, folder: &Path, target_directory: &Path) -> Command {
    let mut command = Command::new(cargo);
    command
        .current_dir(folder)
        .env("CARGO_TARGET_DIR", target_directory);
    command
}

/// `cargo bench` of one `<crate>/<bench target>`, with the `bench` profile.
fn cargo_bench(cargo: &OsStr, folder: &Path, target_directory: &Path, target: &str) -> Command {
    let (package, bench) = target.split_once('/').unwrap_or((target, target));
    let mut command = cargo_command(cargo, folder, target_directory);
    command.args(["bench", "--locked", "--package", package, "--bench", bench]);
    command
}

/// Runs `command` with gungraun's JSON output, appending its stdout to
/// `report`.
fn measure(command: &mut Command, report: &Path, what: &str) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(report)
        .with_context(|| format!("opening {}", report.display()))?;
    command
        .env("GUNGRAUN_OUTPUT_FORMAT", "json")
        .env("GUNGRAUN_NOSUMMARY", "true")
        // A container's default seccomp profile refuses the personality
        // call that turns ASLR off, which only Callgrind's cache simulation
        // is sensitive to; no gated figure uses it, and admission catches
        // any spread.
        .env("GUNGRAUN_ALLOW_ASLR", "yes")
        .stdout(file)
        .status_checked(what)
}

trait StatusChecked {
    fn status_checked(&mut self, what: &str) -> Result<()>;
}

impl StatusChecked for Command {
    fn status_checked(&mut self, what: &str) -> Result<()> {
        let status = self.status().with_context(|| format!("{what}: starting"))?;
        if !status.success() {
            bail!("{what} failed with {status}");
        }
        Ok(())
    }
}

fn git(root: &Path, args: &[&str], operands: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command.args(args);
    if !operands.is_empty() {
        command.args(operands);
    }
    let output = command.current_dir(root).output().context("running git")?;
    if !output.status.success() {
        bail!(
            "git {} {} failed: {}",
            args.join(" "),
            operands.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("reading git's output")
}
