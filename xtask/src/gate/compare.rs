//! The backstop's comparison (ADR 0135; the test-gate audit's option 7): a
//! full gate run set against each incremental run on the same tree, the
//! gated head of a merge, to find the results the incremental run reused
//! that the full run found red; and against each run merged since the last
//! green backstop run, on any tree, for the results that counted earned passes.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Serialize;

use super::plan::Step;
use super::runs;
use crate::git;
use crate::output;

/// One result another run reused that the full run did not find green.
#[derive(Serialize)]
struct Found {
    /// `miss` when the full run found it red; `suspect` for a step run per
    /// member, whose red the members that run reran may hold; `unchecked`
    /// when the full run has no result for it, as when it stopped at a red
    /// step; `earned`, whatever the full run found, for a result that counted
    /// passes from an earlier head: a full run on `main`'s head has that head
    /// as its base, so its stress run stresses nothing, and only a stress run
    /// against this run's base can say; `unreadable` for a run merged since
    /// the last green backstop run whose record cannot be read, which may
    /// have counted such passes, with only its `run` set.
    kind: &'static str,
    /// The unit, `<step>` or `<step>:<member>`.
    unit: String,
    /// Its step.
    step: String,
    /// Its member, for a step run per member.
    member: Option<String>,
    /// The head of the run that reused it.
    run: String,
    /// The head of the run it was reused from, or where its earliest earned
    /// passes ran.
    origin: String,
    /// The base the run that reused it compared with.
    base: String,
}

/// For the full run on `full`, whose record is in `records`, each result
/// that another run in `others` on the same tree reused from an earlier head
/// and the full run did not find green; and each result that counted earned
/// passes of a run `full` holds and `since`, the backstop's last green run,
/// does not, whatever its tree, so a day checks every merge that day and those
/// before it left unchecked. Printed as one JSON array of [`Found`] objects.
/// Fails when the run on `full` reused anything, so it is no full run.
pub(super) fn compare(
    root: &Path,
    records: &Path,
    others: &Path,
    full: &str,
    since: Option<&str>,
) -> Result<ExitCode> {
    let full = crate::git::commit(root, full)?;
    let record = runs::read(records, &full)?;
    if let Some((unit, outcome)) = record
        .units
        .iter()
        .find(|(_, outcome)| outcome.origin != full || outcome.earned_on.is_some())
    {
        output::error(&format!(
            "Gate: the run on {full} is not a full run: it reused {unit} from {}",
            outcome.earned_on.as_ref().unwrap_or(&outcome.origin)
        ));
        return Ok(ExitCode::FAILURE);
    }
    let mut found = Vec::new();
    let tree = git::line(
        root,
        &["rev-parse", "--verify", &format!("{full}^{{tree}}")],
    )?;
    // `just merge` merges with a merge commit, so `full` holds each gated head
    // that merged, on whichever tree.
    let mut range = vec!["rev-list".to_owned(), full.clone()];
    range.extend(since.map(|since| format!("^{since}")));
    let range: Vec<&str> = range.iter().map(String::as_str).collect();
    let unchecked: BTreeSet<String> = git::text(root, &range)?
        .lines()
        .map(str::to_owned)
        .collect();
    for head in runs::heads(others)? {
        if head == full {
            continue;
        }
        let merged = unchecked.contains(&head);
        let same_tree = git::line(
            root,
            &["rev-parse", "--verify", &format!("{head}^{{tree}}")],
        )
        .is_ok_and(|other| other == tree);
        if !same_tree && !merged {
            continue;
        }
        let run = match runs::read(others, &head) {
            Ok(run) => run,
            Err(error) => {
                output::error(&format!(
                    "Gate: the run on {head} is not compared: {error:#}"
                ));
                if merged {
                    found.push(Found {
                        kind: "unreadable",
                        unit: String::new(),
                        step: String::new(),
                        member: None,
                        run: head.clone(),
                        origin: String::new(),
                        base: String::new(),
                    });
                }
                continue;
            }
        };
        for (unit, outcome) in &run.units {
            // A result carried from a run the last green run holds was stressed
            // again on that day, and what carried it does not reach its member.
            // One carried from a head nothing merged, a pull request pushed again
            // over it, was not, and a head git no longer knows may not have been.
            let held_by_last_green = since.is_some_and(|since| {
                git::text(
                    root,
                    &["merge-base", "--is-ancestor", &outcome.origin, since],
                )
                .is_ok()
            });
            let earned = merged && outcome.earned_on.is_some() && !held_by_last_green;
            if !outcome.green || (!earned && (!same_tree || outcome.origin == head)) {
                continue;
            }
            let (step, member) = unit
                .split_once(':')
                .map_or((unit.as_str(), None), |(step, member)| (step, Some(member)));
            let kind = match record.units.get(unit) {
                _ if earned => "earned",
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
                origin: outcome
                    .earned_on
                    .as_ref()
                    .unwrap_or(&outcome.origin)
                    .clone(),
                base: run.base.clone(),
            });
        }
    }
    output::line(&serde_json::to_string(&found).context("writing the comparison as JSON")?);
    Ok(ExitCode::SUCCESS)
}
