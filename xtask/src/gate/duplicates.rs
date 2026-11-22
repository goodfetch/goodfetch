//! The duplicate-run check (ADR 0135's amendment of 2026-11-22): nothing runs
//! twice on one tree. It reads every run in the gate's records and finds each
//! unit a run ran, not reused, on a tree where an earlier run on the same host
//! had recorded it green within a day: the later run had that result to reuse,
//! whichever layer ran each.
//!
//! A rerun after a red result is none, nor one past a day or on another host,
//! which no plan reuses across. Nor is a rerun of a check whose verdict reads
//! more than the tree: the commit checks, whose commits differ from head to
//! head, and the audit, whose advisories change without the tree. A plan on a
//! head replaces that head's record, so a rerun on the same head is out of the
//! check's sight; every other run on a tree is a head of its own.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};

use super::plan::{GateRun, Outcome, Step, past_max_age, short};
use super::runs;
use crate::check::{self, Finding};

/// Reads every record in `records` and reports each unit run twice on one
/// tree; a record that cannot be read is an error, never a pass.
pub(crate) fn check(records: &Path) -> Result<ExitCode> {
    let mut read = Vec::new();
    for head in runs::heads(records)? {
        read.push(
            runs::read(records, &head)
                .with_context(|| format!("reading the gate's record of {}", short(&head)))?,
        );
    }
    Ok(check::report(&duplicates(&read)))
}

/// One unit's result that the run on `head` ran itself.
struct Ran<'a> {
    head: &'a str,
    outcome: &'a Outcome,
}

/// Each unit of `records` that a run ran on a tree where an earlier run on the
/// same host had recorded it green within a day.
pub(super) fn duplicates(records: &[GateRun]) -> Vec<Finding> {
    let mut ran: BTreeMap<(&str, &str, &str), Vec<Ran<'_>>> = BTreeMap::new();
    for record in records {
        for (unit, outcome) in &record.units {
            if outcome.origin == record.head && judged_by_the_tree(unit) {
                ran.entry((unit.as_str(), outcome.tree.as_str(), record.host.as_str()))
                    .or_default()
                    .push(Ran {
                        head: &record.head,
                        outcome,
                    });
            }
        }
    }
    let mut findings = Vec::new();
    for ((unit, tree, _), mut runs) in ran {
        runs.sort_by_key(|run| run.outcome.at);
        let mut reusable: Option<&Ran<'_>> = None;
        for run in &runs {
            if let Some(green) = reusable
                && !past_max_age(run.outcome.at, green.outcome.at)
            {
                findings.push(Finding::new(
                    "duplicate-run",
                    unit,
                    format!(
                        "ran on tree {} by {} on {}, after {} on {} had recorded it green there",
                        short(tree),
                        run.outcome.by.name(),
                        short(run.head),
                        green.outcome.by.name(),
                        short(green.head),
                    ),
                ));
            }
            reusable = run.outcome.green.then_some(run);
        }
    }
    findings
}

/// Whether a rerun of `unit` on its tree is a duplicate: all but the commit
/// checks' and the audit's, and a unit of a step the gate no longer knows.
fn judged_by_the_tree(unit: &str) -> bool {
    let step = unit.split_once(':').map_or(unit, |(step, _)| step);
    Step::from_name(step).is_none_or(|step| !step.reads_the_commits() && step != Step::Audit)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::duplicates;
    use crate::gate::plan::{GateRun, Layer, MAX_AGE, Outcome};

    const TREE: &str = "tree";
    const AT: u64 = 1_800_000_000;
    const NONE: [(String, String); 0] = [];

    /// The run on `head` on `host`, at `at`, which ran each of `ran` with its
    /// status and reused each of `reused` green from the run on its origin.
    fn run(
        head: &str,
        host: &str,
        at: u64,
        by: Layer,
        ran: &[(&str, bool)],
        reused: &[(&str, &str)],
    ) -> GateRun {
        let outcome = |origin: &str, green| Outcome {
            green,
            origin: origin.to_owned(),
            tree: TREE.to_owned(),
            at,
            by,
            earned_on: None,
        };
        let mut units: BTreeMap<String, Outcome> = ran
            .iter()
            .map(|(unit, green)| ((*unit).to_owned(), outcome(head, *green)))
            .collect();
        units.extend(
            reused
                .iter()
                .map(|(unit, origin)| ((*unit).to_owned(), outcome(origin, true))),
        );
        GateRun {
            head: head.to_owned(),
            base: "base".to_owned(),
            host: host.to_owned(),
            units,
            earned: BTreeMap::new(),
        }
    }

    fn found(records: &[GateRun]) -> Vec<(String, String)> {
        duplicates(records)
            .into_iter()
            .map(|finding| (finding.subject, finding.message))
            .collect()
    }

    #[test]
    fn a_unit_run_twice_on_one_tree_is_found_with_both_layers() {
        let records = [
            run(
                "first",
                "host",
                AT,
                Layer::Preflight,
                &[("pre-merge:xtask", true)],
                &[],
            ),
            run(
                "second",
                "host",
                AT + 60,
                Layer::Merge,
                &[("pre-merge:xtask", true)],
                &[],
            ),
        ];

        assert_eq!(
            found(&records),
            [(
                "pre-merge:xtask".to_owned(),
                "ran on tree tree by merge on second, after preflight on first had recorded it green there"
                    .to_owned()
            )]
        );
    }

    #[test]
    fn a_unit_reused_on_its_tree_is_no_duplicate() {
        let records = [
            run("first", "host", AT, Layer::Merge, &[("lint", true)], &[]),
            run(
                "second",
                "host",
                AT + 60,
                Layer::Merge,
                &[],
                &[("lint", "first")],
            ),
        ];

        assert_eq!(found(&records), NONE);
    }

    /// A rerun after a red result, past a day or on another host is none: no
    /// plan had a result there to reuse.
    #[test]
    fn a_rerun_no_plan_could_have_spared_is_no_duplicate() {
        let after_red = [
            run("first", "host", AT, Layer::Merge, &[("lint", false)], &[]),
            run(
                "second",
                "host",
                AT + 60,
                Layer::Merge,
                &[("lint", true)],
                &[],
            ),
        ];
        let past_a_day = [
            run("first", "host", AT, Layer::Merge, &[("lint", true)], &[]),
            run(
                "second",
                "host",
                AT + MAX_AGE + 1,
                Layer::Merge,
                &[("lint", true)],
                &[],
            ),
        ];
        let another_host = [
            run("first", "host", AT, Layer::Merge, &[("lint", true)], &[]),
            run(
                "second",
                "other",
                AT + 60,
                Layer::Merge,
                &[("lint", true)],
                &[],
            ),
        ];

        for records in [after_red, past_a_day, another_host] {
            assert_eq!(found(&records), NONE);
        }
    }

    /// The commit checks judge each head's commits, and the audit the
    /// advisories as they are: neither verdict follows from the tree.
    #[test]
    fn a_check_that_reads_more_than_the_tree_reruns_on_it_freely() {
        let checks = [("dco", true), ("commit-messages", true), ("audit", true)];
        let records = [
            run("first", "host", AT, Layer::Merge, &checks, &[]),
            run("second", "host", AT + 60, Layer::Merge, &checks, &[]),
        ];

        assert_eq!(found(&records), NONE);
    }
}
