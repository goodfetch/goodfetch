//! The backstop's comparison (ADR 0135; the test-gate audit's option 7): a
//! full gate run set against each incremental run on the same tree, the
//! gated head of a merge, to find the results the incremental run reused
//! that the full run found red.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;

use super::plan::Step;
use super::{commit, git, runs};
use crate::output;

/// For the full run on `full`, whose record is in `records`, each result
/// that another run in `others` on the same tree reused from an earlier head
/// and the full run found red, as `miss\t<unit>\t<run>\t<origin>` lines, or
/// `suspect` lines for a step run per member, whose red the members that
/// run reran may hold; `unchecked` lines for a reused result the full run
/// has no result for, as when it stopped at a red step. Fails when the run
/// on `full` reused anything, so it is no full run.
pub(super) fn compare(root: &Path, records: &Path, others: &Path, full: &str) -> Result<ExitCode> {
    let full = commit(root, full)?;
    let record = runs::read(records, &full)?;
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
        let run = match runs::read(others, &head) {
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
