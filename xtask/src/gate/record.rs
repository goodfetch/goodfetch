//! `xtask gate record` (ADR 0135): appends a step's result to the record of
//! the run on the planned head, for the units its plan ran, only on that
//! head with a clean tree. A failed step run per member records each crate
//! its JUnit files show passing in full green and its other members red
//! (`attribution`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::attribution::{Evidence, Judged, StressEvidence};
use super::plan::{Layer, Outcome, unit};
use super::steps::Step;
use super::{Status, plan_list, plan_value, runs};
use crate::affected::{self, WorkspaceMembers};
use crate::environment::Environment;
use crate::{git, output};

#[derive(clap::Args)]
pub(crate) struct Arguments {
    #[arg(long)]
    records: PathBuf,
    /// The head the plan was made for: HEAD must still be it.
    #[arg(long)]
    head: String,
    /// The layer that ran the step, which its result names.
    #[arg(long, value_enum)]
    by: Layer,
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
    junit: Vec<PathBuf>,
    /// The stress run's selection, its `<count>\t<runner>\t<test ID>`
    /// lines: a crate passed in full when each of its selected nextest
    /// tests passed its count of times in the `--junit` files.
    #[arg(long, requires_all = ["red_commands", "started"])]
    selection: Option<PathBuf>,
    /// The stress run's `<runner>\t<status>` lines, one a command that
    /// went red; with `--selection`. A red run the `--junit` files do not
    /// account for leaves no earned passes.
    #[arg(long, requires = "selection")]
    red_commands: Option<PathBuf>,
    /// When the stress run began, in seconds since the Unix epoch; with
    /// `--selection`. The passes it gives age from here, which is no later
    /// than the first of them.
    #[arg(long, requires = "selection")]
    started: Option<u64>,
}

pub(crate) fn run(arguments: Arguments, environment: &Environment) -> Result<()> {
    let root = environment.root.as_path();
    let Arguments {
        records,
        head,
        by,
        now,
        step,
        status,
        junit,
        selection,
        red_commands,
        started,
    } = arguments;
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
            by,
            now,
            step,
            status,
            evidence,
        },
    )
}

/// A step's result, for the head its plan was made for.
struct Recorded {
    head: String,
    by: Layer,
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
        by,
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
    let action = value(&format!("{}.action", step.name()))?;
    let members = if step.per_member() {
        match action {
            "run" => plan_list(value("members")?),
            "part" => plan_list(value(&format!("{}.members", step.name()))?),
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
        let kept = plan_list(value(&format!("{}.earned", step.name()))?);
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
            by: *by,
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
