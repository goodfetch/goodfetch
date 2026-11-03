//! The comparison matrix (the owner's comments of 2026-10-30 on #1577, ADR
//! 0138's Foxhound cells): each scenario sets every tool to the closest
//! equivalent configuration, and each cell writes that mapping. A tool that
//! cannot do what a scenario asks has a cell saying so, and a Foxhound
//! value that does not exist yet is marked pending with the ticket that
//! brings it; neither gives a figure. This slice holds scenarios 1 and 2,
//! Foxhound against Spider, Crawlee and Scrapy; #1903 adds the other
//! scenarios.

use serde::{Deserialize, Serialize};

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
    /// Crawlee's `CheerioCrawler`, on Node.js.
    Crawlee,
    /// Scrapy's default HTTP crawl, on `CPython`.
    Scrapy,
}

impl Tool {
    pub(crate) const ALL: [Self; 4] = [Self::Foxhound, Self::Spider, Self::Crawlee, Self::Scrapy];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Foxhound => "foxhound",
            Self::Spider => "spider",
            Self::Crawlee => "crawlee",
            Self::Scrapy => "scrapy",
        }
    }
}

/// A scenario of the matrix.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Scenario {
    /// Scenario 1, raw fetch and follow: no persistence, no robots.txt, no
    /// delay, 16 requests in flight.
    Raw,
    /// Scenario 2, durable and resumable: state on disk, the crawl killed
    /// once half the site is served, then resumed to its end.
    Durable,
}

impl Scenario {
    pub(crate) const ALL: [Self; 2] = [Self::Raw, Self::Durable];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Durable => "durable",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Raw => {
                "1. Raw fetch and follow: no persistence, no robots.txt, no delay, 16 in flight"
            }
            Self::Durable => {
                "2. Durable and resumable: state on disk, killed once half the site is served, then resumed"
            }
        }
    }

    /// Whether each trial is killed once half the site is served, then
    /// resumed: its re-fetches are no other work, so the same-work check
    /// holds the bytes of each page's first answer instead of every
    /// request's.
    pub(crate) fn resumes(self) -> bool {
        matches!(self, Self::Durable)
    }
}

/// How a measured cell runs its tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Config {
    /// Foxhound's code-free crawl on its `http` tier at this
    /// `run_state.durability`, robots.txt not obeyed, `politeness.domain`
    /// delay 0 and concurrency 16, no format.
    Foxhound { durability: &'static str },
    /// Spider's HTTP mode, robots.txt not obeyed, 16 at once.
    Spider,
    /// Crawlee's `CheerioCrawler` at most 16 at once, robots.txt not obeyed
    /// (its default), nothing kept on disk (`persistStorage: false`).
    Crawlee,
    /// Scrapy's default crawl, `ROBOTSTXT_OBEY: False`, 16 at once in all
    /// and per domain, no delay, nothing kept on disk.
    Scrapy,
}

impl Config {
    /// The tool this configuration runs.
    pub(crate) fn tool(self) -> Tool {
        match self {
            Self::Foxhound { .. } => Tool::Foxhound,
            Self::Spider => Tool::Spider,
            Self::Crawlee => Tool::Crawlee,
            Self::Scrapy => Tool::Scrapy,
        }
    }
}

/// What a cell gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Measured(Config),
    /// The tool cannot do what the scenario asks.
    NotSupported(&'static str),
    /// Foxhound's value does not exist yet.
    Pending(&'static str),
}

/// One cell: a tool's configuration in one scenario.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cell {
    pub(crate) scenario: Scenario,
    pub(crate) tool: Tool,
    /// Its name in the trials and the report, such as `foxhound` or
    /// `foxhound periodic`.
    pub(crate) name: &'static str,
    /// The scenario's settings for this tool, as the report writes them.
    pub(crate) mapping: &'static str,
    /// Values the scenario asks for that the tool runs without yet, each
    /// with the nearest value it runs instead and the ticket that brings it.
    pub(crate) pending: &'static [&'static str],
    pub(crate) status: Status,
}

/// Every cell, in the report's order.
pub(crate) const CELLS: [Cell; 9] = [
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "code-free crawl on the `http` tier, `run_state.durability: fast`, \
                  `site_preferences.respect_robots_txt: false`, `politeness.domain` delay 0 and \
                  concurrency 16 (`sharing: run`), `crawl.handler.concurrency: 16`, `formats: {}`",
        pending: &[
            "`run_state.durability: off` (#1927): runs `fast`, the fastest level there is",
            "`frontier.learning: false` (#1909): runs with learning on, the only state there is",
        ],
        status: Status::Measured(Config::Foxhound { durability: "fast" }),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Spider,
        name: "spider",
        mapping: "HTTP mode (`crawl_raw`, no `chrome` feature), `with_respect_robots_txt(false)`, \
                  `with_concurrency_limit(16)`, no delay, nothing kept on disk",
        pending: &[],
        status: Status::Measured(Config::Spider),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "`CheerioCrawler` (`@crawlee/cheerio`) with `persistStorage: false` and \
                  `maxConcurrency: 16`, its autoscaled pool starting at the HTTP crawlers' \
                  desired concurrency of 10 and growing toward 16 as the system allows (its \
                  default), robots.txt not obeyed (its default), no delay, `enqueueLinks()` on \
                  the same hostname (its default), its default logging",
        pending: &[],
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "a default `scrapy.Spider` following every link (`response.follow_all`, \
                  `allowed_domains` the site's host), `ROBOTSTXT_OBEY: False`, \
                  `CONCURRENT_REQUESTS: 16`, `CONCURRENT_REQUESTS_PER_DOMAIN: 16`, \
                  `DOWNLOAD_DELAY: 0`, `LOG_LEVEL: INFO` (Crawlee's and Foxhound's default \
                  level, where Scrapy's DEBUG logs every request), the first page through the \
                  duplicate filter like every other request, nothing kept on disk",
        pending: &[],
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Foxhound,
        name: "foxhound full",
        mapping: "scenario 1's settings with `run_state.durability: full`, its default: every \
                  outcome synced; killed with SIGKILL once half the site is served, then \
                  `run.resume` in a new process on the same home",
        pending: &[],
        status: Status::Measured(Config::Foxhound { durability: "full" }),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Foxhound,
        name: "foxhound periodic",
        mapping: "scenario 1's settings with `run_state.durability: periodic` and \
                  `run_state.commit_interval: 60s`, the like-for-like cell against Crawlee's \
                  `persistStorage` and Scrapy's `JOBDIR`",
        pending: &[],
        status: Status::Pending(
            "`run_state.durability: periodic` and `run_state.commit_interval` arrive with #1927",
        ),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Spider,
        name: "spider",
        mapping: "none: Spider keeps no crawl state it can resume from",
        pending: &[],
        status: Status::NotSupported(
            "Spider keeps no crawl state on disk that a new process resumes from after a kill (the owner's matrix, 2026-10-30)",
        ),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "none: `persistStorage: true` (its default) loses pages to a kill",
        pending: &[],
        status: Status::NotSupported(
            "Crawlee's `persistStorage` writes each request of its queue to a file of its own \
             with a plain write, so a SIGKILL can leave one half-written; the process started \
             again on the same storage, with `purgeOnStart: false`, ignores it as invalid JSON \
             and never fetches its page. On 2026-11-03 (#1902), 2 of 6 such crawls fetched 997 \
             and 999 of the 1,000 pages, which no same-work check passes; the others took 364 s, \
             waiting for the request locks the killed process held",
        ),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "none: `JOBDIR` resumes only a job stopped cleanly",
        pending: &[],
        status: Status::NotSupported(
            "Scrapy's `JOBDIR` writes its queue's index only on a clean close, so after a SIGKILL \
             the process started again on the same `JOBDIR` has no request left to fetch: it \
             fetched none of the 488 pages left on 2026-11-03 (#1902), as Scrapy's jobs \
             documentation warns; a clean stop (one SIGINT) resumes, but that is no kill",
        ),
    },
];

/// The cell `name` of `scenario`.
pub(crate) fn cell(scenario: Scenario, name: &str) -> Option<&'static Cell> {
    CELLS
        .iter()
        .find(|cell| cell.scenario == scenario && cell.name == name)
}

/// Each measured cell with its configuration.
pub(crate) fn measured() -> impl Iterator<Item = (&'static Cell, Config)> {
    CELLS.iter().filter_map(|cell| match cell.status {
        Status::Measured(config) => Some((cell, config)),
        Status::NotSupported(_) | Status::Pending(_) => None,
    })
}
