//! Measuring the base, then the head, in one job on one runner (ADR 0071),
//! in two steps so the job can deny outbound traffic between them (ADR
//! 0043):
//!
//! - **build**, before the deny: moves the base's own worktree, which stays
//!   from one build to the next beside the output folder, to the base, so
//!   Git rewrites only the files the base changed and Cargo rebuilds only
//!   what they reach; fetches what its lockfile needs, and builds the
//!   head's selected bench targets and every bench target the base has in a
//!   selected crate, into the one target folder, so the base's dependencies
//!   come from the same cache. The base's build and the head's run at the same
//!   time, each with the commands, folders and order it would have alone, so
//!   their binaries are the ones a build of one after the other makes;
//! - **run**, under the deny: runs the base targets with
//!   `--save-baseline=base`, then the head targets with `--baseline=base`,
//!   then twenty more runs of each head target holding a benchmark the base
//!   lacks (its admission), each set as many processes at once as the host
//!   has cores, since counts under Callgrind and DHAT do not depend on what
//!   else runs; each side's report keeps the plan's order of targets,
//!   and size-limit on the base and the head when the head
//!   has a `.size-limit.json`. It writes every report into the output folder
//!   and a copy into `target/junit/perf/`, which the job uploads with its
//!   JUnit files, and leaves the comparison to `compare`.
//!
//! gungraun keeps its baselines in its own folder under the output folder, so
//! no earlier run's baseline is ever compared with.
//!
//! A reference build `ci/perf-reference.toml` pins (`drift.reference`, ADR
//! 0140) goes through the same steps as the base, in its own worktree and
//! target folder: the selected targets it has, and size-limit when it has a
//! `.size-limit.json`, so its figures come from this job and this runner.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process, blocking-fs and thread bans target async crates (STYLE.md)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "xtask is no engine crate loom models (ADR 0060): the two builds share their stop flag through std's atomics"
)]

use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufRead as _, BufReader, Read};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::{base_console, git, metadata, output};

use super::budget;
use super::gungraun;
use super::reference;

/// The output folder, under the workspace's `target/`.
pub(crate) const OUT: &str = "target/perf";
/// Where the job's JUnit files are uploaded from; the reports join them.
const JUNIT: &str = "target/junit";
/// The size-limit configuration whose presence turns size-limit on.
const SIZE_LIMIT_CONFIG: &str = ".size-limit.json";
/// The base's target folder, inside the workspace's.
pub(crate) const BASE_TARGET: &str = "perf-base";
/// The base's worktree, relative to the repository.
const BASE_TREE: &str = "target/perf-base-tree";
/// The reference build's target folder, inside the workspace's, for the
/// base's reason.
const REFERENCE_TARGET: &str = "perf-reference";
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
    /// The pinned reference build, when one is pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<Reference>,
}

/// The pinned reference build, as `build` checked it out.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    commit: String,
    /// The selected targets it has too.
    targets: Vec<String>,
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
    pub(crate) base: PathBuf,
    pub(crate) head: PathBuf,
    pub(crate) admission: Option<PathBuf>,
    pub(crate) size_base: Option<PathBuf>,
    pub(crate) size_head: Option<PathBuf>,
    pub(crate) reference: Option<PathBuf>,
    pub(crate) size_reference: Option<PathBuf>,
}

fn out(root: &Path) -> PathBuf {
    root.join(OUT)
}

/// The base's worktree, kept from one build to the next outside the output
/// folder each build clears: a fresh checkout would give every file a new
/// time, and Cargo, which judges a build by its files' times, would rebuild
/// every workspace crate of the base.
fn base_tree(root: &Path) -> PathBuf {
    root.join(BASE_TREE)
}

fn reference_tree(root: &Path) -> PathBuf {
    out(root).join("reference")
}

/// Checks out `base`, and the reference build when one is pinned, and builds
/// their and the head's `targets`.
pub(crate) fn build(cargo: &OsStr, root: &Path, base: &str, targets: &[String]) -> Result<()> {
    let commit = git::commit(root, base)?;
    let reference = reference::pinned(root)?
        .map(|revision| {
            git::commit(root, &revision)
                .with_context(|| format!("resolving the revision {} pins", reference::PIN_FILE))
        })
        .transpose()?;
    remove_reference_tree(root)?;
    let out = out(root);
    if out.exists() {
        std::fs::remove_dir_all(&out).with_context(|| format!("removing {}", out.display()))?;
    }
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let tree = base_tree(root);
    move_tree(root, &tree, &commit)?;
    let target_directory = metadata::target_directory(cargo, root)?;
    // Every bench target the base has in a selected crate, so a target the
    // change renamed or removed is still measured on the base, and its
    // ceilings find their figures missing from the head (ADR 0134).
    let base_targets: Vec<String> = if targets.is_empty() {
        Vec::new()
    } else {
        let crates: BTreeSet<&str> = targets
            .iter()
            .filter_map(|target| target.split_once('/').map(|(package, _)| package))
            .collect();
        bench_targets(cargo, &tree)?
            .into_iter()
            .filter(|target| {
                target
                    .split_once('/')
                    .is_some_and(|(package, _)| crates.contains(package))
            })
            .collect()
    };
    // The base's list of targets is ready above, so
    // the base's build (then the reference's) and the head's run at the same
    // time: each side runs its own commands, in its own folder and target
    // folder, in the order it ran them alone, so only their overlap is new.
    let (reference, ()) = at_once(
        |builds| {
            build_side(
                cargo,
                builds,
                &Side {
                    name: "base",
                    tree: &tree,
                    target_directory: &target_directory.join(BASE_TARGET),
                },
                &base_targets,
            )?;
            reference
                .map(|commit| -> Result<Reference> {
                    let tree = reference_tree(root);
                    check_out(root, &tree, &commit)?;
                    let has = bench_targets(cargo, &tree)?;
                    let targets: Vec<String> = targets
                        .iter()
                        .filter(|target| has.contains(*target))
                        .cloned()
                        .collect();
                    build_side(
                        cargo,
                        builds,
                        &Side {
                            name: "reference",
                            tree: &tree,
                            target_directory: &target_directory.join(REFERENCE_TARGET),
                        },
                        &targets,
                    )?;
                    Ok(Reference { commit, targets })
                })
                .transpose()
        },
        |builds| {
            for target in targets {
                builds.run(
                    "head",
                    cargo_bench(cargo, root, &target_directory, target).arg("--no-run"),
                    &format!("building the head's {target}"),
                )?;
            }
            Ok(())
        },
    )?;
    let plan = Plan {
        base: commit,
        head_targets: targets.to_vec(),
        base_targets,
        target_directory,
        reference,
    };
    std::fs::write(out.join("plan.json"), serde_json::to_vec_pretty(&plan)?)
        .context("writing the plan")?;
    Ok(())
}

/// Moves the kept worktree at `tree` to `commit`, leaving none of an
/// earlier build's files in it, or checks `commit` out there when no
/// worktree is (a first build, or a folder deleted without Git).
fn move_tree(root: &Path, tree: &Path, commit: &str) -> Result<()> {
    if !tree.join(".git").exists() {
        return check_out(root, tree, commit);
    }
    git::text(
        tree,
        &["checkout", "--force", "--detach", "--quiet", commit],
    )?;
    git::text(tree, &["clean", "-ffdxq"])?;
    Ok(())
}

/// Checks `commit` out at `tree`.
fn check_out(root: &Path, tree: &Path, commit: &str) -> Result<()> {
    // `--force`: a tree whose folder went without Git (a cleared `target/`)
    // leaves its record, which a plain `add` refuses to reuse.
    git::text(
        root,
        &[
            "worktree",
            "add",
            "--force",
            "--detach",
            "--quiet",
            &tree.to_string_lossy(),
            commit,
        ],
    )?;
    Ok(())
}

/// A build measured beside the head: the base, or the reference.
struct Side<'a> {
    name: &'static str,
    tree: &'a Path,
    target_directory: &'a Path,
}

/// Builds `side`'s `targets`, after its own Console when its tree still has
/// one.
fn build_side(cargo: &OsStr, builds: &AtOnce, side: &Side<'_>, targets: &[String]) -> Result<()> {
    if !targets.is_empty() {
        builds.run(
            side.name,
            cargo_command(cargo, side.tree, side.target_directory).args(["fetch", "--locked"]),
            &format!("cargo fetch in the {}", side.name),
        )?;
        // A side that predates the Console's removal builds its own Console
        // first (issue 2516).
        if base_console::in_tree(side.tree) {
            builds.run(
                side.name,
                &mut base_console::install(side.tree),
                &format!("installing the {}'s packages", side.name),
            )?;
            builds.run(
                side.name,
                &mut base_console::build(cargo, side.tree, side.target_directory),
                &format!("building the {}'s Console", side.name),
            )?;
        }
    }
    for target in targets {
        builds.run(
            side.name,
            cargo_bench(cargo, side.tree, side.target_directory, target).arg("--no-run"),
            &format!("building the {}'s {target}", side.name),
        )?;
    }
    Ok(())
}

/// Runs the base's build and the head's at the same time, the base's on a
/// thread of its own, and returns what each returned. Each starts its
/// commands through the [`AtOnce`] it is given. A failure is returned as it
/// stands; when both builds fail, both failures are.
fn at_once<B: Send, H>(
    base: impl FnOnce(&AtOnce) -> Result<B> + Send,
    head: impl FnOnce(&AtOnce) -> Result<H>,
) -> Result<(B, H)> {
    let builds = AtOnce::default();
    let (base, head) = std::thread::scope(|scope| {
        let base = scope.spawn(|| builds.track(base(&builds)));
        let head = builds.track(head(&builds));
        let base = base
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        (base, head)
    });
    match (base, head) {
        (Ok(base), Ok(head)) => Ok((base, head)),
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
        (Err(base), Err(head)) if head.is::<Stopped>() => Err(base),
        (Err(base), Err(head)) if base.is::<Stopped>() => Err(head),
        (Err(base), Err(head)) => Err(anyhow!("both builds failed:\n{base:#}\n{head:#}")),
    }
}

/// The builds [`at_once`] runs side by side. Each line a command writes goes
/// out with its side's name before it, so the two builds' lines stay apart
/// in one log, and once either build has failed the other starts no further
/// command.
#[derive(Default)]
struct AtOnce {
    failed: AtomicBool,
}

/// What a command returns instead of starting, once the other build failed.
#[derive(Debug)]
struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("stopped: the other build failed")
    }
}

impl std::error::Error for Stopped {}

impl AtOnce {
    /// Notes a build's failure, which stops the other build at its next
    /// command.
    fn track<T>(&self, result: Result<T>) -> Result<T> {
        if result.as_ref().is_err_and(|error| !error.is::<Stopped>()) {
            self.failed.store(true, Ordering::SeqCst);
        }
        result
    }

    /// Runs `command` for `side`, writing each line of its stdout and stderr
    /// as `[side] <line>` to the same stream, unless the other build failed.
    fn run(&self, side: &str, command: &mut Command, what: &str) -> Result<()> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(Stopped.into());
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("{what}: starting"))?;
        let stdout = child.stdout.take().expect("invariant: stdout is piped");
        let stderr = child.stderr.take().expect("invariant: stderr is piped");
        let (relayed_stdout, relayed_stderr) = std::thread::scope(|scope| {
            let relayed_stdout = scope.spawn(|| relay(stdout, side, output::line));
            let relayed_stderr = relay(stderr, side, output::error);
            let relayed_stdout = relayed_stdout
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            (relayed_stdout, relayed_stderr)
        });
        let status = child.wait().with_context(|| format!("{what}: waiting"))?;
        relayed_stdout.with_context(|| format!("{what}: reading its stdout"))?;
        relayed_stderr.with_context(|| format!("{what}: reading its stderr"))?;
        if !status.success() {
            bail!("{what} failed with {status}");
        }
        Ok(())
    }
}

/// Writes each line `from` gives as `[side] <line>` through `write`, until
/// `from` ends.
fn relay(from: impl Read, side: &str, write: fn(&str)) -> std::io::Result<()> {
    let mut from = BufReader::new(from);
    let mut line = Vec::new();
    while from.read_until(b'\n', &mut line)? > 0 {
        let text = String::from_utf8_lossy(&line);
        write(&format!("[{side}] {}", text.trim_end_matches(['\n', '\r'])));
        line.clear();
    }
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
    measure_at_once("base", &plan.base_targets, &base_report, |target| {
        let mut command = cargo_bench(cargo, &tree, &plan.base_target_directory(), target);
        command
            .env("GUNGRAUN_HOME", &gungraun_home)
            .args(["--", &format!("--save-baseline={BASELINE}")]);
        command
    })?;
    measure_at_once("head", &plan.head_targets, &head_report, |target| {
        let mut command = cargo_bench(cargo, root, &plan.target_directory, target);
        command
            .env("GUNGRAUN_HOME", &gungraun_home)
            .args(["--", &format!("--baseline={BASELINE}")]);
        command
    })?;
    let admission = admit(cargo, root, &plan, &head_report)?;
    let reference_report = plan
        .reference
        .as_ref()
        .map(|reference| measure_reference(cargo, root, &plan, reference))
        .transpose()?;
    let size_reference = if plan.reference.is_some()
        && root.join(SIZE_LIMIT_CONFIG).is_file()
        && reference_tree(root).join(SIZE_LIMIT_CONFIG).is_file()
    {
        Some(size_limit(
            root,
            &reference_tree(root),
            &out.join("size-reference.json"),
        )?)
    } else {
        None
    };
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
    remove_reference_tree(root)?;
    let relative = |path: &Path| path.strip_prefix(root).unwrap_or(path).to_path_buf();
    Ok(Reports {
        base: relative(&base_report),
        head: relative(&head_report),
        admission: admission.as_deref().map(relative),
        size_base: size_base.as_deref().map(relative),
        size_head: size_head.as_deref().map(relative),
        reference: reference_report.as_deref().map(relative),
        size_reference: size_reference.as_deref().map(relative),
    })
}

/// Measures the `side`'s `targets` as many at once as the host has cores, each
/// into a report of its own, then appends the reports to `report` in the
/// order of `targets`: concurrent writers to one file could interleave
/// their summaries. A failed measurement stops the others.
fn measure_at_once(
    side: &str,
    targets: &[String],
    report: &Path,
    command: impl Fn(&str) -> Command,
) -> Result<()> {
    let folder = report.with_extension("parts");
    if folder.exists() {
        std::fs::remove_dir_all(&folder)
            .with_context(|| format!("removing {}", folder.display()))?;
    }
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    let workers = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    let mut parts = Vec::with_capacity(targets.len());
    let mut runs = Runs::default();
    for (index, target) in targets.iter().enumerate() {
        if runs.running.len() >= workers {
            runs.wait_oldest()?;
        }
        let part = folder.join(format!("{index}.jsonl"));
        File::create(&part).with_context(|| format!("creating {}", part.display()))?;
        let what = format!("measuring the {side}'s {target}");
        let child = spawn_measured(&mut command(target), &part, &what)?;
        runs.running.push_back((child, what));
        parts.push(part);
    }
    while !runs.running.is_empty() {
        runs.wait_oldest()?;
    }
    let mut out = std::fs::OpenOptions::new()
        .append(true)
        .open(report)
        .with_context(|| format!("opening {}", report.display()))?;
    for part in &parts {
        let mut input = File::open(part).with_context(|| format!("opening {}", part.display()))?;
        std::io::copy(&mut input, &mut out)
            .with_context(|| format!("appending {}", part.display()))?;
    }
    std::fs::remove_dir_all(&folder).with_context(|| format!("removing {}", folder.display()))?;
    Ok(())
}

/// Measures the reference's targets and returns their report: gungraun's
/// `new` is the reference's figure, so a folder of its own holds no baseline.
fn measure_reference(
    cargo: &OsStr,
    root: &Path,
    plan: &Plan,
    reference: &Reference,
) -> Result<PathBuf> {
    let out = out(root);
    let report = out.join("reference.jsonl");
    File::create(&report).context("creating reference.jsonl")?;
    let gungraun_home = std::path::absolute(out.join("gungraun-reference"))
        .context("resolving gungraun's folder")?;
    let target_directory = plan.target_directory.join(REFERENCE_TARGET);
    for target in &reference.targets {
        measure(
            cargo_bench(cargo, &reference_tree(root), &target_directory, target)
                .env("GUNGRAUN_HOME", &gungraun_home)
                .arg("--"),
            &report,
            &format!("measuring the reference's {target}"),
        )?;
    }
    Ok(report)
}

/// Runs each head target that holds a benchmark the base lacks 20 more
/// times, one report and one gungraun folder a run of a target, several
/// runs at once, and returns the reports' folder, or `None` when no
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
    let gungraun_homes = std::path::absolute(out(root).join("admission-gungraun"))
        .context("resolving gungraun's admission folder")?;
    let workers = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    let mut runs = Runs::default();
    for run in 1..=budget::ADMISSION_RUNS {
        for (index, target) in new_targets.iter().enumerate() {
            if runs.running.len() >= workers {
                runs.wait_oldest()?;
            }
            let name = format!("run-{run:02}-{index}");
            let report = folder.join(format!("{name}.jsonl"));
            File::create(&report).context("creating an admission report")?;
            let what = format!("admission run {run} of {target}");
            let child = spawn_measured(
                cargo_bench(cargo, root, &plan.target_directory, target)
                    .env("GUNGRAUN_HOME", gungraun_homes.join(&name))
                    .arg("--"),
                &report,
                &what,
            )?;
            runs.running.push_back((child, what));
        }
    }
    while !runs.running.is_empty() {
        runs.wait_oldest()?;
    }
    Ok(Some(folder))
}

/// The admission runs still running, oldest first. Dropped before they all
/// finished (a run failed), it stops the others.
#[derive(Default)]
struct Runs {
    running: VecDeque<(Child, String)>,
}

impl Runs {
    fn wait_oldest(&mut self) -> Result<()> {
        let Some((mut child, what)) = self.running.pop_front() else {
            return Ok(());
        };
        let status = child.wait().with_context(|| format!("{what}: waiting"))?;
        if !status.success() {
            bail!("{what} failed with {status}");
        }
        Ok(())
    }
}

impl Drop for Runs {
    fn drop(&mut self) {
        for (child, _) in &mut self.running {
            // A run that already ended has nothing to stop.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
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
        "reference.jsonl",
        "size-reference.json",
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

/// Removes the reference build's worktree, which each build checks out
/// afresh, and its record, and nothing else: the base's stays for the next
/// build, and never a repository-wide `git worktree prune`, which in the
/// container (where only this worktree is mounted) would drop the record of
/// every other unlocked worktree, whose folders it cannot see.
fn remove_reference_tree(root: &Path) -> Result<()> {
    let tree = reference_tree(root);
    if tree.exists() {
        git::text(
            root,
            &["worktree", "remove", "--force", &tree.to_string_lossy()],
        )?;
    }
    Ok(())
}

fn cargo_command(cargo: &OsStr, folder: &Path, target_directory: &Path) -> Command {
    let mut command = crate::child_cargo::command(cargo);
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
    gungraun_output(command, report)?.status_checked(what)
}

/// Starts `command` with gungraun's JSON output, appending its stdout to
/// `report`.
fn spawn_measured(command: &mut Command, report: &Path, what: &str) -> Result<Child> {
    gungraun_output(command, report)?
        .spawn()
        .with_context(|| format!("{what}: starting"))
}

/// `command` set up to write gungraun's JSON summaries to the end of
/// `report`.
pub(crate) fn gungraun_output<'command>(
    command: &'command mut Command,
    report: &Path,
) -> Result<&'command mut Command> {
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(report)
        .with_context(|| format!("opening {}", report.display()))?;
    Ok(command
        .env("GUNGRAUN_OUTPUT_FORMAT", "json")
        .env("GUNGRAUN_NOSUMMARY", "true")
        // A container's default seccomp profile refuses the personality
        // call that turns ASLR off, which only Callgrind's cache simulation
        // is sensitive to; no gated figure uses it, and admission catches
        // any spread.
        .env("GUNGRAUN_ALLOW_ASLR", "yes")
        .stdout(file))
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

#[cfg(test)]
#[cfg(unix)]
mod tests;
