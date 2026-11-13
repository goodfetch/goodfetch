//! The trials file `run` writes and `report` reads: what was declared
//! before the first trial, the machine, each trial's figures with the work
//! the Proving ground saw it do, and the counted cells' runs with theirs.

use serde::{Deserialize, Serialize};

use super::hostile::Broken;
use super::matrix::Scenario;
pub(crate) use super::matrix::Tool;

/// Trials of each measured cell, interleaved in random order.
pub(crate) const TRIALS: u32 = 10;
/// The 1-minute load average under which a machine counts as quiet: each
/// timed trial waits for it before it starts.
pub(crate) const QUIET_LOAD: f64 = 4.0;
/// The longest a timed trial waits for a quiet machine before it starts
/// anyway, its load recorded.
pub(crate) const QUIET_WAIT_SECONDS: u64 = 30 * 60;
/// The statistic each figure is reported as, with its spread.
pub(crate) const STATISTIC: &str = "median, with the lowest and highest trial";
/// Requests in flight at once, for each tool: the resident-memory check's
/// crawl's, M1's default of `crawl.handler.concurrency`.
pub(crate) const CONCURRENCY: u32 = crate::rss::crawl::CONCURRENCY;
/// The pages of the site every timed trial crawls: the resident-memory
/// check's crawl's.
pub(crate) const SITE_PAGES: u64 = crate::rss::crawl::PAGES;
/// The pages of the site the growth cell crawls, and the crawl sizes its
/// resident memory is read at.
pub(crate) const GROWTH_PAGES: u64 = 100_000;
pub(crate) const MILESTONES: [u64; 3] = [1_000, 10_000, 100_000];
/// Spider's pinned version, which `xtask/compare-spider/Cargo.toml` pins
/// (a unit test holds the two equal).
pub(crate) const SPIDER_VERSION: &str = "2.53.9";
/// Crawlee's pinned version, `@crawlee/cheerio`'s in
/// `xtask/compare-crawlee/package.json`, and Scrapy's, in
/// `xtask/compare-scrapy/pyproject.toml` (unit tests hold each equal).
pub(crate) const CRAWLEE_VERSION: &str = "3.18.2";
pub(crate) const SCRAPY_VERSION: &str = "2.19.0";
/// crawlee-rs's pinned commit, which has no release: the `rev` of
/// `xtask/compare-crawlee-rs/Cargo.toml`'s `crawlee` (a unit test holds
/// the two equal).
pub(crate) const CRAWLEE_RS_COMMIT: &str = "2a3ed90b8391c86005ce6ed8dd3e3248d0d34579";
/// Playwright's pinned version, which both sides' headless browser crawls
/// drive Chrome with (`playwright` in `xtask/compare-crawlee/package.json`
/// and `xtask/compare-scrapy/pyproject.toml`), and `scrapy-playwright`'s,
/// in the latter (unit tests hold each equal).
pub(crate) const PLAYWRIGHT_VERSION: &str = "1.64.1";
pub(crate) const SCRAPY_PLAYWRIGHT_VERSION: &str = "0.0.48";

/// Runs of each tool in the Callgrind cell, whose figure is their median:
/// fewer than the timed trials, since a count barely varies and a run under
/// Callgrind takes minutes.
pub(crate) const INSTRUCTION_RUNS: u32 = 3;
/// The tools the Callgrind cell counts.
pub(crate) const INSTRUCTION_TOOLS: [Tool; 2] = [Tool::Foxhound, Tool::Spider];
/// What the Callgrind cell crawls with.
pub(crate) const INSTRUCTION_SCENARIO: &str = "scenario 1 of ADR 0138's matrix";
/// Runs of each hostile pages' cell (#2439), at most: its counts are read
/// with their spread, not timed.
pub(crate) const RULE_RUNS: u32 = 3;

/// What was declared before the first trial.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Declared {
    /// Trials of each measured cell, and runs of each in the system-call
    /// cell and the growth cell.
    pub(crate) trials: u32,
    pub(crate) statistic: String,
    pub(crate) concurrency: u32,
    /// The site's first page, where each timed crawl starts.
    pub(crate) site: String,
    pub(crate) spider: String,
    pub(crate) crawlee: String,
    pub(crate) scrapy: String,
    /// crawlee-rs's commit.
    pub(crate) crawlee_rs: String,
    pub(crate) playwright: String,
    pub(crate) scrapy_playwright: String,
    /// The Chrome for Testing build every tool of the headless browser
    /// scenario runs.
    pub(crate) browser: String,
    /// The scenarios whose cells ran: every scenario, unless the run asked
    /// for some.
    #[serde(default = "every_scenario")]
    pub(crate) scenarios: Vec<Scenario>,
    /// The command that reruns the run: the recipe and the arguments it
    /// was started with.
    pub(crate) command: String,
}

fn every_scenario() -> Vec<Scenario> {
    Scenario::ALL.to_vec()
}

/// The machine the trials ran on, `local-controlled`: its figures compare
/// with each other, never with another machine's (ADR 0025's notes of
/// 2026-11-10).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Machine {
    /// `local-controlled`.
    pub(crate) class: String,
    /// The name the run was given for it (`--machine`), such as "Hetzner
    /// Cloud CCX43, 16 dedicated vCPUs"; none when it was given none.
    pub(crate) name: Option<String>,
    pub(crate) os: String,
    pub(crate) arch: String,
    pub(crate) cpu: String,
    pub(crate) threads: usize,
    /// Its memory, where the OS gives it.
    pub(crate) memory_bytes: Option<u64>,
    /// The 1-minute load average when the first trial started, where the
    /// OS gives one: a quiet machine is under 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) load: Option<f64>,
}

/// What the Proving ground saw a crawl do on the site, the same-work check's
/// input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Work {
    /// The site's pages it answered `200`, once each.
    pub(crate) pages: u64,
    /// The site's pages it never answered `200`, and the paths under the
    /// site's prefix it answered `200` that are no page of the site.
    pub(crate) missing: Vec<String>,
    pub(crate) extra: Vec<String>,
    /// The requests for the site's paths it answered `200`.
    pub(crate) requests: u64,
    /// The bytes it sent answering them: heads and bodies as framed.
    pub(crate) bytes: u64,
    /// The bytes of each page's first `200` answer, the work of a crawl
    /// that fetched some pages again after a kill.
    pub(crate) first_bytes: u64,
}

/// The p50, p95 and p99 of a trial's per-page times, in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Percentiles {
    pub(crate) p50: f64,
    pub(crate) p95: f64,
    pub(crate) p99: f64,
}

/// What a page's time runs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LatencyEnd {
    /// The commit of its Record.
    Record,
    /// Its answer, for a tool that keeps no Record.
    Fetch,
}

/// The process tree's figures, sampled from `/proc` every
/// [`super::sampler::INTERVAL_MILLISECONDS`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Sampled {
    pub(crate) samples: u64,
    /// The whole tree's resident memory: its highest sample and its
    /// time-weighted mean.
    pub(crate) peak_bytes: u64,
    pub(crate) average_bytes: u64,
    /// The whole tree's PSS at its highest sample: resident memory with
    /// each page two of its processes share counted once. None when a
    /// sample could not read a live process's.
    pub(crate) peak_pss_bytes: Option<u64>,
    /// The most cores the tree used over any window of
    /// [`super::sampler::CORE_WINDOW_SAMPLES`] samples; none for a crawl
    /// that ended within one window.
    pub(crate) peak_cores: Option<f64>,
    pub(crate) peak_threads: u64,
    pub(crate) peak_open_files: u64,
    /// The tree's resident memory when the site's `n`-th page was
    /// answered, for each milestone the crawl reached: `(n, bytes)`.
    pub(crate) at_pages: Vec<(u64, u64)>,
}

/// One timed trial: one cell's tool, started in a fresh process (two, when
/// its scenario resumes), crawling the site served by a fresh Proving
/// ground. Every figure is measured from outside the tool: the OS's
/// accounting of its waited-for children, `/proc`, the Proving ground's
/// observations, and the files it left.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Trial {
    pub(crate) scenario: Scenario,
    pub(crate) cell: String,
    pub(crate) tool: Tool,
    /// Its number among the cell's trials, from 1.
    pub(crate) number: u32,
    /// The pages of the site it crawled.
    pub(crate) site_pages: u64,
    /// The 1-minute load average when it started, where the OS gives one:
    /// a quiet machine is under [`QUIET_LOAD`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) load: Option<f64>,
    /// Why its first attempt was stopped, when the comparison stopped it
    /// and these are its retry's figures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) retried: Option<String>,
    #[serde(flatten)]
    pub(crate) measured: Measured,
}

/// What one trial measured.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Measured {
    /// From the tool's start to its last exit.
    pub(crate) seconds: f64,
    /// The tool's processes it took: 1, or 2 when killed and resumed.
    pub(crate) processes: u32,
    pub(crate) user_seconds: f64,
    pub(crate) system_seconds: f64,
    /// The largest resident set any one of its processes reached, as the
    /// OS records it (`getrusage`).
    pub(crate) peak_bytes: u64,
    /// `/proc`'s samples, absent on an OS without it.
    pub(crate) sampled: Option<Sampled>,
    /// Bytes read from and written to storage (`getrusage`'s block counts,
    /// the I/O counters of every process of the tree).
    pub(crate) read_bytes: u64,
    pub(crate) written_bytes: u64,
    /// What its data folder holds on disk once it exited.
    pub(crate) footprint_bytes: u64,
    /// The size of the executable run; for a tool run by an interpreter,
    /// the interpreter's executable and the packages its side installed.
    pub(crate) executable_bytes: u64,
    pub(crate) work: Work,
    /// Every request the Proving ground received on the site's host from
    /// the tool, robots.txt, retries and duplicates included.
    pub(crate) requests_sent: u64,
    /// Requests for a site page it had already answered `200`.
    pub(crate) duplicates: u64,
    /// Answers other than `200` on the site's host.
    pub(crate) errors: u64,
    /// Bytes on the wire the tool sent to the site's host and received
    /// from it, every byte of its connections.
    pub(crate) sent_bytes: u64,
    pub(crate) received_bytes: u64,
    pub(crate) connections: u64,
    pub(crate) tls_connections: u64,
    /// From its start to its first request, its first page's answer and
    /// its first Record's commit (none for a tool with no Record).
    pub(crate) startup_seconds: f64,
    pub(crate) first_page_seconds: f64,
    pub(crate) first_record_seconds: Option<f64>,
    /// Pages answered in each tenth of the trial's time, as pages per
    /// second.
    pub(crate) curve: Vec<f64>,
    /// Each page's time from its discovery, the answer of the page that
    /// links it (the start, for the first page), to its end.
    pub(crate) latency: Percentiles,
    pub(crate) latency_end: Option<LatencyEnd>,
}

/// One run of a counted cell: one cell's tool in a fresh process under a
/// counting tool, crawling the site served by a fresh Proving ground.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CountedRun {
    pub(crate) scenario: Scenario,
    pub(crate) cell: String,
    pub(crate) tool: Tool,
    /// Its number among the cell's runs, from 1.
    pub(crate) number: u32,
    /// Callgrind's instructions, or strace's system calls.
    pub(crate) count: u64,
    /// strace's calls of `fsync`, `fdatasync`, `sync_file_range`,
    /// `syncfs` and `sync`; 0 for a Callgrind run.
    #[serde(default)]
    pub(crate) syncs: u64,
    pub(crate) work: Work,
}

/// A counted cell: Callgrind's instructions or strace's system calls.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CountedCell {
    /// This host cannot run it, or the run did not ask for it.
    NotRun { reason: String },
    Measured {
        /// Runs of each cell.
        trials: u32,
        runs: Vec<CountedRun>,
        /// The runs the comparison stopped, their crawl no longer
        /// progressing; each leaves its cell not measured, and the cell's
        /// later runs do not run.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stopped: Vec<StoppedRun>,
    },
}

/// A run stopped before its crawl finished: [`CountedCell`]'s `stopped`,
/// or a timed trial stopped twice ([`Comparison`]'s `stopped`).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StoppedRun {
    pub(crate) scenario: Scenario,
    pub(crate) cell: String,
    pub(crate) tool: Tool,
    pub(crate) number: u32,
    /// Why, from what the run measured.
    pub(crate) reason: String,
}

/// The growth cell: each measured cell's crawl of the
/// [`GROWTH_PAGES`]-page site, whose resident memory is read at each
/// milestone.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum GrowthCell {
    NotRun {
        reason: String,
    },
    Measured {
        trials: u32,
        site_pages: u64,
        trials_run: Vec<Trial>,
        /// The cells whose trial was stopped twice: not measured.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stopped: Vec<StoppedRun>,
    },
}

/// The trials file: one comparison.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Comparison {
    pub(crate) declared: Declared,
    pub(crate) machine: Machine,
    /// The commit every trial ran at.
    pub(crate) commit: String,
    /// The seed of the trials' order.
    pub(crate) order_seed: u64,
    pub(crate) trials: Vec<Trial>,
    /// The cells whose timed trial was stopped twice, its first attempt
    /// and its retry: not measured, their later trials not run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) stopped: Vec<StoppedRun>,
    /// strace's count of each measured cell's system calls and syncs.
    pub(crate) syscalls: CountedCell,
    pub(crate) growth: GrowthCell,
    /// Callgrind's count of scenario 1's instructions, Foxhound's and
    /// Spider's: ADR 0140's row compares those two, and instructions are no
    /// metric of the owner's list for the other tools.
    pub(crate) instructions: CountedCell,
    /// The hostile pages' cells' runs: the rules each tool broke (#2439).
    pub(crate) rules: Vec<RuledRun>,
}

/// One run of a hostile pages' cell: the rules its tool broke on each task
/// of its pass, read from the Proving ground's findings.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RuledRun {
    pub(crate) scenario: Scenario,
    pub(crate) cell: String,
    pub(crate) tool: Tool,
    pub(crate) number: u32,
    pub(crate) tasks: Vec<Broken>,
}

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;
