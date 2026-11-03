//! The same-work check: before any figure counts, every trial of both tools
//! fetched the site's own pages and was served the requests and bytes most
//! trials were, as the Proving ground's own observations record them, never
//! a tool's report (spec #152: the Proving ground is the benchmark's only
//! source of bytes). No trial is the others' reference, so the trial that
//! did other work is the one named, whichever ran first (ADR 0134's #1574
//! note). The Callgrind cell's runs are held to it among themselves, as a
//! crawl under Callgrind is another cell of the comparison (ADR 0140).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::check::Finding;

use super::trials::{Comparison, InstructionCell, Tool};

/// What the Proving ground saw one trial do on the site.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Work {
    /// The site's paths it answered `200`, once each.
    pub(crate) pages: BTreeSet<String>,
    /// Its requests for them.
    pub(crate) requests: u64,
    /// The bytes it sent answering them.
    pub(crate) bytes: u64,
}

/// The work `observations` (`GET /observations`) records on `host` under
/// the path prefix `site`: every `GET` it answered `200`. Anything else,
/// such as a robots.txt, is no page of the site.
pub(crate) fn work_of(observations: &Value, host: &str, site: &str) -> Result<Work> {
    let requests = observations["requests"]
        .as_array()
        .context("the observations hold no requests")?;
    let unrecorded = observations["unrecorded_requests"].as_u64().unwrap_or(0);
    anyhow::ensure!(
        unrecorded == 0,
        "the Proving ground left {unrecorded} requests unrecorded"
    );
    let mut work = Work {
        pages: BTreeSet::new(),
        requests: 0,
        bytes: 0,
    };
    for request in requests {
        let on_host = request["host"].as_str().is_some_and(|seen| {
            seen == host || seen.split_once(':').is_some_and(|(name, _)| name == host)
        });
        let Some(target) = request["target"].as_str() else {
            continue;
        };
        if !on_host
            || request["method"] != "GET"
            || request["status"] != 200
            || !target.starts_with(site)
        {
            continue;
        }
        work.pages.insert(target.to_owned());
        work.requests += 1;
        work.bytes += request["bytes"]["sent"]
            .as_u64()
            .with_context(|| format!("the request for {target} counts no bytes sent"))?;
    }
    Ok(work)
}

/// The paths of the site whose first page is `first`, `/sites/<n>/1`:
/// `/sites/<n>/1` to `/sites/<n>/<n>`.
fn site_pages(first: &str) -> Option<BTreeSet<String>> {
    let pages: u64 = first
        .strip_prefix("/sites/")?
        .strip_suffix("/1")?
        .parse()
        .ok()?;
    Some(
        (1..=pages)
            .map(|page| format!("/sites/{pages}/{page}"))
            .collect(),
    )
}

/// What one trial of a cell did, whichever cell.
struct Done<'a> {
    tool: Tool,
    number: u32,
    pages: &'a [String],
    requests: u64,
    bytes: u64,
}

/// The trials of one cell, with what it declared.
struct Cell<'a> {
    /// `<tool>` or `<tool> callgrind`, as a finding names the tool.
    suffix: &'static str,
    declared: u32,
    done: Vec<Done<'a>>,
}

impl Cell<'_> {
    fn tool(&self, tool: Tool) -> String {
        format!("{}{}", tool.name(), self.suffix)
    }

    fn subject(&self, done: &Done<'_>) -> String {
        format!("{} trial {}", self.tool(done.tool), done.number)
    }
}

/// Each finding that keeps the figures from counting, in the timed trials and
/// the Callgrind cell: see [`check_cell`]. A Callgrind run that counted no
/// instruction measured nothing.
pub(crate) fn check(trials: &Comparison) -> Vec<Finding> {
    let Some(site) = site_pages(&trials.declared.site) else {
        return vec![Finding::new(
            "unknown_site",
            trials.declared.site.clone(),
            "the declared site's first page is no `/sites/<n>/1`",
        )];
    };
    let mut findings = check_cell(
        &Cell {
            suffix: "",
            declared: trials.declared.trials,
            done: trials
                .trials
                .iter()
                .map(|trial| Done {
                    tool: trial.tool,
                    number: trial.number,
                    pages: &trial.pages,
                    requests: trial.requests,
                    bytes: trial.bytes,
                })
                .collect(),
        },
        &site,
    );
    if let InstructionCell::Measured {
        trials: declared,
        runs,
        ..
    } = &trials.instructions
    {
        let cell = Cell {
            suffix: " callgrind",
            declared: *declared,
            done: runs
                .iter()
                .map(|run| Done {
                    tool: run.tool,
                    number: run.number,
                    pages: &run.pages,
                    requests: run.requests,
                    bytes: run.bytes,
                })
                .collect(),
        };
        findings.extend(check_cell(&cell, &site));
        for (run, done) in runs.iter().zip(&cell.done) {
            if run.instructions == 0 {
                findings.push(Finding::new(
                    "no_instructions",
                    cell.subject(done),
                    "Callgrind counted no instruction, so the run measured nothing",
                ));
            }
        }
    }
    findings
}

/// Each finding of one cell: a tool short of or past its declared trials,
/// every trial whose pages are not the declared site's, and every other trial
/// whose requests and bytes are not the ones most trials share.
fn check_cell(cell: &Cell<'_>, site: &BTreeSet<String>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for tool in Tool::ALL {
        let count = cell.done.iter().filter(|done| done.tool == tool).count();
        if u32::try_from(count).ok() != Some(cell.declared) {
            findings.push(Finding::new(
                "trial_count",
                cell.tool(tool),
                format!("{count} trials, where {} were declared", cell.declared),
            ));
        }
    }
    let mut whole = Vec::new();
    for done in &cell.done {
        let pages: BTreeSet<String> = done.pages.iter().cloned().collect();
        if pages == *site {
            whole.push(done);
            continue;
        }
        let missing = site.difference(&pages).count();
        let extra = pages.difference(site).count();
        findings.push(Finding::new(
            "different_pages",
            cell.subject(done),
            format!(
                "{missing} of the site's {} pages missing and {extra} other pages fetched",
                site.len()
            ),
        ));
    }
    // The requests and bytes most of the trials that fetched the site share;
    // none when two values are shared by as many trials.
    let mut shared: BTreeMap<(u64, u64), usize> = BTreeMap::new();
    for done in &whole {
        *shared.entry((done.requests, done.bytes)).or_default() += 1;
    }
    let most = shared.values().copied().max().unwrap_or(0);
    let mut held = shared.iter().filter(|(_, count)| **count == most);
    let reference = match (held.next(), held.next()) {
        (Some((work, count)), None) => Some((*work, *count)),
        _ => None,
    };
    for done in whole {
        let message = match reference {
            Some((work, _)) if work == (done.requests, done.bytes) => continue,
            Some(((requests, bytes), count)) => format!(
                "{} requests served {} bytes, where {count} of the {} trials that fetched the site had {requests} requests served {bytes}",
                done.requests,
                done.bytes,
                shared.values().sum::<usize>()
            ),
            None => format!(
                "{} requests served {} bytes, and no count of requests and bytes is shared by more trials than another",
                done.requests, done.bytes
            ),
        };
        findings.push(Finding::new("different_bytes", cell.subject(done), message));
    }
    findings
}

#[cfg(test)]
mod tests;
