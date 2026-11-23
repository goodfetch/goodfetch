//! The duplicate-run check (ADR 0135's amendment of 2026-11-22): nothing runs
//! twice on one tree. It reads every run in the gate's records, and the hosted
//! runs' records `just merge` downloaded when given them, and finds each unit
//! a run ran, not reused, on a tree where an earlier run its plan reads had
//! recorded it green within a day: the later run had that result to reuse,
//! whichever layer ran each. A local plan reads its own host's runs, and the
//! hosted runs' for a step the hosted tier owns; a hosted job reads every
//! hosted run's (spec #3373).
//!
//! A rerun after a red result is none, nor one past a day or on a host no
//! plan reuses across. Nor is a rerun of a check whose verdict reads
//! more than the tree: the commit checks, whose commits differ from head to
//! head, and the audit, whose advisories change without the tree. A plan on a
//! head replaces that head's record, so a rerun on the same head is out of the
//! check's sight; every other run on a tree is a head of its own. Each run of
//! `just quick` the implementer's layer or the pre-push hook's recorded
//! (`quick_runs`) counts as a run of its own, so it sees one on the same head.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};

use super::plan::{GateRun, Layer, Outcome, past_max_age, short};
use super::steps::{Owner, Step};
use super::{earlier, quick_runs, runs};
use crate::check::{self, Finding};

/// Reads every record in `records`, and the hosted runs' in `hosted` that
/// `ci/hosted-records.sh` downloaded, and reports each unit run twice on one
/// tree; a record that cannot be read is an error, never a pass.
pub(crate) fn check(records: &Path, hosted: Option<&Path>) -> Result<ExitCode> {
    let mut read = match hosted {
        Some(folder) => earlier::every_hosted(folder)
            .with_context(|| format!("reading the hosted records in {}", folder.display()))?,
        None => Vec::new(),
    };
    for head in runs::heads(records)? {
        read.push(
            runs::read(records, &head)
                .with_context(|| format!("reading the gate's record of {}", short(&head)))?,
        );
    }
    read.extend(quick_runs::as_gate_runs(quick_runs::read(records)?));
    Ok(check::report(&duplicates(&read)))
}

/// One unit's result that the run on `head` ran itself.
struct Ran<'a> {
    head: &'a str,
    host: &'a str,
    outcome: &'a Outcome,
}

impl Ran<'_> {
    /// Whether the plan of this run of `unit` read `earlier`'s result: a
    /// hosted job reads every hosted run's, whatever runner image ran it
    /// (the tree pins the CI image); `just merge` reads them too for a step
    /// the hosted tier owns; a local run reads its own host's.
    fn reads(&self, unit: &str, earlier: &Ran<'_>) -> bool {
        match (earlier.outcome.by, self.outcome.by) {
            (Layer::Hosted, Layer::Hosted) => true,
            (Layer::Hosted, _) => hosted_owned(unit),
            (_, Layer::Hosted) => false,
            _ => earlier.host == self.host,
        }
    }
}

/// Each unit of `records` that a run ran on a tree where an earlier run its
/// plan reads had recorded it green within a day.
pub(super) fn duplicates(records: &[GateRun]) -> Vec<Finding> {
    let mut ran: BTreeMap<(&str, &str), Vec<Ran<'_>>> = BTreeMap::new();
    for record in records {
        for (unit, outcome) in &record.units {
            if outcome.origin != record.head || !judged_by_the_tree(unit) {
                continue;
            }
            let runs = ran
                .entry((unit.as_str(), outcome.tree.as_str()))
                .or_default();
            // A hosted job's record carries the results of the run's other
            // jobs on the same head unchanged: the same result, not a rerun.
            if !runs
                .iter()
                .any(|seen| seen.head == record.head && seen.outcome == outcome)
            {
                runs.push(Ran {
                    head: &record.head,
                    host: &record.host,
                    outcome,
                });
            }
        }
    }
    let mut findings = Vec::new();
    for ((unit, tree), mut runs) in ran {
        runs.sort_by_key(|run| run.outcome.at);
        for (index, run) in runs.iter().enumerate() {
            let read = runs[..index]
                .iter()
                .rev()
                .find(|earlier| run.reads(unit, earlier));
            if let Some(green) = read.filter(|earlier| earlier.outcome.green)
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
        }
    }
    findings
}

/// The step `unit` is of, when the gate knows it.
fn step_of(unit: &str) -> Option<Step> {
    Step::from_name(unit.split_once(':').map_or(unit, |(step, _)| step))
}

/// Whether a rerun of `unit` on its tree is a duplicate: all but the commit
/// checks' and the audit's, and a unit of a step the gate no longer knows.
fn judged_by_the_tree(unit: &str) -> bool {
    step_of(unit).is_none_or(|step| !step.reads_the_commits() && step != Step::Audit)
}

/// Whether the hosted tier owns `unit`'s step, so `just merge` reads its
/// hosted results.
fn hosted_owned(unit: &str) -> bool {
    step_of(unit).is_some_and(|step| Owner::of(step) == Owner::Hosted)
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

    /// `just merge` reads the hosted runs' records for the steps the hosted
    /// tier owns, whatever runner image ran them, and a hosted job reads
    /// every hosted run's: a later run of such a unit on the tree is a
    /// duplicate across hosts. No hosted job reads a local record.
    #[test]
    fn a_hosted_owned_unit_run_after_a_hosted_run_on_its_tree_is_found_across_hosts() {
        let hosted = run(
            "first",
            "image/runner-1",
            AT,
            Layer::Hosted,
            &[("lint", true)],
            &[],
        );
        let by_the_gate = [
            hosted.clone(),
            run(
                "second",
                "host",
                AT + 60,
                Layer::Merge,
                &[("lint", true)],
                &[],
            ),
        ];
        let by_a_later_runner = [
            hosted,
            run(
                "second",
                "image/runner-2",
                AT + 60,
                Layer::Hosted,
                &[("lint", true)],
                &[],
            ),
        ];

        for (records, by) in [(by_the_gate, "merge"), (by_a_later_runner, "hosted")] {
            assert_eq!(
                found(&records),
                [(
                    "lint".to_owned(),
                    format!(
                        "ran on tree tree by {by} on second, after hosted on first had recorded it green there"
                    )
                )]
            );
        }
    }

    /// The gate host's steps are no hosted tier's, and a hosted job reads no
    /// local record: neither rerun is one a plan could have spared.
    #[test]
    fn a_rerun_across_the_hosted_and_local_hosts_no_plan_reads_is_no_duplicate() {
        let perf_after_hosted = [
            run(
                "first",
                "image/runner-1",
                AT,
                Layer::Hosted,
                &[("perf", true)],
                &[],
            ),
            run(
                "second",
                "host",
                AT + 60,
                Layer::Merge,
                &[("perf", true)],
                &[],
            ),
        ];
        let hosted_after_local = [
            run("first", "host", AT, Layer::Merge, &[("lint", true)], &[]),
            run(
                "second",
                "image/runner-1",
                AT + 60,
                Layer::Hosted,
                &[("lint", true)],
                &[],
            ),
        ];

        for records in [perf_after_hosted, hosted_after_local] {
            assert_eq!(found(&records), NONE);
        }
    }

    /// Each job of a hosted run records the results its plan carried from the
    /// run's other jobs on the same head, unchanged: one result, run once.
    #[test]
    fn a_result_a_hosted_job_carried_from_another_on_its_head_is_no_duplicate() {
        let lint_job = run(
            "head",
            "image/runner-1",
            AT,
            Layer::Hosted,
            &[("lint", true)],
            &[],
        );
        let mut miri_job = run(
            "head",
            "image/runner-2",
            AT + 60,
            Layer::Hosted,
            &[("miri", true)],
            &[],
        );
        miri_job
            .units
            .insert("lint".to_owned(), lint_job.units["lint"].clone());

        assert_eq!(found(&[lint_job, miri_job]), NONE);
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
