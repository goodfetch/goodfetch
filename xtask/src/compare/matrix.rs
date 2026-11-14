//! The comparison matrix (the owner's comments of 2026-10-30 on #1577, ADR
//! 0138's Foxhound cells): each scenario sets every tool to the closest
//! equivalent configuration, and each cell writes that mapping. A tool that
//! cannot do what a scenario asks has a cell saying so, and a Foxhound
//! value that does not exist yet is marked pending with the ticket that
//! brings it; neither gives a figure. A tool's main row carries the figures
//! it is compared on; a sensitivity row beside it, labelled, runs the tool
//! one setting apart, to show how far its figures move with that setting
//! (#2213: Spider's). Each scenario crawls a site of the
//! Proving ground of [`super::trials::SITE_PAGES`] pages in the same tree:
//! `scale.test`'s, or for the polite and headless browser scenarios, the
//! multi-domain and script-rendered sites the Proving ground serves for
//! them (#1903). #2316's four scenarios run scenario 1's settings where a
//! lead or a gap is decided: on pages of realistic size, on a site of
//! [`LONG_PAGES`] pages, and at 64 and 256 requests in flight. They carry
//! no bar until the owner sets one (#2252 Q11).
//!
//! Size: one concern, the matrix: its scenarios, its tools and the table of
//! every cell with its mapping (`CELLS`), which is most of its length.

use serde::{Deserialize, Serialize};

use super::hostile::{LIMITS_IN_FLIGHT, LIMITS_RATE, TRAPS, Task};

/// A tool compared.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Tool {
    /// Foxhound's executable, a code-free crawl on its `http` tier.
    Foxhound,
    /// Spider, in its HTTP mode, or with its `chrome` feature.
    Spider,
    /// Crawlee's `CheerioCrawler`, or its `PlaywrightCrawler`, on Node.js.
    Crawlee,
    /// Scrapy's default HTTP crawl, or `scrapy-playwright`'s, on `CPython`.
    Scrapy,
    /// crawlee-rs's `HtmlCrawler`, Apify's Rust port of Crawlee (#2214).
    #[serde(rename = "crawlee-rs")]
    #[value(name = "crawlee-rs")]
    CrawleeRs,
    /// pagesift's `Scraper`, its breadth-first crawl (#2215).
    Pagesift,
}

impl Tool {
    pub(crate) const ALL: [Self; 6] = [
        Self::Foxhound,
        Self::Spider,
        Self::Crawlee,
        Self::Scrapy,
        Self::CrawleeRs,
        Self::Pagesift,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Foxhound => "foxhound",
            Self::Spider => "spider",
            Self::Crawlee => "crawlee",
            Self::Scrapy => "scrapy",
            Self::CrawleeRs => "crawlee-rs",
            Self::Pagesift => "pagesift",
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
    /// Scenario 3, polite: robots.txt obeyed, one request in flight and a
    /// second between request starts on each domain, on the multi-domain
    /// site, so throughput is not just the delay.
    Polite,
    /// Scenario 4, extraction to a dataset: scenario 1 with each page's
    /// title and main text written to the tool's dataset or output.
    Extraction,
    /// Scenario 5, headless browser: the script-rendered site, whose links
    /// only a browser sees.
    Browser,
    /// Scenario 1 on the realistic-page site (#2316): pages of tens of
    /// kilobytes with 30 to 100 links each, some repeated and some through
    /// a redirect.
    Realistic,
    /// Scenario 1 on a site of [`LONG_PAGES`] pages (#2316), long enough to
    /// reach the steady rate.
    Long,
    /// Scenario 1 at 64 requests in flight (#2316's sweep).
    #[serde(rename = "raw-64")]
    #[value(name = "raw-64")]
    Raw64,
    /// Scenario 1 at 256 requests in flight (#2316's sweep).
    #[serde(rename = "raw-256")]
    #[value(name = "raw-256")]
    Raw256,
    /// The hostile pages at each tool's own defaults (#2439): the `traps`
    /// family, `limits/1` and `limits/3`, each crawl counted for the rules
    /// it broke, not timed.
    Hostile,
    /// The hostile pages' `traps` family at scenario 1's settings, 16 in
    /// flight and no delay, so each tool's waste is read at one pace
    /// (#2439).
    #[serde(rename = "hostile-16")]
    #[value(name = "hostile-16")]
    Hostile16,
}

/// The pages of the long scenario's site.
pub(crate) const LONG_PAGES: u64 = 100_000;

/// The Proving ground site a scenario's trials crawl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    /// `scale.test/sites/<n>/`, reached on the Proving ground's address
    /// with its host name sent as each request's `Host`.
    Scale,
    /// The multi-domain site: page `n` on `polite-<d>.localhost`, `d`
    /// being `n` modulo [`DOMAINS`], from 1.
    MultiDomain,
    /// The script-rendered site on `rendered.localhost`.
    Rendered,
    /// The realistic-page site on `realistic.localhost`.
    Realistic,
    /// The hostile pages: each task's host, reached on the Proving
    /// ground's address with its host name sent as each request's `Host`.
    Hostile,
}

/// The multi-domain site's domains, as the Proving ground serves them.
pub(crate) const DOMAINS: u64 = 50;

/// The host name of `scale.test`'s sites.
pub(crate) const SCALE_HOST: &str = crate::rss::crawl::HOST;

impl Site {
    /// The host page `page` of the site lives on.
    pub(crate) fn host_of(self, page: u64) -> String {
        match self {
            Self::Scale => SCALE_HOST.to_owned(),
            Self::MultiDomain => format!("polite-{}.localhost", (page - 1) % DOMAINS + 1),
            Self::Rendered => "rendered.localhost".to_owned(),
            Self::Realistic => "realistic.localhost".to_owned(),
            Self::Hostile => super::hostile::TRAPS.host.to_owned(),
        }
    }

    /// Every host of the site.
    pub(crate) fn hosts(self) -> Vec<String> {
        match self {
            Self::Scale | Self::Rendered | Self::Realistic => vec![self.host_of(1)],
            Self::MultiDomain => (1..=DOMAINS).map(|page| self.host_of(page)).collect(),
            Self::Hostile => Scenario::Hostile
                .hostile_tasks()
                .iter()
                .map(|task| task.host.to_owned())
                .collect(),
        }
    }

    /// Whether a tool reaches the site by its host names, which the system
    /// and a browser both resolve to loopback (`*.localhost`), rather than
    /// on the Proving ground's address with a `Host` header.
    pub(crate) fn by_name(self) -> bool {
        !matches!(self, Self::Scale | Self::Hostile)
    }
}

impl Scenario {
    pub(crate) const ALL: [Self; 11] = [
        Self::Raw,
        Self::Durable,
        Self::Polite,
        Self::Extraction,
        Self::Browser,
        Self::Realistic,
        Self::Long,
        Self::Raw64,
        Self::Raw256,
        Self::Hostile,
        Self::Hostile16,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Durable => "durable",
            Self::Polite => "polite",
            Self::Extraction => "extraction",
            Self::Browser => "browser",
            Self::Realistic => "realistic",
            Self::Long => "long",
            Self::Raw64 => "raw-64",
            Self::Raw256 => "raw-256",
            Self::Hostile => "hostile",
            Self::Hostile16 => "hostile-16",
        }
    }

    /// The scenario whose settings each tool runs: scenario 1's for
    /// #2316's, which change only the site or the requests in flight, and
    /// for the hostile pages' `traps` pass, so each tool's side takes them
    /// as `raw`.
    pub(crate) fn settings(self) -> Self {
        match self {
            Self::Realistic | Self::Long | Self::Raw64 | Self::Raw256 | Self::Hostile16 => {
                Self::Raw
            }
            Self::Raw
            | Self::Durable
            | Self::Polite
            | Self::Extraction
            | Self::Browser
            | Self::Hostile => self,
        }
    }

    /// Whether its cells carry no bar until the owner sets one: #2316's
    /// (#2252 Q11) and the hostile pages' (#2439, Q3).
    pub(crate) fn carries_no_bar(self) -> bool {
        self.settings() != self || self.hostile()
    }

    /// The Proving ground's tasks its trials crawl, each counted for the
    /// rules the tool broke (#2439): none for a timed scenario.
    pub(crate) fn hostile_tasks(self) -> &'static [Task] {
        match self {
            Self::Hostile => &[TRAPS, LIMITS_RATE, LIMITS_IN_FLIGHT],
            // `limits/1` would flag any tool at 16 in flight and no delay.
            Self::Hostile16 => &[TRAPS],
            Self::Raw
            | Self::Durable
            | Self::Polite
            | Self::Extraction
            | Self::Browser
            | Self::Realistic
            | Self::Long
            | Self::Raw64
            | Self::Raw256 => &[],
        }
    }

    /// Whether its trials count the rules each tool broke instead of
    /// timing a crawl: the hostile pages'.
    pub(crate) fn hostile(self) -> bool {
        !self.hostile_tasks().is_empty()
    }

    /// The requests each tool keeps in flight: the resident-memory check's
    /// crawl's, as [`super::trials::CONCURRENCY`] is, but in the sweep.
    pub(crate) fn concurrency(self) -> u32 {
        match self {
            Self::Raw64 => 64,
            Self::Raw256 => 256,
            Self::Raw
            | Self::Durable
            | Self::Polite
            | Self::Extraction
            | Self::Browser
            | Self::Realistic
            | Self::Long
            | Self::Hostile
            | Self::Hostile16 => crate::rss::crawl::CONCURRENCY,
        }
    }

    /// The pages of the site its trials crawl, `declared` being the run's.
    pub(crate) fn site_pages(self, declared: u64) -> u64 {
        if self == Self::Long {
            LONG_PAGES
        } else {
            declared
        }
    }

    /// Whether only its timed trials run: the long scenario's site is
    /// already the growth cell's length, and strace over it would take
    /// hours.
    pub(crate) fn timed_only(self) -> bool {
        self == Self::Long
    }

    /// The site its trials crawl.
    pub(crate) fn site(self) -> Site {
        match self {
            Self::Raw
            | Self::Durable
            | Self::Extraction
            | Self::Long
            | Self::Raw64
            | Self::Raw256 => Site::Scale,
            Self::Polite => Site::MultiDomain,
            Self::Browser => Site::Rendered,
            Self::Realistic => Site::Realistic,
            Self::Hostile | Self::Hostile16 => Site::Hostile,
        }
    }

    /// Whether the site's links past its first page are added only by a
    /// script, so the same-work check proves each tool rendered them.
    pub(crate) fn renders_links(self) -> bool {
        self.site() == Site::Rendered
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Raw => {
                "1. Raw fetch and follow: no persistence, no robots.txt, no delay, 16 in flight"
            }
            Self::Durable => {
                "2. Durable and resumable: state on disk, killed once half the site is served, then resumed"
            }
            Self::Polite => {
                "3. Polite: robots.txt obeyed, 1 in flight per domain, 1 s delay, on the multi-domain site (50 domains)"
            }
            Self::Extraction => {
                "4. Extraction to a dataset: title and main text per page, written to the tool's dataset or output"
            }
            Self::Browser => {
                "5. Headless browser: the script-rendered site, whose links an HTTP-only crawl never finds"
            }
            Self::Realistic => {
                "Realistic pages: scenario 1 on pages of tens of kilobytes with 30 to 100 links each, some repeated and some through a redirect"
            }
            Self::Long => "Long site: scenario 1 on a site of 100,000 pages",
            Self::Raw64 => "Concurrency sweep: scenario 1 at 64 in flight",
            Self::Raw256 => "Concurrency sweep: scenario 1 at 256 in flight",
            Self::Hostile => {
                "Hostile pages at each tool's defaults: the rules it broke on the `traps` family, `limits/1` and `limits/3`"
            }
            Self::Hostile16 => {
                "Hostile pages at one pace: the rules each tool broke on the `traps` family at scenario 1's settings, 16 in flight and no delay"
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

    /// Whether its cells write content that no check reads, so the report
    /// gives them no standing: the extraction scenario's, until #2136's
    /// output check lands.
    pub(crate) fn content_unverified(self) -> bool {
        matches!(self, Self::Extraction)
    }
}

/// The capacity of the broadcast channel Spider's rows read its pages
/// from. Spider's `Website` keeps a receiver that never reads, and Tokio
/// frees a slot only once every receiver has read it, so the channel keeps
/// that many whole pages alive: a capacity larger than the crawl needs is
/// memory the harness adds to Spider's. Tokio rounds a capacity up to a
/// power of two, so the powers of two were tried.
///
/// The smallest at which a 10,000-page crawl lost no page (#2213), on the
/// gate host on 2026-11-11 at a 1-minute load of 7 to 11: of 40 crawls of
/// scenario 1 at each capacity, 5 lost pages at 32 and none at 64 or 128.
/// The figure holds for a machine with cores to spare, which is how the
/// comparison runs: at a load of 17 to 20 on its 16 cores, 2 of 21 crawls
/// at 64 lost pages, and a crawl that loses one fails its trial.
pub(crate) const SPIDER_CAPACITY: usize = 64;

/// The same for the extraction scenario, whose reader also writes each
/// page: of 40 crawls at each capacity, 3 lost pages at 32, 2 at 64, 1 at
/// 128 and none at 256, 512 or 1,024.
pub(crate) const SPIDER_EXTRACTION_CAPACITY: usize = 256;

/// The capacity Spider's rows had before #2213, kept as a sensitivity row.
pub(crate) const SPIDER_CAPACITY_BEFORE: usize = 4096;

/// Spider's capacity at 64 requests in flight (#2316): [`SPIDER_CAPACITY`]
/// scaled with the requests in flight, four times scenario 1's, not
/// measured apart; a trial that loses a page fails.
pub(crate) const SPIDER_CAPACITY_64: usize = 256;

/// The same at 256 requests in flight.
pub(crate) const SPIDER_CAPACITY_256: usize = 1024;

/// A build of Spider's side, `xtask/compare-spider`: `just compare` builds
/// each apart, with its Cargo feature or none, into a folder of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpiderBuild {
    /// Spider's `basic` and `sync` features, default features off, on the
    /// system's allocator: its HTTP mode.
    Http,
    /// With Spider's `chrome` feature: each page rendered in Chrome.
    Chrome,
    /// With Spider's default features, `io_uring` among them, on top of
    /// [`Self::Http`]'s.
    Defaults,
    /// With Spider's `bloom` feature.
    Bloom,
    /// On `tikv-jemallocator`, the allocator Spider's README recommends
    /// for a pure crawl.
    Jemalloc,
    /// On `mimalloc`, the allocator Spider's README recommends for a crawl
    /// that also parses, and the one Foxhound's executable sets.
    Mimalloc,
}

impl SpiderBuild {
    /// The feature of `xtask/compare-spider` the build turns on, if any.
    pub(crate) fn feature(self) -> Option<&'static str> {
        match self {
            Self::Http => None,
            Self::Chrome => Some("chrome"),
            Self::Defaults => Some("defaults"),
            Self::Bloom => Some("bloom"),
            Self::Jemalloc => Some("jemalloc"),
            Self::Mimalloc => Some("mimalloc"),
        }
    }

    /// The build's folder among Spider's side's builds.
    pub(crate) fn folder(self) -> String {
        self.feature().map_or_else(
            || "spider".to_owned(),
            |feature| format!("spider-{feature}"),
        )
    }
}

/// The folder of crawlee-rs's side's build among the Rust sides' builds,
/// beside Spider's (`just compare`).
pub(crate) const CRAWLEE_RS_FOLDER: &str = "crawlee-rs";

/// The folder of pagesift's side's build among the Rust sides' builds.
pub(crate) const PAGESIFT_FOLDER: &str = "pagesift";

/// What a cell's row is read for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Row {
    /// The tool's figures: what a target, a bar and the instruction ratio
    /// read.
    Main,
    /// A variant of the tool's main row of the scenario, one setting
    /// apart, which shows how much the tool's figures move with it; no
    /// target, bar or ratio reads it.
    Sensitivity,
}

/// How a measured cell runs its tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Config {
    /// Foxhound's code-free crawl on its `http` tier at this
    /// `run_state.durability`, with this `run_state.commit_interval` in
    /// milliseconds at `periodic`, robots.txt not obeyed,
    /// `politeness.domain` delay 0 and concurrency 16, no format.
    Foxhound {
        durability: &'static str,
        commit_interval: Option<u64>,
    },
    /// Spider's side at the scenario's settings: this build of it,
    /// subscribed to Spider's pages with a broadcast channel of this
    /// capacity.
    Spider { build: SpiderBuild, capacity: usize },
    /// Crawlee's side at the scenario's settings: its `CheerioCrawler`, or
    /// its `PlaywrightCrawler` in the headless browser scenario.
    Crawlee,
    /// Scrapy's side at the scenario's settings: its default crawl, or
    /// `scrapy-playwright`'s in the headless browser scenario.
    Scrapy,
    /// crawlee-rs's side at the scenario's settings: its `HtmlCrawler`.
    CrawleeRs,
    /// pagesift's side at the scenario's settings: its `Scraper`'s crawl.
    Pagesift,
}

impl Config {
    /// The tool this configuration runs.
    pub(crate) fn tool(self) -> Tool {
        match self {
            Self::Foxhound { .. } => Tool::Foxhound,
            Self::Spider { .. } => Tool::Spider,
            Self::Crawlee => Tool::Crawlee,
            Self::Scrapy => Tool::Scrapy,
            Self::CrawleeRs => Tool::CrawleeRs,
            Self::Pagesift => Tool::Pagesift,
        }
    }
}

/// What a cell gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Measured(Config),
    /// The tool cannot do what the scenario asks.
    NotSupported(&'static str),
    /// The cell stands in the matrix before the ticket that brings it, as
    /// this names it, has landed: it runs no trial.
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
    pub(crate) row: Row,
    pub(crate) status: Status,
}

/// Every cell, in the report's order.
pub(crate) const CELLS: [Cell; 76] = [
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "the maximum-throughput combination (`docs/throughput.md`): code-free crawl on \
                  the `http` tier, `run_state.durability: off`, \
                  `site_preferences.respect_robots_txt: false`, `politeness.domain` delay 0 and \
                  concurrency 16 (`sharing: run`), `crawl.handler.concurrency: 16`, `formats: {}`, \
                  `frontier.learning: false`, `provenance.log: false`, `dataset.records: false` \
                  and `fetch.block_classification: false`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Spider,
        name: "spider",
        mapping: "HTTP mode (`crawl_raw`, no `chrome` feature), `with_respect_robots_txt(false)`, \
                  `with_concurrency_limit(16)`, no delay, nothing kept on disk, on the system's \
                  allocator; each page read as it arrives from `subscribe(64)`, the smallest \
                  capacity at which a 10,000-page crawl lost no page (#2213), since Spider \
                  keeps that many pages alive in the channel",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY,
        }),
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
        row: Row::Main,
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
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    defaults(Scenario::Raw),
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "`HtmlCrawler`, crawlee-rs's `CheerioCrawler`, on its `reqwest` client, with \
                  `persist_storage: false` and `max_concurrency(16)`, its autoscaled pool \
                  starting at its desired concurrency of 10 and growing toward 16 as the system \
                  allows (its default), robots.txt not obeyed (its default), no delay, \
                  `enqueue_links` on the same hostname (its default), with no log subscriber \
                  (a library, it writes no log of its own), on the system's allocator",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "its library's `Scraper::crawl`, as its own crawl benchmark drives it: breadth-first \
                  from the first page over its whole host (`crawlEntireDomain`), on its HTTP engine \
                  with each page's links its only format, a limit of 100,000 pages (its API's \
                  maximum) and its sitemap not read (`sitemap: skip`); `maxConcurrency: 16`, its \
                  limiter starting at 16 in flight and capped there (`global_concurrency`, \
                  `global_ceiling`), its controller moving under the cap; `politeness` off, its \
                  administrator's switch, so no interval and no cap per site; robots.txt not obeyed \
                  (`ignoreRobotsTxt`, `respect_robots: false`); nothing kept on disk; on its own \
                  allocator (`SmallCache`, tuned at start as its executable does)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Spider,
        name: "spider capacity 4096",
        mapping: "the spider row's settings with its pages read from `subscribe(4096)`, the \
                  capacity every Spider row had before #2213: up to 4,096 pages stay alive in \
                  the channel",
        pending: &[],
        row: Row::Sensitivity,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY_BEFORE,
        }),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Spider,
        name: "spider default features",
        mapping: "the spider row's settings (`subscribe(64)`), built with Spider's default \
                  features on top of the row's own: `io_uring`, `tcp_fastopen`, `splice`, `numa` \
                  and `zero_copy`, as `cargo add spider` builds it",
        pending: &[],
        row: Row::Sensitivity,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Defaults,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Spider,
        name: "spider bloom",
        mapping: "the spider row's settings (`subscribe(64)`), built with Spider's `bloom` \
                  feature: its set of visited links is a Bloom filter",
        pending: &[],
        row: Row::Sensitivity,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Bloom,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Spider,
        name: "spider jemalloc",
        mapping: "the spider row's settings (`subscribe(64)`), with `tikv-jemallocator` as its \
                  global allocator, which Spider's README recommends for a pure crawl; the \
                  spider row runs on the system's allocator and Foxhound on mimalloc",
        pending: &[],
        row: Row::Sensitivity,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Jemalloc,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Foxhound,
        name: "foxhound full",
        mapping: "scenario 1's settings with `run_state.durability: full`, its default: every \
                  outcome synced, and `dataset.records` on, as its resume is judged by them; killed with SIGKILL once half the site is served, then \
                  `run.resume` in a new process on the same home",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "full",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Foxhound,
        name: "foxhound periodic",
        mapping: "scenario 1's settings with `run_state.durability: periodic` and \
                  `run_state.commit_interval: 60s`, the like-for-like cell against Crawlee's \
                  `persistStorage` and Scrapy's `JOBDIR`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "periodic",
            commit_interval: Some(60_000),
        }),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Spider,
        name: "spider",
        mapping: "none: Spider keeps no crawl state it can resume from",
        pending: &[],
        row: Row::Main,
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
        row: Row::Main,
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
        row: Row::Main,
        status: Status::NotSupported(
            "Scrapy's `JOBDIR` writes its queue's index only on a clean close, so after a SIGKILL \
             the process started again on the same `JOBDIR` has no request left to fetch: it \
             fetched none of the 488 pages left on 2026-11-03 (#1902), as Scrapy's jobs \
             documentation warns; a clean stop (one SIGINT) resumes, but that is no kill",
        ),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler` with its storage on disk (`persist_storage: true`, \
                  its default) and `purge_on_start: false`, so a process started again on the \
                  same storage takes up the request queue the killed one left; killed with \
                  SIGKILL once half the site is served, then the same command in a new process \
                  on the same folder",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl as a job of its HTTP API (`POST /v1/crawl`), called in process, \
                  with its store on disk in the trial's folder, which keeps the job, its frontier \
                  and its pages; killed with SIGKILL once half the site is served, then the same \
                  command in a new process on the same folder, where opening the store resumes the \
                  job",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "its defaults (ADR 0086): a code-free crawl with robots.txt obeyed \
                  (`site_preferences.respect_robots_txt: true`), `politeness.sharing: shared`, \
                  `crawl.handler.concurrency: 16`, `run_state.durability: full`, `formats: {}`; \
                  with `politeness.domain.delay: 1s` and `politeness.domain.concurrency: 1` \
                  set, ADR 0086's values, which the catalog holds no default for until the \
                  fetch tiers take them (its #684 note); `fetch.tier: http`, as the other tools \
                  fetch, where `auto` would also fetch each domain's first pages in a browser; \
                  and `crawl.scope.strategy: all`, since the site spans 50 domains and links \
                  nothing else",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "full",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::Spider,
        name: "spider",
        mapping: "none: Spider's delay and robots.txt are its crawl's, not each domain's",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(SPIDER_NOT_POLITE),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "`CheerioCrawler` with `sameDomainDelaySecs: 1`, `respectRobotsTxtFile: true` \
                  and `maxConcurrency: 16`; Crawlee has no limit of requests in flight per \
                  domain, so its same-domain delay, one request start a second on each domain \
                  (by `tldts`' registrable domain), is what holds each domain to one in flight \
                  while an answer takes under a second; `enqueueLinks({ strategy: 'all' })`, \
                  since the site spans 50 domains and links nothing else; \
                  `persistStorage: false`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "a default `scrapy.Spider` following every link (no `allowed_domains`, since the \
                  site spans 50 domains and links nothing else), `ROBOTSTXT_OBEY: True`, \
                  `DOWNLOAD_DELAY: 1` with `RANDOMIZE_DOWNLOAD_DELAY: False` (its default \
                  draws each wait between 0.5 and 1.5 s, where the others wait 1 s), \
                  `CONCURRENT_REQUESTS_PER_DOMAIN: 1`, `CONCURRENT_REQUESTS: 16`, \
                  `LOG_LEVEL: INFO`, nothing kept on disk",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "`HtmlCrawler` with `respect_robots_txt(true)`, `same_domain_delay(1 s)` and \
                  `max_concurrency(16)`; crawlee-rs has no limit of requests in flight per \
                  domain, so its same-domain delay, one request start a second on each domain \
                  (its registrable domain, else its host name), is what holds each domain to \
                  one in flight while an answer takes under a second; `enqueue_links` with \
                  `EnqueueStrategy::All`, since the site spans 50 domains and links nothing \
                  else; `persist_storage: false`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "`Scraper::crawl` with robots.txt obeyed (its default) and `politeness` on (its \
                  default): its default 1 s between the starts of a site's pages, its \
                  robots.txt fetched outside that interval, just before the first, and \
                  `per_host_concurrency` 1 where its default is 2; 16 in flight in all \
                  (`maxConcurrency`, `global_ceiling`); every link followed (`allowExternalLinks`), \
                  since the site spans 50 domains and links nothing else; nothing kept on disk",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings with `formats: { text: true }`, the text of each page's \
                  main content, and the field `title` (the CSS selector `title`), with \
                  `dataset.records` on, each page's Record committed to the code-free crawl's \
                  Dataset `pages` in its run file",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings, each page's URL, title (Spider's page metadata) and HTML \
                  written as one JSON line to `pages.jsonl`: Spider reads no main content, so \
                  its page content is what it writes; its pages read from `subscribe(256)`, the \
                  smallest capacity at which this scenario's 10,000-page crawl lost no page",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_EXTRACTION_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "scenario 1's `CheerioCrawler`, each page's title (`$('title')`) and main text \
                  (the text of its `main`, as Crawlee reads no main content of its own) given \
                  to `pushData`, its default Dataset kept on disk (`persistStorage: true`, its \
                  default, which keeps its request queue on disk too)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "scenario 1's settings, each page's title and main text (the text of its \
                  `main`, as Scrapy reads no main content of its own) yielded as an Item, which \
                  an Item pipeline writes as one JSON line to `items.jsonl`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler`, each page's URL, title (`Document::title`) and main \
                  text (the text of its `main`, as crawlee-rs reads no main content of its own) \
                  given to `push_data`, its default Dataset kept on disk (`persist_storage: \
                  true`, its default, which keeps its request queue on disk too)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl with each page's main content as Markdown (`formats: \
                  [markdown]` with `onlyMainContent`, its default), each page's URL, title and \
                  Markdown written as a line of `pages.jsonl`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Spider,
        name: "spider mimalloc",
        mapping: "the spider row's settings (`subscribe(256)`), with `mimalloc` as its global \
                  allocator, which Spider's README recommends for a crawl that also parses, and \
                  the allocator Foxhound's executable sets; the spider row runs on the system's",
        pending: &[],
        row: Row::Sensitivity,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Mimalloc,
            capacity: SPIDER_EXTRACTION_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings on the `browser` tier (`fetch.tier: browser`), headless \
                  Chrome for Testing, its pinned build, from a Browser cache laid out beside \
                  the data folder (`GOODFETCH_BROWSERS_PATH`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Spider,
        name: "spider",
        mapping: "Spider built with its `chrome` feature, `crawl()` rendering each page in the \
                  same headless Chrome for Testing (`CHROME_BIN`), with its own launch \
                  arguments, `with_respect_robots_txt(false)`, `with_concurrency_limit(16)`, \
                  nothing kept on disk, its pages read from `subscribe(64)`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Chrome,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "`PlaywrightCrawler` (`@crawlee/playwright`) in the same headless Chrome for \
                  Testing (`launchOptions.executablePath`), `maxConcurrency: 16`, robots.txt \
                  not obeyed (its default), `enqueueLinks()` once the page loaded, \
                  `persistStorage: false`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "`scrapy-playwright`'s download handler for every request, in the same \
                  headless Chrome for Testing (`PLAYWRIGHT_LAUNCH_OPTIONS`' `executable_path`), \
                  `CONCURRENT_REQUESTS` and `CONCURRENT_REQUESTS_PER_DOMAIN` 16, \
                  `ROBOTSTXT_OBEY: False`, `LOG_LEVEL: INFO`, nothing kept on disk",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "none: crawlee-rs crawls over HTTP only",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(CRAWLEE_RS_NO_BROWSER),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl on its browser engine (`engine: browser`), each page rendered \
                  in the same headless Chrome for Testing (`BrowserSource::Executable`), its pool \
                  of browsers and tabs at its defaults",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Spider,
        name: "spider smart",
        mapping: "none: Spider's `smart` feature, `crawl_smart()`, which fetches each page over \
                  HTTP first and renders it in Chrome only where Spider finds it needs a browser",
        pending: &[],
        row: Row::Sensitivity,
        status: Status::NotSupported(SPIDER_NOT_SMART),
    },
    Cell {
        scenario: Scenario::Realistic,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings (its maximum-throughput combination, 16 in flight) on the \
                  realistic-page site, `realistic.localhost`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Realistic,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings (`with_concurrency_limit(16)`, `subscribe(64)`) on the \
                  realistic-page site; its redirects followed, as Spider follows them by default",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Realistic,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "scenario 1's `CheerioCrawler` (`maxConcurrency: 16`) on the realistic-page site, \
                  its redirects followed, as Crawlee follows them by default",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Realistic,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "scenario 1's `scrapy.Spider` (`CONCURRENT_REQUESTS` and \
                  `CONCURRENT_REQUESTS_PER_DOMAIN` 16) on the realistic-page site, `allowed_domains` \
                  its host, its redirects followed by its default `RedirectMiddleware`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Realistic,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler` (`max_concurrency(16)`, `persist_storage: false`) on the \
                  realistic-page site, its redirects followed, as its `reqwest` client follows \
                  them by default",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Realistic,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl (`maxConcurrency: 16`, `politeness` off) on the realistic-page \
                  site, its redirects followed (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Long,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings (its maximum-throughput combination, 16 in flight) on \
                  `scale.test`'s site of 100,000 pages",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Long,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings (`with_concurrency_limit(16)`) on the site of 100,000 \
                  pages, its pages read from `subscribe(64)`, the capacity at which a 10,000-page \
                  crawl lost no page (#2213); a trial that loses a page fails",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Long,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "scenario 1's `CheerioCrawler` (`maxConcurrency: 16`, `persistStorage: false`) on the \
                  site of 100,000 pages",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Long,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "scenario 1's `scrapy.Spider` (`CONCURRENT_REQUESTS` and \
                  `CONCURRENT_REQUESTS_PER_DOMAIN` 16) on the site of 100,000 pages",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Long,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler` (`max_concurrency(16)`, `persist_storage: false`) on the \
                  site of 100,000 pages",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Long,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl (`maxConcurrency: 16`, `politeness` off) on the site of \
                  100,000 pages, the most its limit lets a crawl take",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings with `politeness.domain` concurrency 64 and \
                  `crawl.handler.concurrency: 64`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings with `with_concurrency_limit(64)`, its pages read from \
                  `subscribe(256)`: scenario 1's capacity of 64 scaled with the requests in \
                  flight, not measured at 64, so a trial that loses a page fails",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY_64,
        }),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "scenario 1's `CheerioCrawler` with `maxConcurrency: 64`, its autoscaled pool \
                  growing toward 64 as the system allows (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "scenario 1's `scrapy.Spider` with `CONCURRENT_REQUESTS: 64` and \
                  `CONCURRENT_REQUESTS_PER_DOMAIN: 64`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler` with `max_concurrency(64)`, its autoscaled pool growing \
                  toward 64 as the system allows (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl with `maxConcurrency: 64`, its limiter starting at 64 in \
                  flight and capped there",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    defaults(Scenario::Raw64),
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings with `politeness.domain` concurrency 256 and \
                  `crawl.handler.concurrency: 256`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings with `with_concurrency_limit(256)`, its pages read from \
                  `subscribe(1024)`: scenario 1's capacity of 64 scaled with the requests in \
                  flight, not measured at 256, so a trial that loses a page fails",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY_256,
        }),
    },
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "scenario 1's `CheerioCrawler` with `maxConcurrency: 256`, its autoscaled pool \
                  growing toward 256 as the system allows (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "scenario 1's `scrapy.Spider` with `CONCURRENT_REQUESTS: 256` and \
                  `CONCURRENT_REQUESTS_PER_DOMAIN: 256`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler` with `max_concurrency(256)`, its autoscaled pool growing \
                  toward 256 as the system allows (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl with `maxConcurrency: 256`, its limiter starting at 256 in \
                  flight and capped there",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    defaults(Scenario::Raw256),
    Cell {
        scenario: Scenario::Hostile,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "the catalog's defaults, setting nothing but each task's host resolved to the \
                  Proving ground (`network.resolve`, `egress.private_ranges`): `fetch.tier: auto` \
                  with a Browser cache laid out for it (`GOODFETCH_BROWSERS_PATH`), robots.txt \
                  obeyed, `politeness.sharing: shared`, `run_state.durability: full`, and no \
                  delay or cap per domain until #1499 puts ADR 0086's pace in the catalog; one \
                  crawl per task from its start page, `formats: {}` and `follow: scope`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "full",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Hostile,
        tool: Tool::Spider,
        name: "spider",
        mapping: "Spider's defaults in its HTTP mode (`crawl_raw`, no `chrome` feature), setting \
                  nothing but the start URL and the `Host` it sends: its own limit of requests \
                  in flight (no `with_concurrency_limit`), robots.txt not obeyed (its default), \
                  no delay; its pages read from `subscribe(64)`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Hostile,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "`CheerioCrawler` at Crawlee's defaults, setting nothing but the `Host` it sends: \
                  `maxConcurrency` 200, robots.txt not obeyed, no delay, its storage on disk \
                  (`persistStorage: true`), `enqueueLinks()` on the same hostname",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Hostile,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "a default `scrapy.Spider` following every link on the address it starts from \
                  (`allowed_domains`; the task's links are all relative), setting nothing but the `Host` it sends and `LOG_LEVEL: INFO`, which \
                  changes only what it logs: `ROBOTSTXT_OBEY: False`, `CONCURRENT_REQUESTS: 16`, \
                  `CONCURRENT_REQUESTS_PER_DOMAIN: 8`, `DOWNLOAD_DELAY: 0`, `429` and `503` \
                  retried twice (its defaults)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Hostile,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "`HtmlCrawler` at crawlee-rs's defaults (`Configuration::default()`), setting \
                  nothing but the `Host` it sends: `max_concurrency` 200, robots.txt not \
                  obeyed, no delay, its storage on disk (`persist_storage: true`), \
                  `enqueue_links` on the same hostname",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Hostile,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "`Scraper::crawl` at pagesift's defaults (`ScraperConfig::default()` and a crawl \
                  request of the first page alone), setting nothing but what reaches the Proving \
                  ground (its resolver answering the task's host name with the loopback address, \
                  its private-network guard off): robots.txt obeyed and its sitemap read, \
                  `politeness` on at 1 s between request starts and 2 in flight on a site, 64 in \
                  flight in all to start with, a limit of 10,000 pages, each page's Markdown, its \
                  `auto` engine (HTTP or a browser as it judges each page, the browser one it looks \
                  for itself) and its `auto` scope, the first page's folder",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings (its maximum-throughput combination, 16 in flight, no \
                  delay, robots.txt not obeyed) on the `traps` family",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
        }),
    },
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings (`with_concurrency_limit(16)`, \
                  `with_respect_robots_txt(false)`, no delay, `subscribe(64)`) on the `traps` family",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "scenario 1's `CheerioCrawler` (`maxConcurrency: 16`, `persistStorage: false`, \
                  robots.txt not obeyed, no delay) on the `traps` family",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "scenario 1's `scrapy.Spider` (`CONCURRENT_REQUESTS` and \
                  `CONCURRENT_REQUESTS_PER_DOMAIN` 16, `DOWNLOAD_DELAY: 0`, \
                  `ROBOTSTXT_OBEY: False`) on the `traps` family",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler` (`max_concurrency(16)`, `persist_storage: false`, \
                  robots.txt not obeyed, no delay) on the `traps` family",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl (`maxConcurrency: 16`, `politeness` off, robots.txt not \
                  obeyed, no delay) on the `traps` family",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
];

/// The cell at the catalog's defaults (#2175) in `scenario`, one of
/// scenario 1's concurrency sweep: pending until #2175 brings it.
const fn defaults(scenario: Scenario) -> Cell {
    Cell {
        scenario,
        tool: Tool::Foxhound,
        name: "foxhound defaults",
        mapping: "Foxhound at the catalog's defaults, setting nothing but the crawl's name and the \
                  scenario's pace, `politeness.domain.delay: 0` and `politeness.domain.concurrency` \
                  at its requests in flight",
        pending: &[],
        row: Row::Main,
        status: Status::Pending(DEFAULTS_PENDING),
    }
}

/// Why the cell at the catalog's defaults gives no figure yet.
const DEFAULTS_PENDING: &str = "#2175, which brings the cell at the catalog's defaults";

/// Why Spider's `smart` row has no figures: what its crawl of the
/// script-rendered site fetched (#2213).
const SPIDER_NOT_SMART: &str = "Spider's `crawl_smart()` fetched 1 of the site's 100 pages, which no same-work check \
     passes: it renders a page only when its signs that the page needs a browser add up to its \
     threshold, the feed page's did not, and the feed's items are linked by its script alone. \
     On 2026-11-11 (#2213), 2 of 2 such crawls of Spider 2.53.9, built with its `smart` \
     feature beside the spider row's settings, ended so, with no Chrome started";

/// Why crawlee-rs has no headless browser cell.
const CRAWLEE_RS_NO_BROWSER: &str = "crawlee-rs has no browser crawler at its pinned commit: \
     its README plans browser crawlers under \"What's not there yet\", and its \
     `HtmlCrawler` fetches over HTTP only, so it never sees the links the script-rendered site's \
     script adds";

/// Why Spider has no polite cell: what a crawl of the multi-domain site
/// with its delay and robots.txt showed.
const SPIDER_NOT_POLITE: &str = "Spider's `with_delay` waits between any two requests of \
     its crawl, whatever their domain, and it reads only its start URL's robots.txt. On 2026-11-03 \
     (#1903), a crawl of the multi-domain site's 100-page version with `with_delay(1000)`, \
     `with_respect_robots_txt(true)` and every domain allowed (`with_external_domains`) sent its \
     102 requests over 101.5 s, one a second across all 50 domains, read robots.txt on 1 of them, \
     and fetched the page that robots.txt disallows (`Disallow: /sites/*/private`)";

/// The tools' cells declared, in a scenario, as not doing the Product's
/// work, each with why: the report gives them no standing, and the
/// instruction ratio reads scenario 1 only, where none is declared.
const UNLIKE_WORK: &[(Scenario, Tool, &str)] = &[(
    Scenario::Extraction,
    Tool::Spider,
    "it writes each page's raw HTML, where the others write the text of its main content \
     (until #2136)",
)];

impl Cell {
    /// Why the cell does not do the Product's work, when it is declared so.
    pub(crate) fn unlike_work(&self) -> Option<&'static str> {
        UNLIKE_WORK
            .iter()
            .find(|(scenario, tool, _)| *scenario == self.scenario && *tool == self.tool)
            .map(|(_, _, reason)| *reason)
    }
}

/// The cell `name` of `scenario`.
pub(crate) fn cell(scenario: Scenario, name: &str) -> Option<&'static Cell> {
    CELLS
        .iter()
        .find(|cell| cell.scenario == scenario && cell.name == name)
}

/// Each cell that runs, with its configuration.
fn running() -> impl Iterator<Item = (&'static Cell, Config)> {
    CELLS.iter().filter_map(|cell| match cell.status {
        Status::Measured(config) => Some((cell, config)),
        Status::NotSupported(_) | Status::Pending(_) => None,
    })
}

/// Each measured cell with its configuration: each whose trials are timed.
pub(crate) fn measured() -> impl Iterator<Item = (&'static Cell, Config)> {
    running().filter(|(cell, _)| !cell.scenario.hostile())
}

/// Each cell of the hostile pages' passes with its configuration: each
/// whose trials count the rules its tool broke (#2439).
pub(crate) fn ruled() -> impl Iterator<Item = (&'static Cell, Config)> {
    running().filter(|(cell, _)| cell.scenario.hostile())
}

#[cfg(test)]
mod tests;
