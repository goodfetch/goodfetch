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
//! other cells of the comparison (ADR 0140), and the sizes cell's trials at
//! each size and concurrency among themselves (spec #2165). The long
//! scenario's site is of its own length, and its cells run only their timed
//! trials. A strace run that did other work fails nothing: strace's
//! slowdown alone can keep a headless browser crawl from its last pages, so
//! that cell's system calls are not measured, with the pages the run
//! reached, and the timed figures count
//! ([`strace_runs_of_other_work_set_aside`]).
//!
//! On the article site, the same work is also the same output (#2136):
//! each trial's pages were held to the site's right answers as it ended
//! ([`super::extracted`]). A trial of Foxhound's whose pages differ under
//! its cell's check is refused here, in every pass; a competitor's keeps
//! its figures, which the report labels; and a trial no check read is no
//! figure, while the Proving ground serves the site's answers.
//!
//! An auto cell's run (#2411) did the same work when the Proving ground saw
//! every page of the rungs site's manifest requested; the tools render
//! different shares of its pages, so their bytes are not compared, and the
//! Records each page gave are the report's pages wrong.

use std::collections::BTreeMap;

use crate::check::Finding;

use super::extracted::{ANSWERED_PAGES, Extracted};
use super::matrix::{self, OutputCheck, Row, Scenario};
use super::trials::{
    AutoRun, Comparison, CountedCell, CountedRun, GrowthCell, INSTRUCTION_SCENARIOS,
    INSTRUCTION_TOOLS, SIZE_CONCURRENCIES, SIZE_TOOLS, SIZES, SizesCell, StoppedRun, Tool, Trial,
    Work, size_not_run,
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

/// One pass of trials: the timed one, a counted cell, the growth cell or
/// one size and concurrency of the sizes cell.
struct Pass<'a> {
    /// ` callgrind`, ` strace`, ` growth`, ` sizes 100 pages at 16` or
    /// nothing, as a finding names the cell.
    suffix: String,
    declared: u32,
    /// The pages of the comparison's site, from which each scenario's
    /// ([`Scenario::site_pages`]).
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

fn of_trials<'a>(trials: impl IntoIterator<Item = &'a Trial>) -> Vec<Done<'a>> {
    trials
        .into_iter()
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

/// Whether a trial's row fetches some of its pages twice, on `http` and
/// then in a browser: Foxhound's with `fetch.tier` left at `auto`, whose
/// samples do (#2283).
fn samples(done: &Done<'_>) -> bool {
    matrix::measured().any(|(cell, config)| {
        cell.scenario == done.scenario && cell.name == done.cell && config.samples()
    })
}

/// Main and sensitivity rows alike.
const EVERY_ROW: &[Row] = &[Row::Main, Row::Sensitivity];

/// The cells of stopped runs or trials, not measured.
fn of_stopped<'a>(stopped: impl IntoIterator<Item = &'a StoppedRun>) -> Vec<(Scenario, &'a str)> {
    stopped
        .into_iter()
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
        suffix: String::new(),
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
                suffix: suffix.to_owned(),
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
            suffix: " growth".to_owned(),
            declared: *declared,
            site_pages: *site_pages,
            scenarios: &counted,
            tools: &Tool::ALL,
            rows: EVERY_ROW,
            done: of_trials(trials_run),
            stopped: of_stopped(stopped),
        }));
    }
    let raw: &[Scenario] = if ran.contains(&Scenario::Raw) {
        &[Scenario::Raw]
    } else {
        &[]
    };
    findings.extend(check_sizes(&trials.sizes, raw));
    findings.extend(check_auto(&trials.auto));
    findings
}

/// Each auto cell's run that left a page of the rungs site's manifest
/// unrequested (#2411): its tool's Records are held to the manifest as the
/// report's pages wrong, never here.
fn check_auto(runs: &[AutoRun]) -> Vec<Finding> {
    runs.iter()
        .filter(|run| !run.judged.same_work())
        .map(|run| {
            Finding::new(
                "unrequested_pages",
                format!("{} {} run {}", run.scenario.name(), run.cell, run.number),
                format!(
                    "the Proving ground saw {} of the manifest's {} pages requested",
                    run.judged.requested, run.judged.pages
                ),
            )
        })
        .collect()
}

/// Each finding of the sizes cell, one pass per size and concurrency, of
/// the scenario 1 main cells of [`SIZE_TOOLS`] it runs there
/// ([`size_not_run`]) when `raw` holds scenario 1.
fn check_sizes(sizes: &SizesCell, raw: &[Scenario]) -> Vec<Finding> {
    let SizesCell::Measured {
        trials: declared,
        trials_run,
        stopped,
    } = sizes
    else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for site_pages in SIZES {
        for concurrency in SIZE_CONCURRENCIES {
            let at = |pages, at_concurrency| pages == site_pages && at_concurrency == concurrency;
            let tools: Vec<Tool> = SIZE_TOOLS
                .into_iter()
                .filter(|&tool| size_not_run(tool, concurrency).is_none())
                .collect();
            findings.extend(check_pass(&Pass {
                suffix: format!(" sizes {site_pages} pages at {concurrency}"),
                declared: *declared,
                site_pages,
                scenarios: raw,
                tools: &tools,
                rows: &[Row::Main],
                done: of_trials(
                    trials_run
                        .iter()
                        .filter(|trial| at(trial.site_pages, trial.concurrency)),
                ),
                stopped: of_stopped(
                    stopped
                        .iter()
                        .filter(|point| at(point.site_pages, point.concurrency))
                        .map(|point| &point.run),
                ),
            }));
        }
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
/// check, any tool's whose Records' fields differ from the handled crawl's
/// answers (#2317), and any tool's that no check read, on a site whose
/// answers the Proving ground serves.
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
        let differing = extracted.under(check);
        let held = if check == OutputCheck::AdAllowed {
            ", the ad allowed"
        } else {
            ""
        };
        let refused = cell.tool == matrix::Tool::Foxhound || check == OutputCheck::Fields;
        if refused && differing.pages > 0 {
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

/// Each measured cell of a pass short of or past its declared trials, one
/// for a cell that runs one ([`matrix::Cell::one_trial`]).
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
        let declared = cell.runs(pass.declared);
        if u32::try_from(count).ok() != Some(declared) {
            findings.push(Finding::new(
                "trial_count",
                pass.cell(cell.scenario, cell.name),
                format!(
                    "{count} trials, where {declared} {} declared",
                    if declared == 1 { "was" } else { "were" }
                ),
            ));
        }
    }
    findings
}

/// The finding of a trial that did not fetch the site of `site_pages`
/// pages over its scenario's protocol: one that left it (#2284), one of the
/// script-rendered site that missed pages only a script links, and every
/// other whose pages are not the site's; `None` for a trial that fetched
/// the whole site.
fn off_the_site(
    pass: &Pass<'_>,
    scenario: Scenario,
    site_pages: u64,
    done: &Done<'_>,
) -> Option<Finding> {
    let work = done.work;
    if work.other_protocol > 0 {
        return Some(Finding::new(
            "other_protocol",
            pass.subject(done),
            format!(
                "{} of its requests for the site's pages came over no TLS and HTTP/2",
                work.other_protocol
            ),
        ));
    }
    if work.is_the_site_of(site_pages) {
        return None;
    }
    // On the script-rendered site, every page past the first is linked
    // only by a script: a trial that missed some never rendered their
    // links.
    if scenario.renders_links() && work.extra.is_empty() && work.pages > 0 {
        return Some(Finding::new(
            "unrendered_pages",
            pass.subject(done),
            format!(
                "{} of the site's {} pages its script links not reached",
                work.missing.len(),
                site_pages.saturating_sub(1)
            ),
        ));
    }
    Some(Finding::new(
        "different_pages",
        pass.subject(done),
        format!(
            "{} of the site's {} pages missing and {} other pages fetched",
            work.missing.len(),
            site_pages,
            work.extra.len()
        ),
    ))
}

/// Every trial of a pass that did other work, with its finding: each that
/// did not fetch the site over its scenario's protocol ([`off_the_site`]),
/// and every other whose work is not the one most of its scenario's trials
/// share. A trial of a scenario that resumes, or of a
/// row whose samples fetch pages twice, is held to the pages first served
/// and their bytes; every other to the requests served and their bytes.
fn other_work<'a, 'b>(pass: &'a Pass<'b>) -> Vec<(&'a Done<'b>, Finding)> {
    let mut findings = Vec::new();
    for scenario in pass.scenarios {
        let site_pages = scenario.site_pages(pass.site_pages);
        let mut whole = Vec::new();
        for done in pass.done.iter().filter(|done| done.scenario == *scenario) {
            match off_the_site(pass, *scenario, site_pages, done) {
                Some(finding) => findings.push((done, finding)),
                None => whole.push(done),
            }
        }
        let first_served = |done: &Done<'_>| scenario.resumes() || samples(done);
        let key = |work: &Work, first_served: bool| {
            if first_served {
                (work.pages, work.first_bytes)
            } else {
                (work.requests, work.bytes)
            }
        };
        // The work most of the main rows' trials that fetched the site
        // share, by either key; none when two values are shared by as many
        // trials. A sensitivity row is held to it without a vote: Spider's
        // would outnumber the other tools' rows together.
        let held = |first_served: bool| {
            let mut shared: BTreeMap<(u64, u64), usize> = BTreeMap::new();
            for done in whole.iter().filter(|done| is_main(done)) {
                *shared.entry(key(done.work, first_served)).or_default() += 1;
            }
            let most = shared.values().copied().max().unwrap_or(0);
            let mut held = shared.iter().filter(|(_, count)| **count == most);
            let reference = match (held.next(), held.next()) {
                (Some((work, count)), None) => Some((*work, *count)),
                _ => None,
            };
            (reference, shared.values().sum::<usize>())
        };
        let by_requests = held(false);
        let by_pages = held(true);
        for done in whole {
            let first_served = first_served(done);
            let (reference, voters) = if first_served { by_pages } else { by_requests };
            let what = if first_served {
                "pages first served"
            } else {
                "requests served"
            };
            let (count, bytes) = key(done.work, first_served);
            let message = match reference {
                Some((work, _)) if work == (count, bytes) => continue,
                Some(((held_count, held_bytes), trials)) => format!(
                    "{count} {what} {bytes} bytes, where {trials} of the main rows' {voters} trials that fetched the site had {held_count} {what} {held_bytes}"
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
        suffix: " strace".to_owned(),
        declared: *declared,
        site_pages: pages,
        scenarios: &trials.declared.scenarios,
        tools: &Tool::ALL,
        rows: EVERY_ROW,
        done: of_runs(runs),
        stopped: of_stopped(stopped.iter()),
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
                "under `strace -f`, run {} did other work, reaching {} of the site's {} pages: {}",
                done.number,
                done.work.pages,
                done.scenario.site_pages(pages),
                finding.message
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
