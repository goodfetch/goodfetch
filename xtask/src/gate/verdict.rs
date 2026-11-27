//! The check of a gate run's verdict once it has ended (ADR 0135): `verify`,
//! which `just merge` runs before it merges a pull request. Every required unit
//! of the gate form must hold a signed green result, either run on the head's
//! own tree or reused from an earlier run whose own signed record holds that
//! exact green result on an ancestor with that ancestor's tree.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;

use super::plan::{GateRun, Outcome, short, unit};
use super::runs::{self, Key};
use super::steps::{self, Form, Step};
use super::{evidence, head_members};
use crate::affected::WorkspaceMembers;
use crate::environment::Environment;
use crate::{git, output};

/// Checks the verdict of the run on `head`, checked out: each required unit of
/// `form` has a signed green result, run on `head`'s own tree or reused from
/// an earlier run whose own signed record holds that green result, on an
/// ancestor with that ancestor's tree. Prints each such earlier run as an
/// `origin\t<commit>` line, or each problem on stderr and fails.
pub(super) fn verify(
    environment: &Environment,
    records: &Path,
    key: &Key,
    head: &str,
    form: Form,
) -> Result<ExitCode> {
    let root = environment.root.as_path();
    let head = git::commit(root, head)?;
    let checked_out = git::commit(root, "HEAD")?;
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
        let tree = git::line(
            root,
            &["rev-parse", "--verify", &format!("{head}^{{tree}}")],
        )?;
        let mut earlier = Earlier::new(root, records, key);
        for step in Step::ALL.into_iter().filter(|step| {
            steps::runs_in(*step, form)
                && step
                    .covered_by()
                    .is_none_or(|covering| !steps::runs_in(covering, form))
        }) {
            let units: Vec<Option<&str>> = if step.per_member() {
                members.iter().map(String::as_str).map(Some).collect()
            } else {
                vec![None]
            };
            for member in units {
                let label = member.map_or_else(String::new, |member| format!(" for {member}"));
                let name = step.name();
                let key_name = unit(step, member);
                match record.units.get(&key_name) {
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
                    Some(outcome) => match earlier.holds(&head, &key_name, outcome) {
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

/// The earlier runs' records, each read and signature-verified once.
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
        let ancestor = git::succeeds(
            self.root,
            &[
                "merge-base",
                "--is-ancestor",
                "--end-of-options",
                origin,
                head,
            ],
        )
        .unwrap_or(false);
        if !ancestor {
            return Err(format!("{origin} is not an ancestor of {head}"));
        }
        let tree = git::line(
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
