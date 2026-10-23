//! The checks of a gate run's verdict once it has ended (ADR 0135):
//! `verify`, which `just merge` runs before it merges, and `compare`, the
//! backstop's, which sets a full run against the incremental runs on the
//! same tree.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::{Command as Process, ExitCode};

use anyhow::Result;

use super::plan::{GateRun, Outcome, Step, short, unit};
use super::runs::{self, Key};
use super::{commit, evidence, git, head_members};
use crate::affected::WorkspaceMembers;
use crate::environment::Environment;
use crate::output;

/// Checks the verdict of the run on `head`, checked out: each unit of
/// every step has a green result, run on the head's own tree or reused
/// from an earlier run whose own signed record holds that green result, on
/// an ancestor with that ancestor's tree. Prints each such earlier run as
/// an `origin\t<commit>` line, or each problem on stderr and fails.
pub(super) fn verify(
    environment: &Environment,
    records: &Path,
    key: &Key,
    head: &str,
) -> Result<ExitCode> {
    let root = environment.root.as_path();
    let head = commit(root, head)?;
    let checked_out = commit(root, "HEAD")?;
    let mut problems = Vec::new();
    if checked_out != head {
        problems.push(format!(
            "HEAD is {checked_out}: the verdict of {head} is checked in its own checkout"
        ));
    }
    let record = match runs::read(records, &head, key) {
        Ok(record) => Some(record),
        Err(error) => {
            problems.push(format!("{error:#}"));
            None
        }
    };
    let mut origins = BTreeSet::new();
    if let Some(record) = record.filter(|_| problems.is_empty()) {
        let members_read = WorkspaceMembers::read(&environment.cargo, root)?;
        let unowned = evidence::unowned_tests(root, &members_read)?;
        let (members, _) = head_members(&members_read, &unowned);
        let tree = git(
            root,
            &["rev-parse", "--verify", &format!("{head}^{{tree}}")],
        )?;
        let mut earlier = Earlier::new(root, records, key);
        for step in Step::ALL {
            let units: Vec<Option<&str>> = if step.per_member() {
                members.iter().map(|member| Some(member.as_str())).collect()
            } else {
                vec![None]
            };
            for member in units {
                let label = match member {
                    Some(member) => format!(" for {member}"),
                    None => String::new(),
                };
                let name = step.name();
                match record.units.get(&unit(step, member)) {
                    None => problems.push(format!("{name} has no result{label}")),
                    Some(outcome) if !outcome.green => problems.push(format!(
                        "{name} is red on {}{label}",
                        short(&outcome.origin)
                    )),
                    Some(outcome) if outcome.origin == head => {
                        if outcome.tree != tree {
                            problems.push(format!(
                                "{name}{label} ran on the tree {}, not {head}'s",
                                outcome.tree
                            ));
                        }
                    }
                    Some(outcome) => match earlier.holds(&head, &unit(step, member), outcome) {
                        Ok(()) => {
                            origins.insert(outcome.origin.clone());
                        }
                        Err(why) => problems.push(format!(
                            "{name}{label} reuses the run on {}: {why}",
                            outcome.origin
                        )),
                    },
                }
            }
        }
    }
    if !problems.is_empty() {
        for problem in &problems {
            output::error(&format!("Gate verdict of {head}: {problem}"));
        }
        return Ok(ExitCode::FAILURE);
    }
    for origin in origins {
        output::line(&format!("origin\t{origin}"));
    }
    Ok(ExitCode::SUCCESS)
}

/// The earlier runs' records, each read once.
struct Earlier<'a> {
    root: &'a Path,
    records: &'a Path,
    key: &'a Key,
    read: BTreeMap<String, Result<GateRun, String>>,
}

impl<'a> Earlier<'a> {
    const fn new(root: &'a Path, records: &'a Path, key: &'a Key) -> Self {
        Self {
            root,
            records,
            key,
            read: BTreeMap::new(),
        }
    }

    /// Whether `outcome`, a result the run on `head` reused for `unit`,
    /// holds: its origin is an ancestor of `head` with the recorded tree,
    /// and the origin's own signed record holds the same green result.
    fn holds(&mut self, head: &str, unit: &str, outcome: &Outcome) -> Result<(), String> {
        let origin = &outcome.origin;
        if !is_ancestor(self.root, origin, head) {
            return Err(format!("{origin} is not an ancestor of {head}"));
        }
        let tree = git(
            self.root,
            &["rev-parse", "--verify", &format!("{origin}^{{tree}}")],
        )
        .map_err(|error| format!("{error:#}"))?;
        if tree != outcome.tree {
            return Err(format!("its tree is {tree}, not {}", outcome.tree));
        }
        let (records, key) = (self.records, self.key);
        let record = self.read.entry(origin.clone()).or_insert_with(|| {
            runs::read(records, origin, key).map_err(|error| format!("{error:#}"))
        });
        let record = record.as_ref().map_err(Clone::clone)?;
        if record.units.get(unit) != Some(outcome) {
            return Err(format!(
                "its record does not hold that green result of {unit}"
            ));
        }
        Ok(())
    }
}

fn is_ancestor(root: &Path, ancestor: &str, head: &str) -> bool {
    Process::new("git")
        .args([
            "merge-base",
            "--is-ancestor",
            "--end-of-options",
            ancestor,
            head,
        ])
        .current_dir(root)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// The backstop's comparison: for the full run on `full`, whose record is
/// in `records`, each result that another signed run in `others` on the same
/// tree reused from an earlier head and the full run found red, as `miss\t<unit>\t<run>\t<origin>` lines, or
/// `suspect` lines for a step run per member, whose red the members that
/// run reran may hold; `unchecked` lines for a reused result the full run
/// has no result for, as when it stopped at a red step.
pub(super) fn compare(
    root: &Path,
    records: &Path,
    others: &Path,
    key: &Key,
    full: &str,
) -> Result<ExitCode> {
    let full = commit(root, full)?;
    let record = runs::read(records, &full, key)?;
    if let Some((unit, outcome)) = record
        .units
        .iter()
        .find(|(_, outcome)| outcome.origin != full)
    {
        output::error(&format!(
            "Gate: the run on {full} is not a full run: it reused {unit} from {}",
            outcome.origin
        ));
        return Ok(ExitCode::FAILURE);
    }
    let tree = git(
        root,
        &["rev-parse", "--verify", &format!("{full}^{{tree}}")],
    )?;
    for head in runs::heads(others)? {
        if head == full {
            continue;
        }
        let same_tree = git(
            root,
            &["rev-parse", "--verify", &format!("{head}^{{tree}}")],
        )
        .is_ok_and(|other| other == tree);
        if !same_tree {
            continue;
        }
        let run = match runs::read(others, &head, key) {
            Ok(run) => run,
            Err(error) => {
                output::error(&format!(
                    "Gate: the run on {head} is not compared: {error:#}"
                ));
                continue;
            }
        };
        for (unit, outcome) in &run.units {
            if !outcome.green || outcome.origin == head {
                continue;
            }
            let kind = match record.units.get(unit) {
                None => "unchecked",
                Some(found) if found.green => continue,
                Some(_) => {
                    let step = unit.split_once(':').map_or(unit.as_str(), |(step, _)| step);
                    if Step::from_name(step).is_some_and(Step::per_member) {
                        "suspect"
                    } else {
                        "miss"
                    }
                }
            };
            output::line(&format!("{kind}\t{unit}\t{head}\t{}", outcome.origin));
        }
    }
    Ok(ExitCode::SUCCESS)
}
