//! The report: each tool's median pages per second and median peak resident
//! memory over its trials, labelled `local-controlled` with the machine and
//! never published (ADR 0076), written only once the same-work check
//! passed.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use super::trials::{Comparison, Machine, Tool};

/// What the same-work check found every trial did.
#[derive(Serialize)]
pub(crate) struct SameWork {
    pub(crate) pages: usize,
    pub(crate) bytes: u64,
    pub(crate) trials: usize,
}

/// One tool's figures.
#[derive(Serialize)]
pub(crate) struct Figures {
    pub(crate) pages_per_second: f64,
    pub(crate) peak_bytes: u64,
}

/// The report, as `report.json` holds it.
#[derive(Serialize)]
pub(crate) struct Report<'a> {
    pub(crate) label: &'static str,
    pub(crate) published: bool,
    pub(crate) machine: &'a Machine,
    pub(crate) commit: &'a str,
    pub(crate) declared: &'a super::trials::Declared,
    pub(crate) order_seed: u64,
    pub(crate) same_work: SameWork,
    pub(crate) tools: BTreeMap<&'static str, Figures>,
}

/// The median of `values`, the mean of the middle two for an even count.
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    match sorted.len() {
        0 => f64::NAN,
        length if length % 2 == 0 => f64::midpoint(sorted[middle - 1], sorted[middle]),
        _ => sorted[middle],
    }
}

/// `count` as a float, for a figure.
#[expect(
    clippy::cast_precision_loss,
    reason = "pages and bytes counted here stay far below 2^52, where a float is exact"
)]
fn float(count: u64) -> f64 {
    count as f64
}

/// A trial's pages per second.
fn rate(trial: &super::trials::Trial) -> f64 {
    float(trial.pages.len() as u64) / trial.seconds
}

/// The report of trials that passed the same-work check.
pub(crate) fn of(trials: &Comparison) -> Report<'_> {
    let first = trials.trials.first();
    let mut tools = BTreeMap::new();
    for tool in Tool::ALL {
        let ran: Vec<_> = trials
            .trials
            .iter()
            .filter(|trial| trial.tool == tool)
            .collect();
        let rates: Vec<f64> = ran.iter().map(|trial| rate(trial)).collect();
        let mut peaks: Vec<u64> = ran.iter().map(|trial| trial.peak_bytes).collect();
        peaks.sort_unstable();
        let middle = peaks.len() / 2;
        let peak_bytes = match peaks.len() {
            0 => 0,
            // Whole bytes: the mean of the middle two, rounded down.
            length if length % 2 == 0 => u64::midpoint(peaks[middle - 1], peaks[middle]),
            _ => peaks[middle],
        };
        tools.insert(
            tool.name(),
            Figures {
                pages_per_second: median(&rates),
                peak_bytes,
            },
        );
    }
    Report {
        label: "local-controlled",
        published: false,
        machine: &trials.machine,
        commit: &trials.commit,
        declared: &trials.declared,
        order_seed: trials.order_seed,
        same_work: SameWork {
            pages: first.map_or(0, |trial| trial.pages.len()),
            bytes: first.map_or(0, |trial| trial.bytes),
            trials: trials.trials.len(),
        },
        tools,
    }
}

const MIB: f64 = 1024.0 * 1024.0;

/// The report as Markdown, for the M1 owner test's Reports (ADR 0134).
pub(crate) fn markdown(report: &Report<'_>, trials: &Comparison) -> String {
    let machine = report.machine;
    let declared = report.declared;
    let mut text = format!(
        "# Spider comparison\n\n\
         `local-controlled`, never published (ADR 0076): {} {} on {}, {} threads, at commit `{}`.\n\n\
         Declared before the first trial: {} trials of each tool in random order (seed {}), each tool \
         started in a fresh process against a fresh Proving ground, {} requests at once, from `{}`; \
         Spider {} in its HTTP mode; each figure the {} of its trials.\n\n\
         Same work: all {} trials fetched the same {} pages, served in {} bytes each.\n\n\
         | Tool | Pages per second | Peak RSS |\n|---|---:|---:|\n",
        machine.os,
        machine.arch,
        machine.cpu,
        machine.threads,
        report.commit,
        declared.trials,
        report.order_seed,
        declared.concurrency,
        declared.site,
        declared.spider,
        declared.statistic,
        report.same_work.trials,
        report.same_work.pages,
        report.same_work.bytes,
    );
    for (tool, figures) in &report.tools {
        let _ = writeln!(
            text,
            "| {tool} | {:.1} | {:.1} MiB |",
            figures.pages_per_second,
            float(figures.peak_bytes) / MIB
        );
    }
    text.push_str("\n## Trials, in the order run\n\n| Tool | Trial | Seconds | Pages per second | Peak RSS |\n|---|---:|---:|---:|---:|\n");
    for trial in &trials.trials {
        let _ = writeln!(
            text,
            "| {} | {} | {:.3} | {:.1} | {:.1} MiB |",
            trial.tool.name(),
            trial.number,
            trial.seconds,
            rate(trial),
            float(trial.peak_bytes) / MIB
        );
    }
    text
}
