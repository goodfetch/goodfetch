//! `xtask gate`, the incremental local gate (ADR 0135): `just pre-merge all`
//! on a head that descends from a head whose gate already ran reruns only
//! the steps, and the members' tests, that the change since then reaches,
//! by `xtask affected` and by the evidence of what each member read
//! (`evidence`), and every check that judges the whole change; it reuses
//! the other green results. A red result, a result older than a day and a
//! result from another host fingerprint are never reused, a change to the
//! gate's own inputs reruns everything, and a working tree with uncommitted
//! changes neither reuses nor records anything.
//! - **plan**: prints `previous`, `head`, `members` and `unowned` lines,
//!   then each step's action as `<step>\t<run|part|reuse>\t<members>\t<why>`
//!   lines, which `ci/pre-merge.sh` follows; says the same in words on
//!   stderr for the run's log, with the evidence that reaches a member the
//!   selection misses; and writes the plan and the carried results into the
//!   run's record.
//! - **record**: appends a step's result to the run's record, only for the
//!   planned head on a clean tree. A failed step run per member records
//!   green each crate its nextest JUnit files show passed in full, and its
//!   other members red (`attribution`).
//! - **compare**: the backstop's check: from a full run's record, each
//!   result that an incremental run on the same tree reused and the full
//!   run found red, as `miss` lines, or `suspect` lines for a step run per
//!   member, whose red the members that run reran may hold.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

mod attribution;
mod compare;
mod evidence;
mod plan;
mod runs;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};

use crate::affected::{self, Job, Range, RepoPath, WorkspaceMembers};
use crate::environment::Environment;
use crate::{metadata, output};
use evidence::RustInputs;
use plan::{Action, GateRun, Head, Outcome, Planned, Step, Touched, UNOWNED, short, unit};

/// The branch whose merge base with the head is the change's base.
const BASE_BRANCH: &str = "origin/main";

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Status {
    Pass,
    Fail,
}

/// `xtask gate`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Prints which steps this run reruns, over which members, and which it
    /// reuses from an earlier run on an ancestor head, and writes the plan
    /// into `records`.
    Plan {
        /// The folder of the gate's records.
        #[arg(long)]
        records: std::path::PathBuf,
        /// Reuses nothing: every step runs in full, and is recorded.
        #[arg(long)]
        full: bool,
        /// Writes nothing: prints the plan only.
        #[arg(long)]
        dry_run: bool,
        /// Plans against an assumed full green run on this commit, an
        /// ancestor, instead of the records (with `--dry-run`): what a run
        /// would rerun after a green gate there.
        #[arg(long, requires = "dry_run")]
        assume_green: Option<String>,
        /// Now, in seconds since the Unix epoch (`date +%s`): a result older
        /// than a day is not reused.
        #[arg(long)]
        now: u64,
        /// The Cargo target folder whose dep-info shows what each crate
        /// read; Cargo's own by default.
        #[arg(long)]
        target_dir: Option<std::path::PathBuf>,
        /// The variables of the run's environment that change what runs or
        /// how (`CI`, `GOODFETCH_*`, `NEXTEST_*`, `CARGO_*`…), as
        /// `NAME=value` lines: part of the host fingerprint, so a result
        /// counts only for a run in the same environment.
        #[arg(long, default_value = "")]
        environment: String,
    },
    /// Appends a step's result to the record of the run on HEAD, for the
    /// units the plan ran; nothing without a plan for HEAD.
    Record {
        #[arg(long)]
        records: std::path::PathBuf,
        /// The head the plan was made for: HEAD must still be it.
        #[arg(long)]
        head: String,
        /// Now, in seconds since the Unix epoch.
        #[arg(long)]
        now: u64,
        /// The step, by its name in the plan.
        #[arg(long)]
        step: String,
        #[arg(long, value_enum)]
        status: Status,
        /// A failed step run per member's nextest JUnit file, or a folder
        /// of its `nextest*.xml` files, repeatable: each crate that passed
        /// in full by them records green, its other members red
        /// (`attribution`). Without it every member of a failed step is red.
        #[arg(long)]
        junit: Vec<std::path::PathBuf>,
        /// The stress run's selection, its `<count>\t<runner>\t<test ID>`
        /// lines: a crate passed in full when each of its selected nextest
        /// tests passed its count of times in the `--junit` files.
        #[arg(long)]
        selection: Option<std::path::PathBuf>,
    },
    /// Prints each result an incremental run on the full run's tree reused
    /// and the full run on `full` found red: the backstop's check.
    Compare {
        /// The folder of the full run's record.
        #[arg(long)]
        records: std::path::PathBuf,
        /// The folder of the incremental runs to compare it with, as the
        /// backstop reads `just merge`'s; `records` by default.
        #[arg(long)]
        runs: Option<std::path::PathBuf>,
        /// The head of the full run (`GOODFETCH_GATE_FULL=1`).
        #[arg(long)]
        full: String,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    match command {
        Command::Plan {
            records,
            full,
            dry_run,
            assume_green,
            now,
            target_dir,
            environment: variables,
        } => {
            let records = environment.path(&records);
            let target = match target_dir {
                Some(folder) => environment.path(&folder),
                None => metadata::target_directory(&environment.cargo, root)?,
            };
            plan_run(
                environment,
                &records,
                &Options {
                    full,
                    dry_run,
                    assume_green,
                    now,
                    target,
                    variables,
                },
            )?;
        }
        Command::Record {
            records,
            head,
            now,
            step,
            status,
            junit,
            selection,
        } => {
            let Some(step) = Step::from_name(&step) else {
                bail!("no gate step is named {step}");
            };
            // The step's crates are read only for a result they can change.
            let evidence = if status == Status::Fail
                && step.per_member()
                && (!junit.is_empty() || selection.is_some())
            {
                let members = WorkspaceMembers::read(&environment.cargo, root)?;
                let crates = members
                    .all()
                    .filter(|(_, kind)| *kind == affected::Kind::Crate)
                    .map(|(name, _)| name)
                    .collect();
                // The Pre-merge suite judges unreferenced insta snapshots,
                // which cargo-insta does only once every test passed.
                let unjudged = if step == Step::PreMerge {
                    snapshot_holders(root, &members)?
                } else {
                    BTreeSet::new()
                };
                Some(Evidence {
                    junit: junit.iter().map(|path| environment.path(path)).collect(),
                    selection: selection.map(|path| environment.path(&path)),
                    crates,
                    unjudged,
                })
            } else {
                None
            };
            record_run(
                root,
                &environment.path(&records),
                &Recorded {
                    head,
                    now,
                    step,
                    status,
                    evidence,
                },
            )?;
        }
        Command::Compare {
            records,
            runs,
            full,
        } => {
            let records = environment.path(&records);
            let runs = runs.map_or_else(|| records.clone(), |runs| environment.path(&runs));
            return compare::compare(root, &records, &runs, &full);
        }
    }
    Ok(ExitCode::SUCCESS)
}

struct Options {
    full: bool,
    dry_run: bool,
    assume_green: Option<String>,
    now: u64,
    target: std::path::PathBuf,
    variables: String,
}

fn plan_run(environment: &Environment, records: &Path, options: &Options) -> Result<()> {
    let root = environment.root.as_path();
    let head = commit(root, "HEAD")?;
    let plan_file = runs::plan_file(records, &head);
    let dirty = !git(root, &["status", "--porcelain"])?.is_empty();
    if dirty && !options.dry_run {
        // Nothing this run gives holds for HEAD, so nothing is recorded.
        if plan_file.exists() {
            std::fs::remove_file(&plan_file)
                .with_context(|| format!("removing {}", plan_file.display()))?;
        }
        output::error(
            "Gate (ADR 0135): the working tree has uncommitted changes, so every step runs in full and nothing is recorded.",
        );
        output::line("previous\tnone");
        return Ok(());
    }
    let base = commit(root, &git(root, &["merge-base", BASE_BRANCH, "HEAD"])?)?;
    let host = host_fingerprint(root, &environment.cargo, &options.variables);
    let members_read = WorkspaceMembers::read(&environment.cargo, root)?;
    let unowned = evidence::unowned_tests(root, &members_read)?;
    let (members, shuffled) = head_members(&members_read, &unowned);
    let (previous, no_previous) = if options.full {
        (
            None,
            "a full run was asked for (GOODFETCH_GATE_FULL)".to_owned(),
        )
    } else if let Some(assumed) = &options.assume_green {
        let assumed = commit(root, assumed)?;
        let record = assumed_green(
            root,
            &assumed,
            &Head {
                base: &base,
                members: &members,
                shuffled: &shuffled,
                host: &host,
                now: options.now,
            },
        )?;
        (Some(record), String::new())
    } else {
        match nearest(root, records, &head)? {
            Some(record) => (Some(record), String::new()),
            None => (
                None,
                "no earlier gate run on this head or an ancestor".to_owned(),
            ),
        }
    };
    let reader = Reader {
        root,
        cargo: &environment.cargo,
        members: &members_read,
        head: &head,
        target: &options.target,
    };
    let (touched, beyond) = match &previous {
        Some(previous) => reader.touched_by_origin(previous)?,
        None => (BTreeMap::new(), Vec::new()),
    };
    let planned = plan::plan(
        &Head {
            base: &base,
            members: &members,
            shuffled: &shuffled,
            host: &host,
            now: options.now,
        },
        previous.as_ref(),
        &no_previous,
        &touched,
    );
    let previous_head = previous.as_ref().map(|record| record.head.as_str());
    let machine = render(previous_head, &head, &members, &unowned, &planned);
    let mut log = describe(previous_head, &planned);
    if !beyond.is_empty() {
        log.push_str(
            "Evidence reaches these members, which `xtask affected` does not select (they rerun):\n",
        );
        log.push_str(&beyond.join("\n"));
        log.push('\n');
    }
    output::error(&log);
    output::line(machine.trim_end());
    if options.dry_run {
        return Ok(());
    }
    let carried = GateRun {
        head,
        base,
        host,
        units: planned
            .iter()
            .flat_map(|planned| planned.carried.clone())
            .collect(),
    };
    write(records, &carried, &machine)
}

/// Writes the run's record, its carried results, and its plan.
fn write(records: &Path, carried: &GateRun, plan: &str) -> Result<()> {
    std::fs::create_dir_all(records).with_context(|| format!("creating {}", records.display()))?;
    let record_file = runs::file(records, &carried.head);
    std::fs::write(&record_file, runs::render(carried))
        .with_context(|| format!("writing {}", record_file.display()))?;
    let plan_file = runs::plan_file(records, &carried.head);
    std::fs::write(&plan_file, plan).with_context(|| format!("writing {}", plan_file.display()))
}

/// Every member of the head's workspaces, with [`UNOWNED`] when some test
/// belongs to none, and those whose tests Vitest, Playwright or pytest run,
/// ordered by the head's seed: every run reruns them in that order.
fn head_members(
    members: &WorkspaceMembers,
    unowned: &[String],
) -> (BTreeSet<String>, BTreeSet<String>) {
    let unowned = (!unowned.is_empty()).then(|| UNOWNED.to_owned());
    let all = members
        .all()
        .map(|(name, _)| name)
        .chain(unowned.clone())
        .collect();
    let shuffled = members
        .all()
        .filter(|(_, kind)| *kind != affected::Kind::Crate)
        .map(|(name, _)| name)
        .chain(unowned)
        .collect();
    (all, shuffled)
}

/// What reads the change since a head: the workspace and its evidence.
struct Reader<'a> {
    root: &'a Path,
    cargo: &'a OsStr,
    members: &'a WorkspaceMembers,
    head: &'a str,
    target: &'a Path,
}

impl Reader<'_> {
    /// What the change since each head a green result of `previous` ran on
    /// reaches, by that head, and the log's lines for the members the
    /// evidence reaches beyond the selection.
    fn touched_by_origin(
        &self,
        previous: &GateRun,
    ) -> Result<(BTreeMap<String, Touched>, Vec<String>)> {
        let origins: BTreeSet<&String> = previous
            .units
            .values()
            .filter(|outcome| outcome.green)
            .map(|outcome| &outcome.origin)
            .collect();
        let mut rust = None;
        let mut touched = BTreeMap::new();
        let mut beyond = Vec::new();
        for origin in origins {
            let (since, missed) = self.touched_since(origin, &mut rust)?;
            beyond.extend(missed.into_iter().map(|(member, evidence)| {
                format!("  {member}, since {}: {evidence}", short(origin))
            }));
            touched.insert(origin.clone(), since);
        }
        Ok((touched, beyond))
    }

    /// What the change from `origin` to the head reaches: `xtask
    /// affected`'s selection, widened to the members the evidence of what
    /// each read reaches, with those the selection alone misses; a change
    /// git cannot read reruns whatever ran on `origin`. `rust` keeps the
    /// dep-info, read once.
    fn touched_since(
        &self,
        origin: &str,
        rust: &mut Option<RustInputs>,
    ) -> Result<(Touched, BTreeMap<String, String>)> {
        let range = Range::Commits {
            base: origin.to_owned(),
            head: self.head.to_owned(),
        };
        let paths = match affected::changed_paths(self.root, &range) {
            Ok(paths) => paths,
            Err(error) => {
                return Ok((
                    Touched {
                        gate_input: Some(format!("the history git cannot read ({error:#})")),
                        ..Touched::default()
                    },
                    BTreeMap::new(),
                ));
            }
        };
        let selection = self.members.select(&paths);
        if paths.is_empty() || plan::gate_input(&paths).is_some() {
            return Ok((Touched::new(&paths, &selection), BTreeMap::new()));
        }
        let mut found = BTreeMap::new();
        if rust.is_none() {
            *rust = Some(evidence::rust_inputs(
                self.root,
                self.cargo,
                self.target,
                self.members,
            )?);
        }
        if let Some(rust) = rust {
            let changed: BTreeSet<String> =
                paths.iter().map(|path| path.as_str().to_owned()).collect();
            found.extend(rust.reached(self.members, &changed));
        }
        for (member, why) in affected::literal_readers(self.root, self.members, &paths)? {
            found.entry(member).or_insert(why);
        }
        // A member whose build or files read a changed file reaches its
        // dependants as a change to its own files would. The runners'
        // listings name every test they reach, in whichever member, so
        // their members are added as they are.
        let mut widened = paths.clone();
        widened.extend(
            found
                .keys()
                .filter_map(|member| self.members.folder(member))
                .map(|folder| RepoPath::new(format!("{folder}/.evidence"))),
        );
        let listed = evidence::listed_since(self.root, self.cargo, origin, self.members);
        // A listed member's visual tests run in the visual job alone, which
        // the host's listings do not cover.
        let visual = listed
            .keys()
            .any(|member| self.members.owns_a_visual_test(member));
        for (member, why) in listed {
            found.entry(member).or_insert(why);
        }
        let selected = selection.members();
        let missed = found
            .iter()
            .filter(|(member, _)| !selected.contains(*member))
            .map(|(member, why)| (member.clone(), why.clone()))
            .collect();
        let mut touched = Touched::new(&paths, &self.members.select(&widened));
        if visual {
            touched.jobs.insert(Job::Visual);
        }
        touched.evidence = found;
        Ok((touched, missed))
    }
}

/// The fingerprint of this host and the tools the gate's steps run: the
/// kernel, the system's packages, the runtimes, the test runners, the
/// container engines and the toolchain, and the run's environment
/// `variables`. A command that is missing counts as such. Java is asked
/// without its performance data file: when `/tmp` is full, the JVM warns on
/// stderr that it cannot write it, naming its own process ID, and each
/// probe would then fingerprint another host (FQ50).
fn host_fingerprint(root: &Path, cargo: &OsStr, variables: &str) -> String {
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
    let mut probe = |command: &mut Process| match command.current_dir(root).output() {
        Ok(output) => {
            digest.update(output.status.code().unwrap_or(-1).to_le_bytes());
            digest.update(&output.stdout);
            digest.update(&output.stderr);
        }
        Err(_) => digest.update(b"missing"),
    };
    for arguments in PROBES {
        let (program, rest) = arguments.split_first().unwrap_or((&"true", &[]));
        probe(Process::new(program).args(rest));
    }
    probe(Process::new(cargo).args(["nextest", "--version"]));
    digest.update(variables.as_bytes());
    format!("{:x}", digest.finalize())
}

/// A full green run on `assumed` on this host, now, for a dry run.
fn assumed_green(root: &Path, assumed: &str, head: &Head<'_>) -> Result<GateRun> {
    let tree = git(
        root,
        &["rev-parse", "--verify", &format!("{assumed}^{{tree}}")],
    )?;
    let outcome = Outcome {
        green: true,
        origin: assumed.to_owned(),
        tree,
        at: head.now,
    };
    let mut units = BTreeMap::new();
    for step in Step::ALL {
        if step.per_member() {
            for member in head.members {
                units.insert(unit(step, Some(member)), outcome.clone());
            }
        } else {
            units.insert(unit(step, None), outcome.clone());
        }
    }
    Ok(GateRun {
        head: assumed.to_owned(),
        base: head.base.to_owned(),
        host: head.host.to_owned(),
        units,
    })
}

/// The record of the nearest run on `head` or one of its ancestors: the
/// fewest commits away.
fn nearest(root: &Path, records: &Path, head: &str) -> Result<Option<GateRun>> {
    let mut nearest: Option<(u64, String)> = None;
    for candidate in runs::heads(records)? {
        let ancestor = Process::new("git")
            .args([
                "merge-base",
                "--is-ancestor",
                "--end-of-options",
                &candidate,
                head,
            ])
            .current_dir(root)
            .output()
            .context("running git")?
            .status
            .success();
        if !ancestor {
            continue;
        }
        let distance: u64 = git(
            root,
            &["rev-list", "--count", &format!("{candidate}..{head}")],
        )?
        .parse()
        .context("reading git rev-list's count")?;
        if nearest.as_ref().is_none_or(|(best, _)| distance < *best) {
            nearest = Some((distance, candidate));
        }
    }
    nearest
        .map(|(_, candidate)| runs::read(records, &candidate))
        .transpose()
}

/// The plan as `ci/pre-merge.sh` and `xtask gate record` read it.
fn render(
    previous: Option<&str>,
    head: &str,
    members: &BTreeSet<String>,
    unowned: &[String],
    planned: &[Planned],
) -> String {
    let joined = |members: &BTreeSet<String>| {
        if members.is_empty() {
            "-".to_owned()
        } else {
            members.iter().cloned().collect::<Vec<_>>().join(" ")
        }
    };
    let mut text = format!("previous\t{}\nhead\t{head}\n", previous.unwrap_or("none"));
    let _ = writeln!(text, "members\t{}", joined(members));
    let unowned = if unowned.is_empty() {
        "-".to_owned()
    } else {
        unowned.join(" ")
    };
    let _ = writeln!(text, "unowned\t{unowned}");
    for Planned { step, action, .. } in planned {
        let (name, units, why) = match action {
            Action::Run { why } => ("run", "-".to_owned(), why.clone()),
            Action::Part { members, why } => ("part", joined(members), why.clone()),
            Action::Reuse { origins } => (
                "reuse",
                "-".to_owned(),
                format!("green on {}", joined(origins)),
            ),
        };
        let _ = writeln!(text, "{}\t{name}\t{units}\t{why}", step.name());
    }
    text
}

/// The plan in words, for the run's log: each step rerun or reused, and
/// from which run and head.
fn describe(previous: Option<&str>, planned: &[Planned]) -> String {
    let mut text = match previous {
        Some(previous) => format!(
            "Gate (ADR 0135): the nearest earlier gate run is the run on {previous}; each step:\n"
        ),
        None => "Gate (ADR 0135): no earlier result is reused; each step:\n".to_owned(),
    };
    for Planned {
        step,
        action,
        carried,
    } in planned
    {
        let line = match action {
            Action::Run { why } => format!("rerun in full ({why})"),
            Action::Part { members, why } => format!(
                "rerun for {} of its members: {} ({why}); the other {} reused, green on {}",
                members.len(),
                members.iter().cloned().collect::<Vec<_>>().join(" "),
                carried.len(),
                origins(carried)
            ),
            Action::Reuse { .. } => format!(
                "reused, green on {} (carried by the run on {})",
                origins(carried),
                previous.map_or("-", short)
            ),
        };
        let _ = writeln!(text, "  {}: {line}", step.name());
    }
    text
}

fn origins(carried: &BTreeMap<String, Outcome>) -> String {
    let origins: BTreeSet<&str> = carried
        .values()
        .map(|outcome| short(&outcome.origin))
        .collect();
    origins.into_iter().collect::<Vec<_>>().join(" ")
}

/// A step's result, for the head its plan was made for.
struct Recorded {
    head: String,
    now: u64,
    step: Step,
    status: Status,
    /// What a failed step run per member's crates are judged by.
    evidence: Option<Evidence>,
}

/// A failed step's nextest files and, for the stress run, its selection,
/// with the workspace's crates (`attribution`).
struct Evidence {
    junit: Vec<std::path::PathBuf>,
    selection: Option<std::path::PathBuf>,
    crates: BTreeSet<String>,
    /// The crates a red run of the step does not judge in full whatever
    /// their tests did: after a red Pre-merge suite, those holding insta
    /// snapshots, whose unreferenced snapshots cargo-insta judges only
    /// once every test passed.
    unjudged: BTreeSet<String>,
}

/// The crates whose folders hold a committed insta snapshot (`.snap`).
fn snapshot_holders(root: &Path, members: &WorkspaceMembers) -> Result<BTreeSet<String>> {
    let crates: BTreeSet<String> = members
        .all()
        .filter(|(_, kind)| *kind == affected::Kind::Crate)
        .map(|(name, _)| name)
        .collect();
    Ok(git(root, &["ls-files", "--", "*.snap"])?
        .lines()
        .filter_map(|path| members.owner(path))
        .filter(|owner| crates.contains(owner))
        .collect())
}

/// Appends a step's result to the record of the run on the planned head,
/// for the units its plan ran. HEAD must still be that head and the tree
/// clean, else the result holds for another tree: the plan is dropped, so
/// nothing more is recorded, and the step fails.
fn record_run(root: &Path, records: &Path, recorded: &Recorded) -> Result<()> {
    let Recorded {
        head,
        now,
        step,
        status,
        evidence,
    } = recorded;
    let plan_file = runs::plan_file(records, head);
    let Ok(plan) = std::fs::read_to_string(&plan_file) else {
        output::error(&format!(
            "Gate: no plan for {head}, so {} is not recorded",
            step.name()
        ));
        return Ok(());
    };
    let checked_out = commit(root, "HEAD")?;
    let dirty = !git(root, &["status", "--porcelain"])?.is_empty();
    if &checked_out != head || dirty {
        std::fs::remove_file(&plan_file)
            .with_context(|| format!("removing {}", plan_file.display()))?;
        bail!(
            "the gate planned {head} on a clean tree, but HEAD is {checked_out}{}: {} is not recorded, and nothing more of this run is",
            if dirty {
                " with uncommitted changes"
            } else {
                ""
            },
            step.name()
        );
    }
    let field = |key: &str| -> Option<Vec<&str>> {
        plan.lines()
            .find(|line| line.split('\t').next() == Some(key))
            .map(|line| line.split('\t').skip(1).collect())
    };
    let words = |text: &str| -> Vec<String> {
        text.split(' ')
            .filter(|word| !word.is_empty() && *word != "-")
            .map(str::to_owned)
            .collect()
    };
    let line = field(step.name()).with_context(|| {
        format!(
            "the plan in {} has no {} line",
            plan_file.display(),
            step.name()
        )
    })?;
    let members = if step.per_member() {
        match line.first().copied() {
            Some("run") => words(field("members").unwrap_or_default().first().unwrap_or(&"")),
            Some("part") => words(line.get(1).unwrap_or(&"")),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let passed = match evidence {
        Some(evidence) if *status == Status::Fail => passed_in_full(*step, &members, evidence),
        _ => BTreeSet::new(),
    };
    let tree = git(root, &["rev-parse", "--verify", "HEAD^{tree}"])?;
    let outcome = |green: bool| Outcome {
        green,
        origin: head.clone(),
        tree: tree.clone(),
        at: *now,
    };
    let text: String = if step.per_member() {
        members
            .iter()
            .map(|member| {
                let green = *status == Status::Pass || passed.contains(member);
                runs::line(&unit(*step, Some(member)), &outcome(green))
            })
            .collect()
    } else {
        runs::line(&unit(*step, None), &outcome(*status == Status::Pass))
    };
    output::append(&runs::file(records, head), &text)
}

/// The members of a failed step that passed in full: its crates the
/// evidence shows passed, or none when it does not account for the failure.
/// Says which in the run's log.
fn passed_in_full(step: Step, members: &[String], evidence: &Evidence) -> BTreeSet<String> {
    let crates: BTreeSet<String> = members
        .iter()
        .filter(|member| evidence.crates.contains(*member))
        .cloned()
        .collect();
    match attribution::attribute(&crates, &evidence.junit, evidence.selection.as_deref()) {
        attribution::Attribution::Passed(passed) => {
            let unjudged: BTreeSet<String> =
                passed.intersection(&evidence.unjudged).cloned().collect();
            let passed: BTreeSet<String> = passed.difference(&unjudged).cloned().collect();
            let failed: BTreeSet<String> = crates
                .difference(&passed)
                .filter(|krate| !unjudged.contains(*krate))
                .cloned()
                .collect();
            output::error(&format!(
                "Gate: {} failed in {}; its other crates passed in full and are recorded green: {}",
                step.name(),
                listed(&failed),
                listed(&passed)
            ));
            if !unjudged.is_empty() {
                output::error(&format!(
                    "Gate: {} hold insta snapshots, which cargo-insta judges unreferenced only once every test passed, so they are recorded red",
                    listed(&unjudged)
                ));
            }
            passed
        }
        attribution::Attribution::Unattributed(why) => {
            output::error(&format!(
                "Gate: {} failed and {why}, so each of its members is recorded red",
                step.name()
            ));
            BTreeSet::new()
        }
    }
}

/// Crates as the log lists them: joined by spaces, or `none`.
fn listed(crates: &BTreeSet<String>) -> String {
    if crates.is_empty() {
        "none".to_owned()
    } else {
        crates.iter().cloned().collect::<Vec<_>>().join(" ")
    }
}

/// The commit `revision` names, in full.
fn commit(root: &Path, revision: &str) -> Result<String> {
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )
}

/// Runs git with `args` in `root`, and returns its trimmed output.
fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Process::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("running git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)
        .context("reading git's output")?
        .trim()
        .to_owned())
}

#[cfg(test)]
mod tests;
