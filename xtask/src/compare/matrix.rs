//! The comparison matrix (the owner's comments of 2026-10-30 on #1577, ADR
//! 0138's Foxhound cells): each scenario sets every tool to the closest
//! equivalent configuration, and each cell writes that mapping. A tool that
//! cannot do what a scenario asks has a cell saying so, and a Foxhound
//! value that does not exist yet is marked pending with the ticket that
//! brings it; neither gives a figure. Each scenario crawls a site of the
//! Proving ground of [`super::trials::SITE_PAGES`] pages in the same tree:
//! `scale.test`'s, or for the polite and headless browser scenarios, the
//! multi-domain and script-rendered sites the Proving ground serves for
//! them (#1903).

use serde::{Deserialize, Serialize};

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
}

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
        }
    }

    /// Every host of the site.
    pub(crate) fn hosts(self) -> Vec<String> {
        match self {
            Self::Scale | Self::Rendered => vec![self.host_of(1)],
            Self::MultiDomain => (1..=DOMAINS).map(|page| self.host_of(page)).collect(),
        }
    }

    /// Whether a tool reaches the site by its host names, which the system
    /// and a browser both resolve to loopback (`*.localhost`), rather than
    /// on the Proving ground's address with a `Host` header.
    pub(crate) fn by_name(self) -> bool {
        !matches!(self, Self::Scale)
    }
}

impl Scenario {
    pub(crate) const ALL: [Self; 5] = [
        Self::Raw,
        Self::Durable,
        Self::Polite,
        Self::Extraction,
        Self::Browser,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Durable => "durable",
            Self::Polite => "polite",
            Self::Extraction => "extraction",
            Self::Browser => "browser",
        }
    }

    /// The site its trials crawl.
    pub(crate) fn site(self) -> Site {
        match self {
            Self::Raw | Self::Durable | Self::Extraction => Site::Scale,
            Self::Polite => Site::MultiDomain,
            Self::Browser => Site::Rendered,
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
    /// Spider's HTTP mode, or with `chrome` its `chrome` feature's
    /// executable rendering each page in Chrome; at the scenario's settings.
    Spider { chrome: bool },
    /// Crawlee's side at the scenario's settings: its `CheerioCrawler`, or
    /// its `PlaywrightCrawler` in the headless browser scenario.
    Crawlee,
    /// Scrapy's side at the scenario's settings: its default crawl, or
    /// `scrapy-playwright`'s in the headless browser scenario.
    Scrapy,
}

impl Config {
    /// The tool this configuration runs.
    pub(crate) fn tool(self) -> Tool {
        match self {
            Self::Foxhound { .. } => Tool::Foxhound,
            Self::Spider { .. } => Tool::Spider,
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
pub(crate) const CELLS: [Cell; 21] = [
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
        status: Status::Measured(Config::Spider { chrome: false }),
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
        status: Status::Measured(Config::Foxhound { durability: "full" }),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::Spider,
        name: "spider",
        mapping: "none: Spider's delay and robots.txt are its crawl's, not each domain's",
        pending: &[],
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
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings with `formats: { text: true }`, the text of each page's \
                  main content, and the field `title` (the CSS selector `title`), each page's \
                  Record committed to the code-free crawl's Dataset `pages` in its run file",
        pending: &[
            "`run_state.durability: off` (#1927): runs `fast`, the fastest level there is",
            "`frontier.learning: false` (#1909): runs with learning on, the only state there is",
        ],
        status: Status::Measured(Config::Foxhound { durability: "fast" }),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings, each page's URL, title (Spider's page metadata) and HTML \
                  written as one JSON line to `pages.jsonl`: Spider reads no main content, so \
                  its page content is what it writes",
        pending: &[],
        status: Status::Measured(Config::Spider { chrome: false }),
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
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings on the `browser` tier (`fetch.tier: browser`), headless \
                  Chrome for Testing, its pinned build, from a Browser cache laid out beside \
                  the data folder (`GOODFETCH_BROWSERS_PATH`)",
        pending: &[
            "`run_state.durability: off` (#1927): runs `fast`, the fastest level there is",
            "`frontier.learning: false` (#1909): runs with learning on, the only state there is",
        ],
        status: Status::Pending(FOXHOUND_BROWSER_LINKS),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Spider,
        name: "spider",
        mapping: "Spider built with its `chrome` feature, `crawl()` rendering each page in the \
                  same headless Chrome for Testing (`CHROME_BIN`), with its own launch \
                  arguments, `with_respect_robots_txt(false)`, `with_concurrency_limit(16)`, \
                  nothing kept on disk",
        pending: &[],
        status: Status::Measured(Config::Spider { chrome: true }),
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
        status: Status::Measured(Config::Scrapy),
    },
];

/// Why Foxhound's headless browser cell gives no figure yet: what two of its
/// crawls of the script-rendered site showed, which no same-work check
/// passes.
const FOXHOUND_BROWSER_LINKS: &str = "Foxhound's `browser` tier misses the links a page's \
     script adds on some pages: on 2026-11-03 (#1903), two crawls of the script-rendered site \
     reached 890 and 990 of its 1,000 pages, each fetching page 9 but none of the 10 pages its \
     script links (and in the first, none of theirs); the fix is the issue #1903's report asks \
     for, which also covers the `Connection: close` it sends with each page, 19 bytes more in \
     each answer than the other tools are served";

/// Why Spider has no polite cell: what a crawl of the multi-domain site
/// with its delay and robots.txt showed.
const SPIDER_NOT_POLITE: &str = "Spider's `with_delay` waits between any two requests of \
     its crawl, whatever their domain, and it reads only its start URL's robots.txt. On 2026-11-03 \
     (#1903), a crawl of the multi-domain site's 100-page version with `with_delay(1000)`, \
     `with_respect_robots_txt(true)` and every domain allowed (`with_external_domains`) sent its \
     102 requests over 101.5 s, one a second across all 50 domains, read robots.txt on 1 of them, \
     and fetched the page that robots.txt disallows (`Disallow: /sites/*/private`)";

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
