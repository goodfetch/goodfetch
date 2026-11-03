//! The same-work check, which gates every figure: before any counts, every
//! trial of every measured cell of a scenario fetched the site's own pages
//! and was served the requests and bytes most of that scenario's trials
//! were, as the Proving ground's own observations record them, never a
//! tool's report (spec #152). No trial is the others' reference, so the
//! trial that did other work is the one named, whichever ran first (ADR
//! 0134's #1574 note). A scenario that kills its crawl halfway and resumes
//! it fetches some pages again, so its trials are held to the bytes of each
//! page's first answer instead. On the headless browser scenario's site,
//! whose links past the first page only a script adds, a trial that missed
//! pages is named for the script-rendered pages it never reached. Each
//! counted cell and the growth cell are held to it among themselves, as
//! other cells of the comparison (ADR 0140).

use std::collections::BTreeMap;

use crate::check::Finding;

use super::matrix::{self, Scenario};
use super::trials::{
    Comparison, CountedCell, CountedRun, GrowthCell, INSTRUCTION_TOOLS, Tool, Trial, Work,
};

/// What one trial of a pass did, whichever pass.
struct Done<'a> {
    scenario: Scenario,
    cell: &'a str,
    number: u32,
    work: &'a Work,
}

/// One pass of trials: the timed one, a counted cell or the growth cell.
struct Pass<'a> {
    /// ` callgrind`, ` strace`, ` growth` or nothing, as a finding names the
    /// cell.
    suffix: &'static str,
    declared: u32,
    site_pages: u64,
    /// The scenarios whose measured cells the pass runs, and those cells'
    /// tools.
    scenarios: &'a [Scenario],
    tools: &'a [Tool],
    done: Vec<Done<'a>>,
}

impl Pass<'_> {
    fn cell(&self, scenario: Scenario, cell: &str) -> String {
        format!("{} {cell}{}", scenario.name(), self.suffix)
    }

    fn subject(&self, done: &Done<'_>) -> String {
        format!(
            "{} trial {}",
            self.cell(done.scenario, done.cell),
            done.number
        )
    }
}

fn of_trials(trials: &[Trial]) -> Vec<Done<'_>> {
    trials
        .iter()
        .map(|trial| Done {
            scenario: trial.scenario,
            cell: &trial.cell,
            number: trial.number,
            work: &trial.measured.work,
        })
        .collect()
}

fn of_runs(runs: &[CountedRun]) -> Vec<Done<'_>> {
    runs.iter()
        .map(|run| Done {
            scenario: run.scenario,
            cell: &run.cell,
            number: run.number,
            work: &run.work,
        })
        .collect()
}

/// The pages of the site `/sites/<n>/1` declares.
fn site_pages(first: &str) -> Option<u64> {
    first
        .strip_prefix("/sites/")?
        .strip_suffix("/1")?
        .parse()
        .ok()
}

/// Each finding that keeps the figures from counting: see [`check_pass`].
/// A counted run that counted nothing measured nothing.
pub(crate) fn check(trials: &Comparison) -> Vec<Finding> {
    let Some(pages) = site_pages(&trials.declared.site) else {
        return vec![Finding::new(
            "unknown_site",
            trials.declared.site.clone(),
            "the declared site's first page is no `/sites/<n>/1`",
        )];
    };
    let declared = trials.declared.trials;
    let mut findings = check_pass(&Pass {
        suffix: "",
        declared,
        site_pages: pages,
        scenarios: &Scenario::ALL,
        tools: &Tool::ALL,
        done: of_trials(&trials.trials),
    });
    for (suffix, cell, scenarios, tools, empty) in [
        (
            " strace",
            &trials.syscalls,
            &Scenario::ALL[..],
            &Tool::ALL[..],
            "strace counted no system call, so the run measured nothing",
        ),
        (
            " callgrind",
            &trials.instructions,
            &[Scenario::Raw][..],
            &INSTRUCTION_TOOLS[..],
            "Callgrind counted no instruction, so the run measured nothing",
        ),
    ] {
        if let CountedCell::Measured {
            trials: declared,
            runs,
        } = cell
        {
            let pass = Pass {
                suffix,
                declared: *declared,
                site_pages: pages,
                scenarios,
                tools,
                done: of_runs(runs),
            };
            findings.extend(check_pass(&pass));
            for (run, done) in runs.iter().zip(&pass.done) {
                if run.count == 0 {
                    findings.push(Finding::new("nothing_counted", pass.subject(done), empty));
                }
            }
        }
    }
    if let GrowthCell::Measured {
        trials: declared,
        site_pages,
        trials_run,
    } = &trials.growth
    {
        findings.extend(check_pass(&Pass {
            suffix: " growth",
            declared: *declared,
            site_pages: *site_pages,
            scenarios: &Scenario::ALL,
            tools: &Tool::ALL,
            done: of_trials(trials_run),
        }));
    }
    findings
}

/// Each finding of one pass: a measured cell short of or past its declared
/// trials, every trial whose pages are not the site's, and every other
/// trial whose work is not the one most of its scenario's trials share.
fn check_pass(pass: &Pass<'_>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (cell, _) in matrix::measured().filter(|(cell, config)| {
        pass.scenarios.contains(&cell.scenario) && pass.tools.contains(&config.tool())
    }) {
        let count = pass
            .done
            .iter()
            .filter(|done| done.scenario == cell.scenario && done.cell == cell.name)
            .count();
        if u32::try_from(count).ok() != Some(pass.declared) {
            findings.push(Finding::new(
                "trial_count",
                pass.cell(cell.scenario, cell.name),
                format!("{count} trials, where {} were declared", pass.declared),
            ));
        }
    }
    for scenario in pass.scenarios {
        let mut whole = Vec::new();
        for done in pass.done.iter().filter(|done| done.scenario == *scenario) {
            let work = done.work;
            if work.pages == pass.site_pages && work.missing.is_empty() && work.extra.is_empty() {
                whole.push(done);
                continue;
            }
            // On the script-rendered site, every page past the first is
            // linked only by a script: a trial that missed some never
            // rendered their links.
            if scenario.renders_links() && work.extra.is_empty() && work.pages > 0 {
                findings.push(Finding::new(
                    "unrendered_pages",
                    pass.subject(done),
                    format!(
                        "{} of the site's {} pages its script links not reached",
                        work.missing.len(),
                        pass.site_pages.saturating_sub(1)
                    ),
                ));
                continue;
            }
            findings.push(Finding::new(
                "different_pages",
                pass.subject(done),
                format!(
                    "{} of the site's {} pages missing and {} other pages fetched",
                    work.missing.len(),
                    pass.site_pages,
                    work.extra.len()
                ),
            ));
        }
        let key = |work: &Work| {
            if scenario.resumes() {
                (work.pages, work.first_bytes)
            } else {
                (work.requests, work.bytes)
            }
        };
        let what = if scenario.resumes() {
            "pages first served"
        } else {
            "requests served"
        };
        // The work most of the trials that fetched the site share; none
        // when two values are shared by as many trials.
        let mut shared: BTreeMap<(u64, u64), usize> = BTreeMap::new();
        for done in &whole {
            *shared.entry(key(done.work)).or_default() += 1;
        }
        let most = shared.values().copied().max().unwrap_or(0);
        let mut held = shared.iter().filter(|(_, count)| **count == most);
        let reference = match (held.next(), held.next()) {
            (Some((work, count)), None) => Some((*work, *count)),
            _ => None,
        };
        for done in whole {
            let (count, bytes) = key(done.work);
            let message = match reference {
                Some((work, _)) if work == (count, bytes) => continue,
                Some(((held_count, held_bytes), trials)) => format!(
                    "{count} {what} {bytes} bytes, where {trials} of the {} trials that fetched the site had {held_count} {what} {held_bytes}",
                    shared.values().sum::<usize>()
                ),
                None => format!(
                    "{count} {what} {bytes} bytes, and no count and bytes is shared by more trials than another"
                ),
            };
            findings.push(Finding::new("different_bytes", pass.subject(done), message));
        }
    }
    findings
}

#[cfg(test)]
mod tests;
