//! `xtask gate`, the incremental local gate (ADR 0135): `just pre-merge all`
//! on a head that descends from a head whose gate already ran reruns only
//! the steps, and the members' tests, that the change since then reaches,
//! by `xtask affected` and by the evidence of what each member read
//! (`evidence`), and every check that judges the whole change; it reuses
//! the other green results. A red result, a result older than a day and a
//! result from another host fingerprint are never reused, a change to the
//! gate's own inputs reruns everything, and a working tree with uncommitted
//! changes neither reuses nor records anything.
//! - **steps**: prints what a form of `just pre-merge` runs, in its order,
//!   and the `just` command of each step (`steps`), which `ci/pre-merge.sh`
//!   follows.
//! - **plan**: prints the `previous`, `head`, `members` and `unowned` lines,
//!   then each step's `<step>.action` (`run`, `part` or `reuse`),
//!   `<step>.members` and `<step>.why` lines, which `ci/pre-merge.sh` reads
//!   through `ci/output-value.sh` and follows; says the same in words on
//!   stderr for the run's log, with the evidence that reaches a member the
//!   selection misses; and writes the plan and the carried results into the
//!   run's record.
//! - **record**: appends a step's result to the run's record, only for the
//!   planned head on a clean tree. A failed step run per member records
//!   green each crate its nextest JUnit files show passed in full, and its
//!   other members red (`attribution`). A red stress run also leaves each
//!   crate short of its counts, with no failing test, its clean passes
//!   ([`plan::EarnedPasses`]), which the plan keeps while a green result would
//!   hold.
//! - **compare**: the backstop's check: from a full run's record, each
//!   result that an incremental run on the same tree reused and the full
//!   run found red, as `miss` lines, or `suspect` lines for a step run per
//!   member, whose red the members that run reran may hold; and, whatever
//!   the full run found and whatever its tree, each result of a merge since
//!   the backstop's last green run that counted earned passes, as `earned`
//!   lines.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

mod attribution;
mod compare;
mod evidence;
mod plan;
mod runs;
mod steps;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command as Process, ExitCode};
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};

use crate::affected::{self, Job, Range, RepoPath, WorkspaceMembers};
use crate::environment::Environment;
use crate::{git, metadata, output};
use attribution::{Evidence, Judged, StressEvidence};
use evidence::RustInputs;
use plan::{Action, GateRun, Head, Outcome, Planned, Step, Touched, UNOWNED, short, unit};

/// The branch whose merge base with the head is the change's base.
const BASE_BRANCH: &str = "origin/main";

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Status {
    Pass,
    Fail,
}

/// Each test's clean passes in the earned file `xtask gate plan` wrote, which
/// `xtask stress select` takes from their counts; none when it is missing or
/// cannot be read, and none of a member's past the age limit by now, since the
/// stress run reads the file later than the plan that kept them.
pub(crate) fn earned_passes(file: &Path) -> BTreeMap<String, u32> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(u64::MAX, |since_the_epoch| since_the_epoch.as_secs());
    runs::read_earned(file)
        .into_values()
        .filter(|earned| !plan::past_max_age(now, earned.at))
        .flat_map(|earned| earned.passes)
        .collect()
}

/// `xtask gate`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Prints the steps a form of `just pre-merge` runs, in their order,
    /// with the `just` command of each.
    Steps {
        #[arg(long, value_enum)]
        form: steps::Form,
    },

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
        #[arg(long, requires_all = ["red_commands", "started"])]
        selection: Option<std::path::PathBuf>,
        /// The stress run's `<runner>\t<status>` lines, one a command that
        /// went red; with `--selection`. A red run the `--junit` files do not
        /// account for leaves no earned passes.
        #[arg(long, requires = "selection")]
        red_commands: Option<std::path::PathBuf>,
        /// When the stress run began, in seconds since the Unix epoch; with
        /// `--selection`. The passes it gives age from here, which is no later
        /// than the first of them.
        #[arg(long, requires = "selection")]
        started: Option<u64>,
    },
    /// Prints, as a JSON array, each result an incremental run on the full
    /// run's tree reused and the full run on `full` did not find green, with
    /// its kind (`miss`, `suspect`, `unchecked`), unit, step and member; each
    /// result of a run merged since `since` that counted earned passes
    /// (`earned`), whatever its tree, with its run's base; and each such run
    /// whose record cannot be read (`unreadable`): the backstop's check.
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
        /// The commit of the backstop's last green run that `full` holds: what
        /// it holds has been checked. With none, every run `full` holds counts.
        #[arg(long)]
        since: Option<String>,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    match command {
        Command::Steps { form } => output::line(steps::render(form).trim_end()),
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
            red_commands,
            started,
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
                    stress: selection
                        .zip(red_commands)
                        .map(|(selection, red_commands)| StressEvidence {
                            selection: environment.path(&selection),
                            red_commands: environment.path(&red_commands),
                        }),
                    started,
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
            since,
        } => {
            let records = environment.path(&records);
            let runs = runs.map_or_else(|| records.clone(), |runs| environment.path(&runs));
            return compare::compare(root, &records, &runs, &full, since.as_deref());
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
    let head = git::commit(root, "HEAD")?;
    let plan_file = runs::plan_file(records, &head);
    let dirty = !git::line(root, &["status", "--porcelain"])?.is_empty();
    if dirty && !options.dry_run {
        return plan_dirty(&plan_file);
    }

    let base = git::commit(
        root,
        &git::line(root, &["merge-base", BASE_BRANCH, "HEAD"])?,
    )?;
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
        let assumed = git::commit(root, assumed)?;
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
        earned: planned
            .iter()
            .flat_map(|planned| planned.earned.clone())
            .collect(),
    };
    write(records, &carried, &machine)
}

/// The plan of a run on a working tree with uncommitted changes: nothing
/// it gives holds for HEAD, so every step runs in full, nothing is
/// recorded, and an earlier plan for HEAD is dropped.
fn plan_dirty(plan_file: &Path) -> Result<()> {
    if plan_file.exists() {
        std::fs::remove_file(plan_file)
            .with_context(|| format!("removing {}", plan_file.display()))?;
    }
    let why = "the working tree has uncommitted changes";
    output::error(&format!(
        "Gate (ADR 0135): {why}, so every step runs in full and nothing is recorded."
    ));
    let planned: Vec<Planned> = Step::ALL
        .into_iter()
        .map(|step| Planned {
            step,
            action: Action::Run {
                why: why.to_owned(),
            },
            carried: BTreeMap::new(),
            earned: BTreeMap::new(),
        })
        .collect();
    let machine = render(None, NO_HEAD, &BTreeSet::new(), &[], &planned);
    output::line(machine.trim_end());
    Ok(())
}

/// Writes the run's record, its carried results and earned passes, and its plan.
fn write(records: &Path, carried: &GateRun, plan: &str) -> Result<()> {
    std::fs::create_dir_all(records).with_context(|| format!("creating {}", records.display()))?;
    let record_file = runs::file(records, &carried.head);
    std::fs::write(&record_file, runs::render(carried))
        .with_context(|| format!("writing {}", record_file.display()))?;
    runs::write_earned(records, &carried.head, &carried.earned)?;
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
            .chain(previous.earned.values().map(|earned| &earned.origin))
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
        let selection = self.members.select(&plan::selected(&paths));
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
        for (member, why) in affected::literal_readers(
            self.root,
            self.members,
            &paths,
            &affected::ChangedParts::default(),
        )? {
            found.entry(member).or_insert(why);
        }
        // A member whose build or files read a changed file reaches its
        // dependants as a change to its own files would. The runners'
        // listings name every test they reach, in whichever member, so
        // their members are added as they are. xtask has no dependant and no
        // benchmark, and the crate builds read no file of its tests, so
        // what reaches it reaches its tests alone.
        let mut widened = plan::selected(&paths);
        widened.extend(
            found
                .keys()
                .filter(|member| *member != plan::XTASK)
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
        // `just linux` runs some of xtask's tests on a host that is not Linux.
        if found.contains_key(plan::XTASK) {
            touched.steps.insert(Step::Linux);
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
    probe(crate::child_cargo::command(cargo).args(["nextest", "--version"]));
    digest.update(variables.as_bytes());
    format!("{:x}", digest.finalize())
}

/// A full green run on `assumed` on this host, now, for a dry run.
fn assumed_green(root: &Path, assumed: &str, head: &Head<'_>) -> Result<GateRun> {
    let tree = git::line(
        root,
        &["rev-parse", "--verify", &format!("{assumed}^{{tree}}")],
    )?;
    let outcome = Outcome {
        green: true,
        origin: assumed.to_owned(),
        tree,
        at: head.now,
        earned_on: None,
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
        earned: BTreeMap::new(),
    })
}

/// The record of the nearest run on `head` or one of its ancestors: the
/// fewest commits away.
fn nearest(root: &Path, records: &Path, head: &str) -> Result<Option<GateRun>> {
    let mut nearest: Option<(u64, String)> = None;
    for candidate in runs::heads(records)? {
        let ancestor = git::succeeds(
            root,
            &[
                "merge-base",
                "--is-ancestor",
                "--end-of-options",
                &candidate,
                head,
            ],
        )?;
        if !ancestor {
            continue;
        }
        let distance: u64 = git::line(
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

/// The plan's `head` when the run records nothing: its working tree has
/// uncommitted changes.
const NO_HEAD: &str = "none";

/// The plan as `ci/pre-merge.sh` reads it, through `ci/output-value.sh`,
/// and `xtask gate record` (`plan_value`): the `previous`, `head`, `members`
/// and `unowned` lines, then each step's `<step>.action` (`run`, `part` or
/// `reuse`), `<step>.members` and `<step>.why` lines, with `<step>.earned`, the
/// members that keep passes, for a step that counts them. A list is its items
/// joined by spaces, `-` when it is empty; a value is one line.
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
    let mut text = String::new();
    let mut line = |key: &str, value: &str| {
        let _ = writeln!(text, "{key}={}", value.replace('\n', " "));
    };
    line("previous", previous.unwrap_or("none"));
    line("head", head);
    line("members", &joined(members));
    line(
        "unowned",
        &if unowned.is_empty() {
            "-".to_owned()
        } else {
            unowned.join(" ")
        },
    );
    for Planned {
        step,
        action,
        earned,
        ..
    } in planned
    {
        let (name, units, why) = match action {
            Action::Run { why } => ("run", "-".to_owned(), why.clone()),
            Action::Part { members, why } => ("part", joined(members), why.clone()),
            Action::Reuse { origins } => (
                "reuse",
                "-".to_owned(),
                format!("green on {}", joined(origins)),
            ),
        };
        if step.counts_passes() {
            let kept = earned.keys().cloned().collect();
            line(&format!("{}.earned", step.name()), &joined(&kept));
        }
        let step = step.name();
        line(&format!("{step}.action"), name);
        line(&format!("{step}.members"), &units);
        line(&format!("{step}.why"), &why);
    }
    text
}

/// The value of the plan's one `<key>=` line, as `ci/output-value.sh`
/// reads it: a plan without the key, or with it twice, is an error.
fn plan_value<'a>(plan: &'a str, key: &str) -> Result<&'a str> {
    let prefix = format!("{key}=");
    let mut values = plan.lines().filter_map(|line| line.strip_prefix(&prefix));
    match (values.next(), values.next()) {
        (Some(value), None) => Ok(value),
        (None, _) => bail!("the plan has no `{key}=` line"),
        (Some(_), Some(_)) => bail!("the plan has more than one `{key}=` line"),
    }
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
        earned,
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
        for (member, passes) in earned {
            let _ = writeln!(
                text,
                "  {}: {member} keeps the clean passes of its {} tests, begun on {}; each runs what its count still owes",
                step.name(),
                passes.passes.len(),
                short(&passes.origin)
            );
        }
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

/// The crates whose folders hold a committed insta snapshot (`.snap`).
fn snapshot_holders(root: &Path, members: &WorkspaceMembers) -> Result<BTreeSet<String>> {
    let crates: BTreeSet<String> = members
        .all()
        .filter(|(_, kind)| *kind == affected::Kind::Crate)
        .map(|(name, _)| name)
        .collect();
    Ok(git::line(root, &["ls-files", "--", "*.snap"])?
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
    let checked_out = git::commit(root, "HEAD")?;
    let dirty = !git::line(root, &["status", "--porcelain"])?.is_empty();
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
    let value = |key: &str| {
        plan_value(&plan, key).with_context(|| format!("reading {}", plan_file.display()))
    };
    let words = |text: &str| -> Vec<String> {
        text.split(' ')
            .filter(|word| !word.is_empty() && *word != "-")
            .map(str::to_owned)
            .collect()
    };
    let action = value(&format!("{}.action", step.name()))?;
    let members = if step.per_member() {
        match action {
            "run" => words(value("members")?),
            "part" => words(value(&format!("{}.members", step.name()))?),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let judged = match evidence {
        Some(evidence) if *status == Status::Fail => evidence.judge(*step, &members),
        _ => Judged::default(),
    };
    let earned = if step.counts_passes() {
        let kept = words(value(&format!("{}.earned", step.name()))?);
        runs::earned_the_plan_kept(records, head, &kept)
            .with_context(|| format!("{} is not recorded", step.name()))?
    } else {
        BTreeMap::new()
    };
    let tree = git::line(root, &["rev-parse", "--verify", "HEAD^{tree}"])?;
    let outcome = |member: Option<&str>, green: bool| {
        // A green result that counted earlier passes is as old as they are.
        let counted = member
            .and_then(|member| earned.get(member))
            .filter(|_| green);
        Outcome {
            green,
            origin: head.clone(),
            tree: tree.clone(),
            at: counted.map_or(*now, |passes| passes.at),
            earned_on: counted.map(|passes| passes.origin.clone()),
        }
    };
    let text: String = if step.per_member() {
        members
            .iter()
            .map(|member| {
                let green = *status == Status::Pass || judged.passed.contains(member);
                runs::line(
                    &unit(*step, Some(member)),
                    &outcome(Some(member.as_str()), green),
                )
            })
            .collect()
    } else {
        runs::line(&unit(*step, None), &outcome(None, *status == Status::Pass))
    };
    output::append(&runs::file(records, head), &text)?;
    if step.counts_passes() {
        let started = evidence.as_ref().and_then(|evidence| evidence.started);
        let kept = runs::add_earned(&earned, judged.clean, head, started.unwrap_or(*now));
        runs::write_earned(records, head, &kept)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
