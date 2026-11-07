//! The backstop's comparison (ADR 0135; the test-gate audit's option 7): a
//! full gate run set against each incremental run on the same tree, the
//! gated head of a merge, to find the results the incremental run reused
//! that the full run found red.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Serialize;

use super::plan::Step;
use super::{commit, runs};
use crate::git;
use crate::output;

/// One result another run reused that the full run did not find green.
#[derive(Serialize)]
struct Found {
    /// `miss` when the full run found it red; `suspect` for a step run per
    /// member, whose red the members that run reran may hold; `unchecked`
    /// when the full run has no result for it, as when it stopped at a red
    /// step.
    kind: &'static str,
    /// The unit, `<step>` or `<step>:<member>`.
    unit: String,
    /// Its step.
    step: String,
    /// Its member, for a step run per member.
    member: Option<String>,
    /// The head of the run that reused it.
    run: String,
    /// The head of the run it was reused from.
    origin: String,
}

/// For the full run on `full`, whose record is in `records`, each result
/// that another run in `others` on the same tree reused from an earlier head
/// and the full run did not find green, printed as one JSON array of
/// [`Found`] objects. Fails when the run on `full` reused anything, so it is
/// no full run.
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
    let mut found = Vec::new();
    let tree = git::line(
        root,
        &["rev-parse", "--verify", &format!("{full}^{{tree}}")],
    )?;
    for head in runs::heads(others)? {
        if head == full {
            continue;
        }
        let same_tree = git::line(
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
            let (step, member) = unit
                .split_once(':')
                .map_or((unit.as_str(), None), |(step, member)| (step, Some(member)));
            let kind = match record.units.get(unit) {
                None => "unchecked",
                Some(full) if full.green => continue,
                Some(_) if Step::from_name(step).is_some_and(Step::per_member) => "suspect",
                Some(_) => "miss",
            };
            found.push(Found {
                kind,
                unit: unit.clone(),
                step: step.to_owned(),
                member: member.map(str::to_owned),
                run: head.clone(),
                origin: outcome.origin.clone(),
            });
        }
    }
    output::line(&serde_json::to_string(&found).context("writing the comparison as JSON")?);
    Ok(ExitCode::SUCCESS)
}
