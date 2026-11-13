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
//! other cells of the comparison (ADR 0140). The long scenario's site is of
//! its own length, and its cells run only their timed trials. A strace run
//! that did other work fails nothing: strace's slowdown alone can keep a
//! headless browser crawl from its last pages, so that cell's system calls
//! are not measured, with the pages the run reached, and the timed figures
//! count ([`strace_runs_of_other_work_set_aside`]).
//!
//! On the article site, the same work is also the same output (#2136):
//! each trial's pages were held to the site's right answers as it ended
//! ([`super::extracted`]). A trial of Foxhound's whose pages differ under
//! its cell's check is refused here, in every pass; a competitor's keeps
//! its figures, which the report labels; and a trial no check read is no
//! figure, while the Proving ground serves the site's answers.

use std::collections::BTreeMap;

use crate::check::Finding;

use super::extracted::{ANSWERED_PAGES, Extracted};
use super::matrix::{self, OutputCheck, Row, Scenario};
use super::trials::{
    Comparison, CountedCell, CountedRun, GrowthCell, INSTRUCTION_SCENARIOS, INSTRUCTION_TOOLS,
    StoppedRun, Tool, Trial, Work,
};

/// What one trial of a pass did, whichever pass.
struct Done<'a> {
    scenario: Scenario,
    cell: &'a str,
    number: u32,
    work: &'a Work,
    /// What the output check found of the pages it wrote, where it read
    /// them.
    extracted: Option<&'a Extracted>,
}

/// One pass of trials: the timed one, a counted cell or the growth cell.
struct Pass<'a> {
    /// ` callgrind`, ` strace`, ` growth` or nothing, as a finding names the
    /// cell.
    suffix: &'static str,
    declared: u32,
    site_pages: u64,
    /// The scenarios whose measured cells the pass runs, those cells'
    /// tools, and whether it runs their sensitivity rows beside their main
    /// ones.
    scenarios: &'a [Scenario],
    tools: &'a [Tool],
    rows: &'a [Row],
    done: Vec<Done<'a>>,
    /// The cells whose run the comparison stopped: not measured, so not
    /// short of their declared runs.
    stopped: Vec<(Scenario, &'a str)>,
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
            extracted: trial.measured.extracted.as_ref(),
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
            extracted: run.extracted.as_ref(),
        })
        .collect()
}

/// Whether a trial is a main row's.
fn is_main(done: &Done<'_>) -> bool {
    matrix::measured().any(|(cell, _)| {
        cell.scenario == done.scenario && cell.name == done.cell && cell.row == Row::Main
    })
}

/// Main and sensitivity rows alike.
const EVERY_ROW: &[Row] = &[Row::Main, Row::Sensitivity];

/// The cells of stopped runs or trials, not measured.
fn of_stopped(stopped: &[StoppedRun]) -> Vec<(Scenario, &str)> {
    stopped
        .iter()
        .map(|run| (run.scenario, run.cell.as_str()))
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
    let ran = &trials.declared.scenarios[..];
    let counted: Vec<Scenario> = ran
        .iter()
        .copied()
        .filter(|scenario| !scenario.timed_only())
        .collect();
    let instructions: Vec<Scenario> = INSTRUCTION_SCENARIOS
        .into_iter()
        .filter(|scenario| ran.contains(scenario))
        .collect();
    let mut findings = check_pass(&Pass {
        suffix: "",
        declared,
        site_pages: pages,
        scenarios: ran,
        tools: &Tool::ALL,
        rows: EVERY_ROW,
        done: of_trials(&trials.trials),
        stopped: of_stopped(&trials.stopped),
    });
    for (suffix, cell, scenarios, tools, rows, empty) in [
        (
            " strace",
            &trials.syscalls,
            &counted[..],
            &Tool::ALL[..],
            EVERY_ROW,
            "strace counted no system call, so the run measured nothing",
        ),
        (
            " callgrind",
            &trials.instructions,
            &instructions[..],
            &INSTRUCTION_TOOLS[..],
            // The ratios read each tool's main rows.
            &[Row::Main][..],
            "Callgrind counted no instruction, so the run measured nothing",
        ),
    ] {
        if let CountedCell::Measured {
            trials: declared,
            runs,
            stopped,
        } = cell
        {
            let pass = Pass {
                suffix,
                declared: *declared,
                site_pages: pages,
                scenarios,
                tools,
                rows,
                done: of_runs(runs),
                stopped: of_stopped(stopped),
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
        stopped,
    } = &trials.growth
    {
        findings.extend(check_pass(&Pass {
            suffix: " growth",
            declared: *declared,
            site_pages: *site_pages,
            scenarios: &counted,
            tools: &Tool::ALL,
            rows: EVERY_ROW,
            done: of_trials(trials_run),
            stopped: of_stopped(stopped),
        }));
    }
    findings
}

/// Each finding of one pass: a measured cell short of or past its declared
/// trials, every trial that did other work ([`other_work`]), and every
/// one whose output is refused ([`other_output`]).
fn check_pass(pass: &Pass<'_>) -> Vec<Finding> {
    let mut findings = short_or_past(pass);
    findings.extend(other_work(pass).into_iter().map(|(_, finding)| finding));
    findings.extend(other_output(pass));
    findings
}

/// Every trial of a pass on the article site whose output is refused:
/// Foxhound's whose pages differ from the right answer under its cell's
/// check, and any tool's that no check read, on a site whose answers the
/// Proving ground serves.
fn other_output(pass: &Pass<'_>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for done in &pass.done {
        let site_pages = done.scenario.site_pages(pass.site_pages);
        let Some((cell, check)) = matrix::cell(done.scenario, done.cell)
            .and_then(|cell| Some((cell, cell.output_check()?)))
            .filter(|_| site_pages <= ANSWERED_PAGES)
        else {
            continue;
        };
        let Some(extracted) = done.extracted else {
            findings.push(Finding::new(
                "output_unchecked",
                pass.subject(done),
                "no output check read the pages it wrote, so its figures are of no known output",
            ));
            continue;
        };
        let (differing, held) = match check {
            OutputCheck::Whole => (&extracted.whole, ""),
            OutputCheck::AdAllowed => (&extracted.ad_allowed, ", the ad allowed"),
        };
        if cell.tool == matrix::Tool::Foxhound && differing.pages > 0 {
            findings.push(Finding::new(
                "output_differs",
                pass.subject(done),
                format!(
                    "{} of the site's {site_pages} pages differ from the right answer{held}; the first: {}",
                    differing.pages,
                    differing.first.as_deref().unwrap_or("not named")
                ),
            ));
        }
    }
    findings
}

/// Each measured cell of a pass short of or past its declared trials.
fn short_or_past(pass: &Pass<'_>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (cell, _) in matrix::measured().filter(|(cell, config)| {
        pass.scenarios.contains(&cell.scenario)
            && pass.tools.contains(&config.tool())
            && pass.rows.contains(&cell.row)
            && !pass.stopped.contains(&(cell.scenario, cell.name))
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
    findings
}

/// Every trial of a pass that did other work, with its finding: each whose
/// pages are not the site's, and every other whose work is not the one
/// most of its scenario's trials share.
fn other_work<'a, 'b>(pass: &'a Pass<'b>) -> Vec<(&'a Done<'b>, Finding)> {
    let mut findings = Vec::new();
    for scenario in pass.scenarios {
        let site_pages = scenario.site_pages(pass.site_pages);
        let mut whole = Vec::new();
        for done in pass.done.iter().filter(|done| done.scenario == *scenario) {
            let work = done.work;
            if work.is_the_site_of(site_pages) {
                whole.push(done);
                continue;
            }
            // On the script-rendered site, every page past the first is
            // linked only by a script: a trial that missed some never
            // rendered their links.
            if scenario.renders_links() && work.extra.is_empty() && work.pages > 0 {
                findings.push((
                    done,
                    Finding::new(
                        "unrendered_pages",
                        pass.subject(done),
                        format!(
                            "{} of the site's {} pages its script links not reached",
                            work.missing.len(),
                            site_pages.saturating_sub(1)
                        ),
                    ),
                ));
                continue;
            }
            findings.push((
                done,
                Finding::new(
                    "different_pages",
                    pass.subject(done),
                    format!(
                        "{} of the site's {} pages missing and {} other pages fetched",
                        work.missing.len(),
                        site_pages,
                        work.extra.len()
                    ),
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
        // The work most of the main rows' trials that fetched the site
        // share; none when two values are shared by as many trials. A
        // sensitivity row is held to it without a vote: Spider's would
        // outnumber the other tools' rows together.
        let mut shared: BTreeMap<(u64, u64), usize> = BTreeMap::new();
        for done in whole.iter().filter(|done| is_main(done)) {
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
                    "{count} {what} {bytes} bytes, where {trials} of the main rows' {} trials that fetched the site had {held_count} {what} {held_bytes}",
                    shared.values().sum::<usize>()
                ),
                None => format!(
                    "{count} {what} {bytes} bytes, and no count and bytes is shared by more trials than another"
                ),
            };
            findings.push((
                done,
                Finding::new("different_bytes", pass.subject(done), message),
            ));
        }
    }
    findings
}

/// Sets aside each cell of the system-call cell one of whose runs did
/// other work: the cell's runs leave, and its first such run joins the
/// stopped runs with what it did and the pages it reached, so the report
/// reads the cell's system calls as not measured and [`check`] holds
/// nothing against it. The timed trials, the growth cell and the Callgrind
/// cell keep the check as it is, so a tool that does other work without
/// strace still fails the report there.
pub(crate) fn strace_runs_of_other_work_set_aside(trials: &mut Comparison) {
    let Some(pages) = site_pages(&trials.declared.site) else {
        return;
    };
    let CountedCell::Measured {
        trials: declared,
        runs,
        stopped,
    } = &mut trials.syscalls
    else {
        return;
    };
    let pass = Pass {
        suffix: " strace",
        declared: *declared,
        site_pages: pages,
        scenarios: &trials.declared.scenarios,
        tools: &Tool::ALL,
        rows: EVERY_ROW,
        done: of_runs(runs),
        stopped: of_stopped(stopped),
    };
    let mut aside: Vec<StoppedRun> = Vec::new();
    for (done, finding) in other_work(&pass) {
        if aside
            .iter()
            .any(|run| run.scenario == done.scenario && run.cell == done.cell)
        {
            continue;
        }
        aside.push(StoppedRun {
            scenario: done.scenario,
            cell: done.cell.to_owned(),
            tool: runs
                .iter()
                .find(|run| run.scenario == done.scenario && run.cell == done.cell)
                .expect("invariant: a finding names a run of `runs`")
                .tool,
            number: done.number,
            reason: format!(
                "under `strace -f`, run {} did other work, reaching {} of the site's {pages} pages: {}",
                done.number, done.work.pages, finding.message
            ),
        });
    }
    runs.retain(|run| {
        !aside
            .iter()
            .any(|stop| stop.scenario == run.scenario && stop.cell == run.cell)
    });
    stopped.extend(aside);
}

#[cfg(test)]
mod tests;
