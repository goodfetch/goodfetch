//! The trials file `run` writes and `report` reads: what was declared
//! before the first trial, the machine, each trial's figures with the work
//! the Proving ground saw it do, and the Callgrind cell's runs with theirs.

use serde::{Deserialize, Serialize};

/// Trials of each tool, interleaved in random order.
pub(crate) const TRIALS: u32 = 10;
/// The statistic each figure is reported as.
pub(crate) const STATISTIC: &str = "median";
/// Requests in flight at once, for each tool: the resident-memory check's
/// crawl's, M1's default of `crawl.handler.concurrency`.
pub(crate) const CONCURRENCY: u32 = crate::rss::crawl::CONCURRENCY;
/// The pages of the site both tools crawl: the resident-memory check's
/// crawl's.
pub(crate) const SITE_PAGES: u64 = crate::rss::crawl::PAGES;
/// Spider's pinned version, which `xtask/compare-spider/Cargo.toml` pins
/// (a unit test holds the two equal).
pub(crate) const SPIDER_VERSION: &str = "2.53.9";

/// Runs of each tool in the Callgrind cell, whose figure is their median:
/// fewer than the timed trials, since a count barely varies and a run under
/// Callgrind takes minutes.
pub(crate) const INSTRUCTION_RUNS: u32 = 3;
/// What the Callgrind cell crawls with.
pub(crate) const INSTRUCTION_SCENARIO: &str = "scenario 1 of ADR 0138's matrix";

/// A tool compared.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Tool {
    /// Foxhound's executable, a code-free crawl on its `http` tier.
    Foxhound,
    /// Spider, in its HTTP mode.
    Spider,
}

impl Tool {
    pub(crate) const ALL: [Self; 2] = [Self::Foxhound, Self::Spider];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Foxhound => "foxhound",
            Self::Spider => "spider",
        }
    }
}

/// What was declared before the first trial.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Declared {
    /// Trials of each tool.
    pub(crate) trials: u32,
    pub(crate) statistic: String,
    pub(crate) concurrency: u32,
    /// The site's first page, where each crawl starts.
    pub(crate) site: String,
    pub(crate) spider: String,
}

/// The machine the trials ran on: never a published host (ADR 0076).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Machine {
    /// `local-controlled`.
    pub(crate) class: String,
    pub(crate) os: String,
    pub(crate) arch: String,
    pub(crate) cpu: String,
    pub(crate) threads: usize,
}

/// One trial: one tool, started in a fresh process, crawling the site
/// served by a fresh Proving ground.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Trial {
    pub(crate) tool: Tool,
    /// Its number among the tool's trials, from 1.
    pub(crate) number: u32,
    /// From the tool's start to its exit.
    pub(crate) seconds: f64,
    /// The tool's peak resident memory, as the OS records it.
    pub(crate) peak_bytes: u64,
    /// The site's paths the Proving ground answered `200`, sorted, once
    /// each.
    pub(crate) pages: Vec<String>,
    /// The requests for the site's paths it answered `200`.
    pub(crate) requests: u64,
    /// The bytes it sent answering them: heads and bodies as framed.
    pub(crate) bytes: u64,
}

/// The trials file: one comparison.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Comparison {
    pub(crate) declared: Declared,
    pub(crate) machine: Machine,
    /// The commit both tools' trials ran at.
    pub(crate) commit: String,
    /// The seed of the trials' order.
    pub(crate) order_seed: u64,
    pub(crate) trials: Vec<Trial>,
    pub(crate) instructions: InstructionCell,
}

/// One run of the Callgrind cell: one tool, in a fresh process under
/// Callgrind, crawling the site served by a fresh Proving ground.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct InstructionRun {
    pub(crate) tool: Tool,
    /// Its number among the tool's runs, from 1.
    pub(crate) number: u32,
    /// Callgrind's total of the whole process, its start and exit included.
    pub(crate) instructions: u64,
    /// As a [`Trial`]'s.
    pub(crate) pages: Vec<String>,
    pub(crate) requests: u64,
    pub(crate) bytes: u64,
}

/// The Callgrind cell, which gives `foxhound.spider_instruction_ratio` (ADR
/// 0140).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum InstructionCell {
    /// This host cannot run it.
    NotRun { reason: String },
    Measured {
        scenario: String,
        /// Runs of each tool.
        trials: u32,
        runs: Vec<InstructionRun>,
    },
}

#[cfg(test)]
mod tests;
