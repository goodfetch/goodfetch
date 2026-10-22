//! `xtask gate`, the incremental local gate (ADR 0135): `just pre-merge all`
//! on a head that descends from a head whose gate already ran reruns only
//! the steps, and the members' tests, that `xtask affected` finds the change
//! since then reaches, and every check that judges the whole change; it
//! reuses the other green results. A red result is never reused, a change to
//! the gate's own inputs reruns everything, and a working tree with
//! uncommitted changes neither reuses nor records anything.
//! - **plan**: prints each step's action as `<step>\t<run|part|reuse>\t<members>\t<why>`
//!   lines after a `previous\t<head|none>` line, which `ci/pre-merge.sh`
//!   follows, says the same in words on stderr for the run's log, and
//!   writes the plan and the carried results into the run's record.
//! - **record**: appends a step's result to the run's record.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

mod plan;
mod runs;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result, bail};

use crate::affected::{self, Range, WorkspaceMembers};
use crate::environment::Environment;
use crate::output;
use plan::{Action, GateRun, Head, Outcome, Planned, Step, Touched, short, unit};

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
    },
    /// Appends a step's result to the record of the run on HEAD, for the
    /// units the plan ran; nothing without a plan for HEAD.
    Record {
        #[arg(long)]
        records: std::path::PathBuf,
        /// The step, by its name in the plan.
        #[arg(long)]
        step: String,
        #[arg(long, value_enum)]
        status: Status,
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
        } => {
            let records = environment.path(&records);
            plan_run(
                environment,
                &records,
                &Options {
                    full,
                    dry_run,
                    assume_green,
                },
            )?;
        }
        Command::Record {
            records,
            step,
            status,
        } => {
            let Some(step) = Step::from_name(&step) else {
                bail!("no gate step is named {step}");
            };
            record_run(root, &environment.path(&records), step, status)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

struct Options {
    full: bool,
    dry_run: bool,
    assume_green: Option<String>,
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
    let members_read = WorkspaceMembers::read(&environment.cargo, root)?;
    let members: BTreeSet<String> = members_read.all().map(|(name, _)| name).collect();
    let (previous, no_previous) = if options.full {
        (
            None,
            "a full run was asked for (GOODFETCH_GATE_FULL)".to_owned(),
        )
    } else if let Some(assumed) = &options.assume_green {
        let assumed = commit(root, assumed)?;
        let record = assumed_green(root, &assumed, &base, &members)?;
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
    let mut touched = BTreeMap::new();
    if let Some(previous) = &previous {
        let origins: BTreeSet<&String> = previous
            .units
            .values()
            .filter(|outcome| outcome.green)
            .map(|outcome| &outcome.origin)
            .collect();
        for origin in origins {
            touched.insert(
                origin.clone(),
                touched_since(root, &members_read, origin, &head),
            );
        }
    }
    let planned = plan::plan(
        &Head {
            base: &base,
            members: &members,
        },
        previous.as_ref(),
        &no_previous,
        &touched,
    );
    let previous_head = previous.as_ref().map(|record| record.head.as_str());
    let machine = render(previous_head, &members, &planned);
    output::error(&describe(previous_head, &planned));
    output::line(machine.trim_end());
    if options.dry_run {
        return Ok(());
    }
    std::fs::create_dir_all(records).with_context(|| format!("creating {}", records.display()))?;
    let carried = GateRun {
        head: head.clone(),
        base,
        units: planned
            .iter()
            .flat_map(|planned| planned.carried.clone())
            .collect(),
    };
    let record_file = runs::file(records, &head);
    std::fs::write(&record_file, runs::render(&carried))
        .with_context(|| format!("writing {}", record_file.display()))?;
    std::fs::write(&plan_file, machine)
        .with_context(|| format!("writing {}", plan_file.display()))?;
    Ok(())
}

/// What the change from `origin` to `head` reaches; a change git cannot
/// read reruns whatever ran on `origin`.
fn touched_since(root: &Path, members: &WorkspaceMembers, origin: &str, head: &str) -> Touched {
    let range = Range::Commits {
        base: origin.to_owned(),
        head: head.to_owned(),
    };
    match affected::changed_paths(root, &range) {
        Ok(paths) => Touched::new(&paths, &members.select(&paths)),
        Err(error) => Touched {
            gate_input: Some(format!("the history git cannot read ({error:#})")),
            ..Touched::default()
        },
    }
}

/// A full green run on `assumed`, for a dry run.
fn assumed_green(
    root: &Path,
    assumed: &str,
    base: &str,
    members: &BTreeSet<String>,
) -> Result<GateRun> {
    let tree = git(
        root,
        &["rev-parse", "--verify", &format!("{assumed}^{{tree}}")],
    )?;
    let outcome = Outcome {
        green: true,
        origin: assumed.to_owned(),
        tree,
    };
    let mut units = BTreeMap::new();
    for step in Step::ALL {
        if step.per_member() {
            for member in members {
                units.insert(unit(step, Some(member)), outcome.clone());
            }
        } else {
            units.insert(unit(step, None), outcome.clone());
        }
    }
    Ok(GateRun {
        head: assumed.to_owned(),
        base: base.to_owned(),
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
fn render(previous: Option<&str>, members: &BTreeSet<String>, planned: &[Planned]) -> String {
    let joined = |members: &BTreeSet<String>| {
        if members.is_empty() {
            "-".to_owned()
        } else {
            members.iter().cloned().collect::<Vec<_>>().join(" ")
        }
    };
    let mut text = format!("previous\t{}\n", previous.unwrap_or("none"));
    let _ = writeln!(text, "members\t{}", joined(members));
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

/// Appends `step`'s result to the record of the run on HEAD, for the units
/// its plan ran.
fn record_run(root: &Path, records: &Path, step: Step, status: Status) -> Result<()> {
    let head = commit(root, "HEAD")?;
    let plan_file = runs::plan_file(records, &head);
    let Ok(plan) = std::fs::read_to_string(&plan_file) else {
        output::error(&format!(
            "Gate: no plan for {head}, so {} is not recorded",
            step.name()
        ));
        return Ok(());
    };
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
    let units: Vec<String> = if step.per_member() {
        let members = match line.first().copied() {
            Some("run") => words(field("members").unwrap_or_default().first().unwrap_or(&"")),
            Some("part") => words(line.get(1).unwrap_or(&"")),
            _ => Vec::new(),
        };
        members
            .iter()
            .map(|member| unit(step, Some(member)))
            .collect()
    } else {
        vec![unit(step, None)]
    };
    let outcome = Outcome {
        green: status == Status::Pass,
        origin: head.clone(),
        tree: git(root, &["rev-parse", "--verify", "HEAD^{tree}"])?,
    };
    let text: String = units
        .iter()
        .map(|unit| runs::line(unit, &outcome))
        .collect();
    output::append(&runs::file(records, &head), &text)
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
