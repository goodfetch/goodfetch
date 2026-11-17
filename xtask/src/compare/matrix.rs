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
//! them (#1903), the multi-domain site with [`PAGES_PER_DOMAIN`] pages on
//! each of its [`DOMAINS`] (#2283). #2316's four scenarios run scenario 1's
//! settings where a lead or a gap is decided: on pages of realistic size,
//! on a site of [`LONG_PAGES`] pages, and at 64 and 256 requests in flight.
//! They carry no bar until the owner sets one (#2252 Q11).
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
    /// OpenSpider's spider crawl through its engine's library (#2216).
    #[value(name = "openspider")]
    OpenSpider,
    /// fastCRW's server, `crw-server`, a crawl job on its HTTP API (#2217).
    #[value(name = "fastcrw")]
    FastCrw,
    /// crawlberg's engine, its streaming crawl through its library (#2218).
    #[value(name = "crawlberg")]
    Crawlberg,
}

impl Tool {
    pub(crate) const ALL: [Self; 9] = [
        Self::Foxhound,
        Self::Spider,
        Self::Crawlee,
        Self::Scrapy,
        Self::CrawleeRs,
        Self::Pagesift,
        Self::OpenSpider,
        Self::FastCrw,
        Self::Crawlberg,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Foxhound => "foxhound",
            Self::Spider => "spider",
            Self::Crawlee => "crawlee",
            Self::Scrapy => "scrapy",
            Self::CrawleeRs => "crawlee-rs",
            Self::Pagesift => "pagesift",
            Self::OpenSpider => "openspider",
            Self::FastCrw => "fastcrw",
            Self::Crawlberg => "crawlberg",
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
    /// Scenario 4, extraction to a dataset: scenario 1 on the article
    /// site, with each page's title and Main content written to the tool's
    /// dataset or output and held to the page's right answer (#2136).
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
    /// Scenario 1 on the article site over TLS and HTTP/2, under the
    /// Proving ground's 20 ms round trip (#2284), where requests in flight
    /// and connection reuse decide the result.
    #[serde(rename = "crawl-like")]
    #[value(name = "crawl-like")]
    CrawlLike,
    /// Each tool's own choice of HTTP or a browser, page by page, on the
    /// rungs site (#2411): its rate, the pages it rendered and the pages
    /// whose Records it got wrong, read against the site's manifest, not
    /// timed.
    Auto,
    /// A program's crawl of the article site (#2317): its handler reads
    /// each page's twenty fields, pushes one Record and enqueues the
    /// page's links, each Record held to the page's answers.
    Handled,
}

/// What a scenario's trials read of each tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reading {
    /// A timed crawl's figures.
    Timed,
    /// The rules it broke on the hostile pages (#2439).
    Rules,
    /// What it read on the rungs site against the site's manifest (#2411).
    Manifest,
}

/// How a scenario's trials reach the Proving ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Network {
    /// Plain HTTP on loopback, with no latency added.
    Loopback,
    /// TLS from the Proving ground's local certificate authority on
    /// loopback, with no latency added: the rungs site's browser-identity
    /// pages answer only a TLS fingerprint (#2411).
    TlsOnLoopback,
    /// TLS from the Proving ground's local certificate authority, with
    /// HTTP/2 offered first, through its latency proxy at a 20 ms round
    /// trip (#2284).
    TlsUnderRtt20,
}

impl Network {
    /// The Proving ground's `--network-profile`.
    pub(crate) fn network_profile(self) -> &'static str {
        match self {
            Self::Loopback | Self::TlsOnLoopback => "none",
            Self::TlsUnderRtt20 => "rtt-20",
        }
    }

    /// The scheme of the site's URLs.
    pub(crate) fn scheme(self) -> &'static str {
        if self.tls() { "https" } else { "http" }
    }

    /// Whether the site is served over TLS, so each tool trusts the
    /// Proving ground's certificate authority and each request for the
    /// site's pages is held to TLS and HTTP/2.
    pub(crate) fn tls(self) -> bool {
        match self {
            Self::Loopback => false,
            Self::TlsUnderRtt20 | Self::TlsOnLoopback => true,
        }
    }
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
    /// being `n` modulo [`DOMAINS`], from 1, with at least
    /// [`PAGES_PER_DOMAIN`] pages on each.
    MultiDomain,
    /// The script-rendered site on `rendered.localhost`.
    Rendered,
    /// The realistic-page site on `realistic.localhost`.
    Realistic,
    /// The article site on `article.localhost` (#2136): `scale.test`'s tree
    /// of pages, each an article whose right title and Main content the
    /// Proving ground holds.
    Article,
    /// The hostile pages: each task's host, reached on the Proving
    /// ground's address with its host name sent as each request's `Host`.
    Hostile,
    /// The rungs site on `rungs.localhost` (#2410): static,
    /// browser-identity, script-rendered, refused and blocked pages, each
    /// page's right Records in the site's manifest.
    Rungs,
}

/// The multi-domain site's domains, as the Proving ground serves them
/// (#2283).
pub(crate) const DOMAINS: u64 = 1000;

/// The fewest pages the multi-domain site has on each domain (#2283): at 1
/// s per domain, its 1,000 pages a second then sit above what each tool
/// can do, so each tool sets its own figure.
pub(crate) const PAGES_PER_DOMAIN: u64 = 10;

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
            Self::Article => "article.localhost".to_owned(),
            Self::Hostile => super::hostile::TRAPS.host.to_owned(),
            Self::Rungs => "rungs.localhost".to_owned(),
        }
    }

    /// Every host of the site.
    pub(crate) fn hosts(self) -> Vec<String> {
        match self {
            Self::Scale | Self::Rendered | Self::Realistic | Self::Article | Self::Rungs => {
                vec![self.host_of(1)]
            }
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
    pub(crate) const ALL: [Self; 14] = [
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
        Self::CrawlLike,
        Self::Auto,
        Self::Handled,
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
            Self::CrawlLike => "crawl-like",
            Self::Auto => "auto",
            Self::Handled => "handled",
        }
    }

    /// The scenario whose settings each tool runs: scenario 1's for
    /// #2316's, which change only the site or the requests in flight, for
    /// the hostile pages' `traps` pass and for the crawl-like scenario,
    /// which changes only the site and the network, so each tool's side
    /// takes them as `raw`.
    pub(crate) fn settings(self) -> Self {
        match self {
            Self::Realistic
            | Self::Long
            | Self::Raw64
            | Self::Raw256
            | Self::Hostile16
            | Self::CrawlLike => Self::Raw,
            Self::Raw
            | Self::Durable
            | Self::Polite
            | Self::Extraction
            | Self::Browser
            | Self::Hostile
            | Self::Auto
            | Self::Handled => self,
        }
    }

    /// The issue that says why its cells carry no bar until the owner sets
    /// one: #2316's four scenarios (#2252 Q11), the crawl-like scenario
    /// (#2284) and the hostile pages' (#2439, Q3); `None` for a scenario
    /// whose cells carry one.
    pub(crate) fn no_bar(self) -> Option<&'static str> {
        match self {
            Self::Raw | Self::Durable | Self::Polite | Self::Extraction | Self::Browser => None,
            Self::Realistic | Self::Long | Self::Raw64 | Self::Raw256 => Some("#2316"),
            Self::CrawlLike => Some("#2284"),
            Self::Hostile | Self::Hostile16 => Some("#2439, Q3"),
            Self::Auto => Some("#2411, G-04 Q11"),
            Self::Handled => Some("#2317"),
        }
    }

    /// How its trials reach the Proving ground: over TLS under a 20 ms
    /// round trip in the crawl-like scenario, over TLS on loopback in the
    /// auto cell, on plain loopback otherwise.
    pub(crate) fn network(self) -> Network {
        match self {
            Self::CrawlLike => Network::TlsUnderRtt20,
            Self::Auto => Network::TlsOnLoopback,
            Self::Raw
            | Self::Durable
            | Self::Polite
            | Self::Extraction
            | Self::Browser
            | Self::Realistic
            | Self::Long
            | Self::Raw64
            | Self::Raw256
            | Self::Hostile
            | Self::Hostile16
            | Self::Handled => Network::Loopback,
        }
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
            | Self::Raw256
            | Self::CrawlLike
            | Self::Auto
            | Self::Handled => &[],
        }
    }

    /// Whether its trials count the rules each tool broke instead of
    /// timing a crawl: the hostile pages'.
    pub(crate) fn hostile(self) -> bool {
        !self.hostile_tasks().is_empty()
    }

    /// What its trials read of each tool.
    pub(crate) fn reading(self) -> Reading {
        if self.hostile() {
            Reading::Rules
        } else if self == Self::Auto {
            Reading::Manifest
        } else {
            Reading::Timed
        }
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
            | Self::Hostile16
            | Self::CrawlLike
            | Self::Auto
            | Self::Handled => crate::rss::crawl::CONCURRENCY,
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
            Self::Raw | Self::Durable | Self::Long | Self::Raw64 | Self::Raw256 => Site::Scale,
            Self::Extraction | Self::CrawlLike | Self::Handled => Site::Article,
            Self::Polite => Site::MultiDomain,
            Self::Browser => Site::Rendered,
            Self::Realistic => Site::Realistic,
            Self::Hostile | Self::Hostile16 => Site::Hostile,
            Self::Auto => Site::Rungs,
        }
    }

    /// The pages of the site its trials crawl where the comparison's other
    /// sites have `pages`: as many, but the long scenario's [`LONG_PAGES`],
    /// and on the multi-domain site at least [`PAGES_PER_DOMAIN`] on each
    /// of its [`DOMAINS`], 10,000 where the others have
    /// [`super::trials::SITE_PAGES`].
    pub(crate) fn site_pages(self, pages: u64) -> u64 {
        if self == Self::Long {
            return LONG_PAGES;
        }
        match self.site() {
            Site::MultiDomain => pages.max(DOMAINS * PAGES_PER_DOMAIN),
            Site::Scale
            | Site::Article
            | Site::Rendered
            | Site::Realistic
            | Site::Hostile
            | Site::Rungs => pages,
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
                "3. Polite: robots.txt obeyed, 1 in flight per domain, 1 s delay, on the multi-domain site (10,000 pages on 1,000 domains)"
            }
            Self::Extraction => {
                "4. Extraction to a dataset: title and Main content per page of the article site, written to the tool's dataset or output and held to the page's right answer"
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
            Self::CrawlLike => {
                "Crawl-like: scenario 1 on the article site over TLS and HTTP/2, under the Proving ground's 20 ms round trip"
            }
            Self::Auto => {
                "Auto: each tool's own choice of HTTP or a browser, page by page, on the rungs site of static, browser-identity, script-rendered, refused and blocked pages, each page's Records held to the site's manifest"
            }
            Self::Handled => {
                "Handled crawl: scenario 1 on the article site with a program's handler on each page, which reads its twenty fields, pushes one Record and enqueues its links, each Record held to the page's answers"
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

/// The capacity Spider's main row reads its pages at with 64 requests in
/// flight, the sizes cell's most (spec #2165): [`SPIDER_CAPACITY`] holds at
/// 16. Of 40 crawls of scenario 1's 10,000-page site at 64 in flight at
/// each capacity, on the gate host on 2026-11-13 at a 1-minute load of 6 to
/// 12, 15 lost pages at 64 and none at 128, 256 or 512.
pub(crate) const SPIDER_CAPACITY_AT_64: usize = 128;

/// The same for the extraction scenario, whose reader starts a task for
/// each page, which reads its Main content, and writes each page read
/// (#2136), on the gate host on 2026-11-13 at a 1-minute load of 10 to 22:
/// 16 of 30 crawls lost pages at 32, and none of 40 at 64. With
/// `llm_readability` run in the reader itself, 7 or 8 of 8 crawls lost
/// pages at each capacity from 256 to 4,096: the reader, not the channel,
/// was what fell behind. A counted run takes a channel that holds the
/// whole site instead (`trial::output::traced`).
pub(crate) const SPIDER_EXTRACTION_CAPACITY: usize = 64;

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
    /// With `llm_readability`, the readability step of
    /// `spider_transformations`, which reads each page's Main content, and
    /// the title's decoding: the extraction scenario's (#2136).
    Extraction,
    /// [`Self::Extraction`] on `mimalloc`, the allocator Spider's README
    /// recommends for a crawl that also parses, and the one Foxhound's
    /// executable sets.
    ExtractionMimalloc,
    /// With Spider's `smart` feature: each page fetched over HTTP first and
    /// rendered in Chrome only where Spider finds it needs a browser, its
    /// Records read from what Spider delivers (#2411).
    Smart,
    /// With `lol_html`, which reads the handled crawl's twenty fields from
    /// each page as it streams, and the decoding of their text (#2317).
    Handled,
}

impl SpiderBuild {
    /// The features of `xtask/compare-spider` the build turns on, if any, as
    /// `cargo build --features` takes them.
    pub(crate) fn feature(self) -> Option<&'static str> {
        match self {
            Self::Http => None,
            Self::Chrome => Some("chrome"),
            Self::Defaults => Some("defaults"),
            Self::Bloom => Some("bloom"),
            Self::Jemalloc => Some("jemalloc"),
            Self::Extraction => Some("extraction"),
            Self::ExtractionMimalloc => Some("extraction,mimalloc"),
            Self::Smart => Some("smart"),
            Self::Handled => Some("handled"),
        }
    }

    /// The build's folder among Spider's side's builds.
    pub(crate) fn folder(self) -> String {
        self.feature().map_or_else(
            || "spider".to_owned(),
            |features| format!("spider-{}", features.replace(',', "-")),
        )
    }
}

/// The folder of crawlee-rs's side's build among the Rust sides' builds,
/// beside Spider's (`just compare`).
pub(crate) const CRAWLEE_RS_FOLDER: &str = "crawlee-rs";

/// The folder of pagesift's side's build among the Rust sides' builds.
pub(crate) const PAGESIFT_FOLDER: &str = "pagesift";

/// The folder of OpenSpider's side's build among the Rust sides' builds.
pub(crate) const OPENSPIDER_FOLDER: &str = "openspider";

/// The folder of fastCRW's side's build among the Rust sides' builds, where
/// `just compare` also installs the server it starts, under the profile's
/// folder, and `{FASTCRW_FOLDER}-server` holds that server's build.
pub(crate) const FASTCRW_FOLDER: &str = "fastcrw";

/// The folder of crawlberg's side's build among the Rust sides' builds.
pub(crate) const CRAWLBERG_FOLDER: &str = "crawlberg";

/// The folder of chromiumoxide's side's build among the Rust sides' builds,
/// for the Collie scenario's chromiumoxide arm (issue 2444).
pub(crate) const CHROMIUMOXIDE_FOLDER: &str = "chromiumoxide";

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
    /// `politeness.domain` delay 0 and concurrency 16, no format; or, where
    /// `auto`, with `fetch.tier` left at `auto`, its default, which learns
    /// each Label's tier on each domain from samples fetched on `http` and
    /// in a browser (ADR 0020).
    Foxhound {
        durability: &'static str,
        commit_interval: Option<u64>,
        auto: bool,
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
    /// OpenSpider's side at the scenario's settings: its engine's spider
    /// crawl.
    OpenSpider,
    /// fastCRW's side at the scenario's settings: a crawl job on the server
    /// it starts.
    FastCrw,
    /// crawlberg's side at the scenario's settings: its engine's streaming
    /// crawl.
    Crawlberg,
    /// Foxhound's TypeScript program on `@goodfetch/foxhound`, run by
    /// Node with Foxhound's executable as its Attached engine: a handler on
    /// each page that reads its fields, pushes a Record and enqueues its
    /// links (#2317), `fetch.tier` not set, so the engine's own tier
    /// choice samples pages on `http` and in a browser (#1602).
    FoxhoundProgram,
}

impl Config {
    /// The tool this configuration runs.
    pub(crate) fn tool(self) -> Tool {
        match self {
            Self::Foxhound { .. } | Self::FoxhoundProgram => Tool::Foxhound,
            Self::Spider { .. } => Tool::Spider,
            Self::Crawlee => Tool::Crawlee,
            Self::Scrapy => Tool::Scrapy,
            Self::CrawleeRs => Tool::CrawleeRs,
            Self::Pagesift => Tool::Pagesift,
            Self::OpenSpider => Tool::OpenSpider,
            Self::FastCrw => Tool::FastCrw,
            Self::Crawlberg => Tool::Crawlberg,
        }
    }

    /// Whether the row leaves Foxhound's `fetch.tier` at `auto`, so its
    /// samples fetch their pages twice, on `http` and then in a browser:
    /// the program sets no tier.
    pub(crate) fn samples(self) -> bool {
        matches!(
            self,
            Self::Foxhound { auto: true, .. } | Self::FoxhoundProgram
        )
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
pub(crate) const CELLS: [Cell; 130] = [
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
            auto: false,
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
        mapping: "its library's `Scraper::crawl`, the entry point its own crawl benchmark uses: breadth-first \
                  from the first page over its whole host (`crawlEntireDomain`), on its HTTP engine \
                  with each page's links its only format, a limit of 100,000 pages (its API's \
                  maximum) and its sitemap not read (`sitemap: skip`); `maxConcurrency: 16`, its \
                  limiter starting at 16 in flight and capped there (`global_concurrency`, \
                  `global_ceiling`), its controller moving under the cap; `politeness` off, its \
                  administrator's switch, so no interval between a site's requests, and \
                  `per_host_concurrency` 16 where its default is 2, the pages its crawl holds \
                  in flight on a site with `politeness` off too; robots.txt not obeyed \
                  (`ignoreRobotsTxt`, `respect_robots: false`); nothing kept on disk; on its own \
                  allocator (`SmallCache`, tuned at start as its executable does)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "its engine's spider crawl (`os_engine::start_crawl`), the call its own command line \
                  starts a crawl with, from the first page over its host, on its own HTTP \
                  fetcher: `speed.threads: 16`, with at most 6 requests in flight on the host, \
                  one on each of its at most 6 connections to it under HTTP/1.1 \
                  (`connections_per_host: auto`, its default), no rate (`max_urls_per_sec` \
                  unset) and its adaptive throttle on (its default); robots.txt not obeyed \
                  (`robots.mode: ignore`); its crawl file, an SQLite database, kept in the \
                  trial's folder on its `Balanced` storage profile, its command line's; the \
                  analysis its command line runs once a crawl ends (its SEO reports) not run",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::OpenSpider),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "its server, `crw-server`, run as its own process by its side and driven over \
                  its HTTP API as its users drive it: one crawl job (`POST /v1/crawl`), polled \
                  every 25 ms until it ends (`GET /v1/crawl/{id}`), from the first page with \
                  `limit: 1000` and `maxDepth: 10`, its highest, and links its one format \
                  (`formats: [\"links\"]`); one page fetched at a time, since a crawl job awaits \
                  each fetch before the next (`run_crawl_inner`, `crw-crawl/src/crawl.rs`), \
                  whatever its `max_concurrency` and `per_host_max_concurrent`, both set to the \
                  scenario's 16; no rate (`crawler.requests_per_second: 0`) and its API's rate \
                  limit off (`server.rate_limit_rps: 0`), which would count the side's polls \
                  alone; robots.txt not obeyed (`respect_robots_txt: false`); no browser \
                  (`renderer.mode: none`); the site's host sent as each page's `Host` (the crawl \
                  request's `headers`, which its crawl applies to every page it fetches) and the \
                  loopback address allowed (`CRW_ALLOW_LOOPBACK_FOR_TESTS`), which its server \
                  refuses otherwise",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::FastCrw),
    },
    Cell {
        scenario: Scenario::Raw,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "its engine's streaming crawl (`CrawlEngineHandle::crawl_stream`), which hands \
                  each page on as it is crawled, from the first page over its host, on its own \
                  HTTP client (reqwest): `max_concurrent: 16`, the requests in flight across its \
                  crawl, no delay between a host's requests (`rate_limit_ms: 0` where its default \
                  is 200 ms) and no page or depth bound (`max_pages`, `max_depth`, unset by \
                  default); robots.txt not obeyed (`respect_robots_txt: false`, its default); each \
                  page converted to Markdown, which its crawl always does; its frontier in memory; \
                  private addresses allowed (`allow_private_networks`), which it refuses \
                  otherwise, and the site's host sent as each page's `Host` (`custom_headers`, \
                  which it sends to its first page's host alone)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
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
            auto: false,
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
            auto: false,
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
        scenario: Scenario::Durable,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "none: its crawl file holds nothing yet when a kill lands halfway",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(
            "OpenSpider's writer commits its crawl file in groups and, while pages come faster \
             than it writes them, gathers up to 4,000 pages or 500 ms into one transaction, so \
             on the Proving ground it fetches about 500 pages before its first commit, and a \
             SIGKILL once half the site is served leaves the file with no page and an empty \
             queue; the process started again on it (`os_engine::resume_crawl`, its command \
             line's `--resume`), once the file's 10 s heartbeat has gone stale, ends at once \
             with nothing to fetch, and does not start again from its first page. On \
             2026-11-16 (#2216), 3 of 3 crawls of the 1,000-page site so killed fetched 526 to \
             610 pages, which no same-work check passes",
        ),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "none: fastCRW keeps its crawl jobs in its server's memory",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(FASTCRW_IN_MEMORY),
    },
    Cell {
        scenario: Scenario::Durable,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "none: crawlberg keeps its frontier in memory",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(CRAWLBERG_IN_MEMORY),
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
                  and `crawl.scope.strategy: all`, since the site spans 1,000 domains and links \
                  nothing else",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "full",
            commit_interval: None,
            auto: false,
        }),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::Foxhound,
        name: "foxhound auto",
        mapping: "the foxhound row's settings with `fetch.tier` left at `auto`, its default \
                  (#2283; ADR 0020's note of #768): the first `fetch.sample.count` (3) pages of \
                  the crawl's Label on each domain are samples, each fetched on `http` and then \
                  in Chrome for Testing, whose run commits; the row reports the share of its \
                  pages that were samples, read from each Record's Provenance, and is held to the \
                  bytes of each page's first answer, since a sample fetches its page twice",
        pending: &[],
        row: Row::Sensitivity,
        status: Status::Measured(Config::Foxhound {
            durability: "full",
            commit_interval: None,
            auto: true,
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
                  since the site spans 1,000 domains and links nothing else; \
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
                  site spans 1,000 domains and links nothing else), `ROBOTSTXT_OBEY: True`, \
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
                  `EnqueueStrategy::All`, since the site spans 1,000 domains and links nothing \
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
                  since the site spans 1,000 domains and links nothing else; nothing kept on \
                  disk. Its frontier ranks a domain's pages after its first four as repeats of one \
                  template, since the site's pages differ only in their numbers, and takes those \
                  last, from one queue for every domain, whose head waits for its domain's interval",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "none: OpenSpider's one pace setting is its whole crawl's, not each domain's",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(OPENSPIDER_NOT_POLITE),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "none: fastCRW follows links on its first page's origin alone",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(FASTCRW_ONE_ORIGIN),
    },
    Cell {
        scenario: Scenario::Polite,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "none: crawlberg follows a link only on its first page's host",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(CRAWLBERG_ONE_HOST),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings with `formats: { text: true }`, the text of each page's \
                  Main content as Foxhound reads it by default (its Clean-up rules, the Filter \
                  lists at their defaults, then dom_smoothie), and the field `title` (the CSS \
                  selector `title`), with `dataset.records` on, each page's Record committed to \
                  the code-free crawl's Dataset `pages` in its run file",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
            auto: false,
        }),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Foxhound,
        name: FILTER_LISTS_OFF,
        mapping: "the foxhound row's settings with `filter_lists.ads`, `filter_lists.trackers` \
                  and `filter_lists.annoyances` off, so no Clean-up rule removes the page's ad \
                  and dom_smoothie alone reads its Main content: the like-for-like row against \
                  a tool that removes no ad, held to the right answer with the ad allowed; no \
                  bar reads it",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
            auto: false,
        }),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings, each page's URL, title and Main content written as one \
                  JSON line to `pages.jsonl`: its Main content is the text `llm_readability` \
                  0.0.17 reads (`extractor::extract`'s `Product.text`), the readability step of \
                  `spider_transformations` 2.39.13, Spider's only path to one, which removes no \
                  ad; its title is its page metadata's, character references decoded and white \
                  space collapsed in its own process; each page read in a task of its own, as \
                  `Website::subscribe`'s example does, from `subscribe(64)`, the smallest \
                  capacity at which this scenario's 10,000-page crawl lost no page; under \
                  strace and Callgrind, which slow those tasks unevenly, from a channel that \
                  holds the whole site",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Extraction,
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
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "scenario 1's crawl with one custom extractor, the text of its `main` \
                  (`custom_extraction`: CSS `main`, text output), each crawled page's URL, title \
                  and that text read from its crawl file once the crawl ends and written as a \
                  line of `pages.jsonl`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::OpenSpider),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "scenario 1's crawl job with each page's main content in Markdown, its one \
                  format (`formats: [\"markdown\"]`, `onlyMainContent: true`), each crawled page's URL \
                  (`metadata.sourceURL`), title (`metadata.title`) and that Markdown written as a \
                  line of `pages.jsonl`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::FastCrw),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "scenario 1's crawl with each page's URL, title (`metadata.title`) and Markdown \
                  (`markdown.content`) written as a line of `pages.jsonl`, the Markdown of its \
                  main content (`content.preprocessing_preset: \"aggressive\"`, the setting its \
                  own API's `onlyMainContent` turns on), which drops every footer and aside \
                  where its default, `standard`, drops navigation, forms and the headers, \
                  footers and asides that navigation hints at",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
    },
    Cell {
        scenario: Scenario::Extraction,
        tool: Tool::Spider,
        name: "spider mimalloc",
        mapping: "the spider row's settings (`subscribe(64)`), with `mimalloc` as its global \
                  allocator, which Spider's README recommends for a crawl that also parses, and \
                  the allocator Foxhound's executable sets; the spider row runs on the system's",
        pending: &[],
        row: Row::Sensitivity,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::ExtractionMimalloc,
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
            auto: false,
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
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "scenario 1's crawl with each page rendered in the same headless Chrome for \
                  Testing (`rendering.mode: javascript`, `rendering.chromium_path`), its tabs at \
                  their default, as many as threads up to the machine's cores, and its other \
                  rendering settings at their defaults",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::OpenSpider),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "scenario 1's crawl job with each page rendered in the same headless Chrome for \
                  Testing, which its side starts and its server drives over CDP (`renderer.mode: \
                  chrome` with `renderer.chrome.ws_url`, its `cdp` feature; `renderJs: true` and \
                  `renderer: \"chrome\"` in the request), each page fetched over HTTP first \
                  and then in Chrome, as its `renderJs` does; its anti-bot classifier's \
                  escalation off (`renderer.antibot.escalate_in_failover: false`), which at its \
                  default reads the site's short rendered pages as blocks and fails them",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::FastCrw),
    },
    Cell {
        scenario: Scenario::Browser,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "scenario 1's crawl with each page rendered in the same headless Chrome for \
                  Testing, which its engine starts and drives over CDP (`browser.mode: always` \
                  with `browser.chrome_path`, its `browser` feature), with no delay between a \
                  host's pages there either (`rate_limit_ms: 0`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
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
            auto: false,
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
        scenario: Scenario::Realistic,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "scenario 1's crawl (`speed.threads: 16`, robots.txt not obeyed) on the \
                  realistic-page site",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::OpenSpider),
    },
    Cell {
        scenario: Scenario::Realistic,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "scenario 1's crawl job (one page at a time, robots.txt not obeyed, no rate) on \
                  the realistic-page site",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::FastCrw),
    },
    Cell {
        scenario: Scenario::Realistic,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "scenario 1's crawl (`max_concurrent: 16`, robots.txt not obeyed, no delay) on \
                  the realistic-page site, following each redirect itself",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
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
            auto: false,
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
        scenario: Scenario::Long,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "scenario 1's crawl (`speed.threads: 16`, robots.txt not obeyed) on the site of \
                  100,000 pages, under its limit of 5,000,000 (`limits.max_urls`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::OpenSpider),
    },
    Cell {
        scenario: Scenario::Long,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "none: fastCRW crawls at most 1,000 pages a job",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(FASTCRW_THOUSAND_PAGES),
    },
    Cell {
        scenario: Scenario::Long,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "scenario 1's crawl (`max_concurrent: 16`, robots.txt not obeyed, no delay, no \
                  page bound) on the site of 100,000 pages",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
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
            auto: false,
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
        mapping: "scenario 1's crawl with `maxConcurrency: 64` and `per_host_concurrency` 64, \
                  its limiter starting at 64 in flight and capped there",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "none: OpenSpider keeps at most 6 requests in flight on a host",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(OPENSPIDER_SIX_IN_FLIGHT),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "none: a fastCRW crawl job fetches one page at a time",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(FASTCRW_ONE_IN_FLIGHT),
    },
    Cell {
        scenario: Scenario::Raw64,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "scenario 1's crawl with `max_concurrent: 64`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
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
            auto: false,
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
        mapping: "scenario 1's crawl with `maxConcurrency: 256` and `per_host_concurrency` 256, \
                  its limiter starting at 256 in flight and capped there",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "none: OpenSpider keeps at most 6 requests in flight on a host",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(OPENSPIDER_SIX_IN_FLIGHT),
    },
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "none: a fastCRW crawl job fetches one page at a time",
        pending: &[],
        row: Row::Main,
        status: Status::NotSupported(FASTCRW_ONE_IN_FLIGHT),
    },
    Cell {
        scenario: Scenario::Raw256,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "scenario 1's crawl with `max_concurrent: 256`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
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
            auto: false,
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
        scenario: Scenario::Hostile,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "its engine's spider crawl at OpenSpider's defaults (`CrawlConfig::default()`), \
                  setting nothing but its resolver answering the task's host name with the \
                  loopback address (`network.resolve`): robots.txt obeyed (`robots.mode: \
                  respect`) and the sitemaps it names read (`sitemaps.auto_discover_robots`), 5 \
                  threads, its adaptive throttle on, a limit of 5,000,000 pages and no browser \
                  (`rendering.mode: text`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::OpenSpider),
    },
    Cell {
        scenario: Scenario::Hostile,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "its crawl job at fastCRW's defaults, setting nothing but the Proving ground's \
                  open forward proxy (`crawler.proxy`), through which it reaches the task's host \
                  by name: robots.txt obeyed (`respect_robots_txt`), 10 requests a second and one \
                  in flight on a host (`requests_per_second`, `per_host_max_concurrent`), depth \
                  2 and 100 pages a job (`default_max_depth`, `default_max_pages`), and its \
                  `auto` renderer with no browser configured",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::FastCrw),
    },
    Cell {
        scenario: Scenario::Hostile,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "its engine's streaming crawl at crawlberg's defaults (`CrawlConfig::default()`), \
                  setting nothing but private addresses allowed (`allow_private_networks`) and the \
                  `Host` it sends (`custom_headers`): robots.txt not obeyed \
                  (`respect_robots_txt`), 200 ms between requests to a host (`rate_limit_ms` \
                  unset), 10 requests in flight (`max_concurrent` unset) and no page or depth \
                  bound (`max_pages`, `max_depth`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
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
            auto: false,
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
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "scenario 1's crawl (`speed.threads: 16`, robots.txt not obeyed, no rate) on the \
                  `traps` family",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::OpenSpider),
    },
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "scenario 1's crawl job (one page at a time, robots.txt not obeyed, no rate, at \
                  most 1,000 pages and depth 10, the API's ceilings, which end a trap) on the \
                  `traps` family, reached by name through the Proving ground's open forward proxy \
                  (`crawler.proxy`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::FastCrw),
    },
    Cell {
        scenario: Scenario::Hostile16,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "scenario 1's crawl (`max_concurrent: 16`, robots.txt not obeyed, no delay) on \
                  the `traps` family",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "scenario 1's settings (its maximum-throughput combination, 16 in flight) on the \
                  article site over `https`, HTTP/2 negotiated by ALPN, the Proving ground's \
                  certificate authority trusted for `article.localhost` in its user configuration \
                  (`tls.trusted_cas`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "off",
            commit_interval: None,
            auto: false,
        }),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings (`with_concurrency_limit(16)`, `subscribe(64)`) on the \
                  article site over `https`, its `reqwest` client's HTTP/2 negotiated by ALPN, the \
                  certificate authority trusted through `SSL_CERT_FILE` and certificates verified \
                  (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Http,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "scenario 1's `CheerioCrawler` (`maxConcurrency: 16`) on the article site over \
                  `https`, its `got-scraping` client's HTTP/2 negotiated by ALPN, the certificate \
                  authority trusted through `NODE_EXTRA_CA_CERTS` and certificates verified \
                  (`ignoreSslErrors: false`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "scenario 1's `scrapy.Spider` (`CONCURRENT_REQUESTS` and \
                  `CONCURRENT_REQUESTS_PER_DOMAIN` 16) on the article site over `https`, through \
                  its HTTP/2 download handler (`H2DownloadHandler`, the `twisted-http2` extra), \
                  the certificate authority trusted through `SSL_CERT_FILE` and certificates \
                  verified (`DOWNLOAD_VERIFY_CERTIFICATES`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler` (`max_concurrency(16)`, `persist_storage: false`) on \
                  the article site over `https`, its `reqwest` client's HTTP/2 negotiated by ALPN, \
                  the certificate authority trusted through `SSL_CERT_FILE` and certificates \
                  verified (`ignore_tls_errors: false`)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::Pagesift,
        name: "pagesift",
        mapping: "scenario 1's crawl (`maxConcurrency: 16`, `politeness` off) on the article site \
                  over `https`, its `pagesift-net` client's HTTP/2 negotiated by ALPN, the \
                  certificate authority trusted through `SSL_CERT_FILE` and certificates verified \
                  (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Pagesift),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::OpenSpider,
        name: "openspider",
        mapping: "scenario 1's crawl (`speed.threads: 16`, robots.txt not obeyed) on the article \
                  site over `https`, its `reqwest` client's HTTP/2 negotiated by ALPN \
                  (`http.version: auto`, its default), the certificate authority `SSL_CERT_FILE` \
                  names added to the certificates it trusts (`network.trusted_certificates`) and \
                  certificates verified (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::OpenSpider),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::FastCrw,
        name: "fastcrw",
        mapping: "scenario 1's crawl job (one page at a time, robots.txt not obeyed) on the \
                  article site over `https`, the certificate authority `SSL_CERT_FILE` names \
                  trusted and certificates verified (its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::FastCrw),
    },
    Cell {
        scenario: Scenario::CrawlLike,
        tool: Tool::Crawlberg,
        name: "crawlberg",
        mapping: "scenario 1's crawl (`max_concurrent: 16`, robots.txt not obeyed, no delay) on \
                  the article site over `https`, the certificate authority `SSL_CERT_FILE` names \
                  trusted by its TLS (rustls, with the platform's verifier) and certificates \
                  verified (`danger_accept_invalid_certs: false`, its default)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlberg),
    },
    Cell {
        scenario: Scenario::Auto,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "a code-free crawl of the rungs site over `https` at `fetch.tier: auto`, its \
                  default, with `fetch.http_from: learned`, so each Label's tier on each domain \
                  is learned from samples fetched on `http` and in Chrome for Testing; the \
                  Proving ground's certificate authority trusted for `rungs.localhost` \
                  (`tls.trusted_cas`), `run_state.durability: full`, `dataset.records: true`, \
                  robots.txt not obeyed, `politeness.domain` delay 0 and concurrency 16 \
                  (`sharing: run`) and `crawl.handler.concurrency: 16`; each page's Records \
                  read with the fields `id` (`p.id`) and `name` (`h1.name`), and a page \
                  counted rendered when its Records' Provenance names the `browser` or \
                  `light_browser` tier",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Foxhound {
            durability: "full",
            commit_interval: None,
            auto: true,
        }),
    },
    Cell {
        scenario: Scenario::Auto,
        tool: Tool::Spider,
        name: "spider",
        mapping: "Spider's `smart` feature, `crawl_smart()`, which fetches each page over HTTP \
                  first and renders it in the Chrome `CHROME_BIN` names only where Spider finds \
                  it needs a browser, on the rungs site over `https` (certificate errors \
                  ignored, its Chrome's default), 16 in flight, robots.txt not obeyed, its \
                  pages read through `subscribe(64)`; each page's Records read from the HTML \
                  Spider delivers, and a page counted rendered when Spider rendered it in \
                  Chrome (`Page::bytes_transferred`, which only its Chrome path sets)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Smart,
            capacity: SPIDER_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Auto,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "Crawlee's `AdaptivePlaywrightCrawler` at its defaults, which renders a page \
                  in Chrome or fetches it over HTTP as it predicts, on the rungs site over \
                  `https` in the Chrome `--browser` names (certificate errors ignored, Crawlee's \
                  default over HTTP, and in Chrome through `--ignore-certificate-errors`), \
                  `maxConcurrency: 16`, robots.txt not obeyed, nothing kept on disk; each \
                  page's Records read with `parseWithCheerio()`, its links enqueued only when \
                  it has one (in Chrome, `enqueueLinks()` waits 5 s for a link and fails the \
                  request when none comes), and a page counted rendered when the run of its \
                  request handler that Crawlee kept had a browser page",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Handled,
        tool: Tool::Foxhound,
        name: "foxhound",
        mapping: "a TypeScript program on `@goodfetch/foxhound` (`xtask/compare-foxhound/crawl.ts`) \
                  run by Node, Foxhound's executable its Attached engine, with one handler for \
                  the Label `page` that makes four exchanges a page: the handled request, one \
                  `page.read` of the twenty fields, one `push` of them to the Dataset `pages` and \
                  one `enqueueLinks` of the page's links; `fetch.tier` not set, so the engine's \
                  own tier choice samples pages on `http` and in Chrome for Testing (#1602), and \
                  scenario 1's other settings, `run_state.durability: off`, robots.txt not \
                  obeyed, `politeness.domain` delay 0 and concurrency 16 (`sharing: run`), \
                  `crawl.handler.concurrency: 16`, `dataset.records: true`",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::FoxhoundProgram),
    },
    Cell {
        scenario: Scenario::Handled,
        tool: Tool::Foxhound,
        name: "foxhound rust",
        mapping: "the foxhound row's handler written in Rust on the Rust SDK",
        pending: &[],
        row: Row::Main,
        status: Status::Pending(HANDLED_RUST_PENDING),
    },
    Cell {
        scenario: Scenario::Handled,
        tool: Tool::Foxhound,
        name: "foxhound getters",
        mapping: "the foxhound row's handler reading each field with the TypeScript getters, one \
                  call a field, in place of one `page.read`",
        pending: &[],
        row: Row::Main,
        status: Status::Pending(HANDLED_GETTERS_PENDING),
    },
    Cell {
        scenario: Scenario::Handled,
        tool: Tool::Spider,
        name: "spider",
        mapping: "scenario 1's settings, each page's twenty fields read with `lol_html` 2.9.0 \
                  CSS selectors as Spider delivers it, text with its character references \
                  decoded, written with its URL as one JSON line to `pages.jsonl`, the page's \
                  links followed by Spider's own crawl; each page read in a task of its own from \
                  `subscribe(64)`, as the extraction scenario's spider row does",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Spider {
            build: SpiderBuild::Handled,
            capacity: SPIDER_EXTRACTION_CAPACITY,
        }),
    },
    Cell {
        scenario: Scenario::Handled,
        tool: Tool::Crawlee,
        name: "crawlee",
        mapping: "scenario 1's `CheerioCrawler`, its request handler reading each page's twenty \
                  fields with Cheerio, giving them to `pushData` and calling `enqueueLinks()`, \
                  its default Dataset kept on disk (`persistStorage: true`, its default, which \
                  keeps its request queue on disk too)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Crawlee),
    },
    Cell {
        scenario: Scenario::Handled,
        tool: Tool::Scrapy,
        name: "scrapy",
        mapping: "scenario 1's settings, its callback reading each page's twenty fields with \
                  Scrapy's CSS selectors and yielding them as an Item, which an Item pipeline \
                  writes as one JSON line to `items.jsonl`, then following the page's links",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::Scrapy),
    },
    Cell {
        scenario: Scenario::Handled,
        tool: Tool::CrawleeRs,
        name: "crawlee-rs",
        mapping: "scenario 1's `HtmlCrawler`, its handler reading each page's twenty fields from \
                  its `Document`, giving them to `push_data` and enqueueing the page's links, its \
                  default Dataset kept on disk (`persist_storage: true`, its default, which \
                  keeps its request queue on disk too)",
        pending: &[],
        row: Row::Main,
        status: Status::Measured(Config::CrawleeRs),
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

/// Why the handled crawl's Rust row gives no figure yet.
pub(crate) const HANDLED_RUST_PENDING: &str =
    "#1469, which brings the Rust SDK's crawl handler that the row's program is written on";

/// Why the handled crawl's getters row gives no figure yet.
pub(crate) const HANDLED_GETTERS_PENDING: &str = "#724 and #1296's TypeScript part, which bring \
     the TypeScript getters the row's handler reads each field with";

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
     102 requests over 101.5 s, one a second across all 50 domains the site then had, read robots.txt on 1 of them, \
     and fetched the page that robots.txt disallows (`Disallow: /sites/*/private`)";

/// Why OpenSpider has no polite cell: what its code at the pinned commit
/// does.
const OPENSPIDER_NOT_POLITE: &str = "OpenSpider spaces its fetches with one rate for its whole \
     crawl, `speed.max_urls_per_sec`: at its pinned commit, a bucket of one token in its fetcher \
     that holds pages, resources, robots.txt and sitemaps alike, whatever their domain \
     (`crates/os-fetch/src/rate.rs`), not a delay between one domain's requests. Its only delay of \
     a site's own is robots.txt's `Crawl-delay` (`robots.respect_crawl_delay`), which the site \
     does not set, and it fetches a host other than its first page's for that URL's status alone \
     unless `cdns` names it (`crates/os-url/src/scope.rs`)";

/// Why OpenSpider has no cell of 64 or 256 requests in flight: what its
/// code at the pinned commit does.
const OPENSPIDER_SIX_IN_FLIGHT: &str = "OpenSpider keeps at most 6 requests in flight on a \
     host, whatever its threads: at its pinned commit, its fetcher opens at most 6 connections \
     to a host (`ConnectionsPerHost::MAX`, `crates/os-model/src/config.rs`), and under HTTP/1.1, \
     which it speaks to the Proving ground's `http` site, each carries one request at a time, a \
     further request waiting for one rather than opening another \
     (`crates/os-fetch/src/client/lanes.rs`); a cell of 64 or 256 would measure scenario 1's 6 \
     again";

/// Why fastCRW has no durable cell: what its server at the pinned version
/// does.
const FASTCRW_IN_MEMORY: &str = "fastCRW keeps its crawl jobs in its server's memory: at its pinned \
     version, a map from each job's id to its state (`AppState::crawl_jobs`, \
     `crw-server/src/state.rs`), which no file backs, so a crawl killed with its server is lost and \
     a new server has no job to resume";

/// Why fastCRW has no polite cell: what its crawl at the pinned version
/// does.
const FASTCRW_ONE_ORIGIN: &str = "fastCRW follows a link only on its first page's origin, port \
     included: at its pinned version, a crawl job drops every link to another origin \
     (`enqueue_discovered_links`, `crw-crawl/src/crawl.rs`) and has no setting to follow one, so \
     it would crawl one of the site's 1,000 domains";

/// Why fastCRW has no long cell: what its crawl at the pinned version
/// does.
const FASTCRW_THOUSAND_PAGES: &str = "fastCRW crawls at most 1,000 pages a job, a hundredth of \
     the site's 100,000 pages: at its pinned version, a crawl job's `limit` is held to 1,000 \
     whatever the request asks (`max_pages.min(1000)` in `run_crawl_inner`, \
     `crw-crawl/src/crawl.rs`), and its depth to 10";

/// Why fastCRW has no cell of 64 or 256 requests in flight: what its crawl
/// at the pinned version does.
const FASTCRW_ONE_IN_FLIGHT: &str = "a fastCRW crawl job fetches one page at a time: at its pinned \
     version, it takes the next page from its queue only once the fetch before it ends \
     (`run_crawl_inner`, `crw-crawl/src/crawl.rs`), whatever its `max_concurrency` and \
     `per_host_max_concurrent`, so a cell of 64 or 256 would measure scenario 1's one again";

/// Why crawlberg has no durable cell: what its library at the pinned
/// version ships.
const CRAWLBERG_IN_MEMORY: &str = "crawlberg keeps its frontier in memory: at its pinned version, \
     its frontiers are `InMemoryFrontier` and `LifoFrontier` (`src/defaults/frontier.rs`), and \
     nothing it ships writes a crawl's frontier to disk or resumes a crawl, so a crawl killed \
     with its process is lost; its `Frontier` trait would take one written for it, which would \
     be the comparison's code, not crawlberg's";

/// Why crawlberg has no polite cell: what its crawl at the pinned version
/// does.
const CRAWLBERG_ONE_HOST: &str = "crawlberg follows a link only on its first page's host, or \
     that host's subdomains with `allow_subdomains`: at its pinned version, a page link to any \
     other host is dropped (`host_in_scope`, `src/engine/link_scope.rs`) whatever \
     `stay_on_domain`, which loosens document links alone, and the site's 1,000 domains are \
     each other's siblings (`polite-<d>.localhost`), so it would crawl one of them";

/// The name of Foxhound's row of the extraction scenario with its Filter
/// lists off (#2136).
pub(crate) const FILTER_LISTS_OFF: &str = "foxhound filter lists off";

/// What the output check holds a cell's pages to (#2136; see
/// `extracted`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputCheck {
    /// The whole right answer: the title, every paragraph, and neither the
    /// boilerplate's markers nor the ad's.
    Whole,
    /// The same with the ad's marker allowed, since nothing removes the ad
    /// there.
    AdAllowed,
    /// The handled crawl's twenty fields of each page, each its answer
    /// (#2317).
    Fields,
}

/// Why pagesift's polite cell runs one trial of each pass, where the run
/// asks for more.
const PAGESIFT_POLITE_ONE_TRIAL: &str = "the owner's decision of 2026-11-14 (#2215): a trial of \
     this cell takes about 50 minutes, since its frontier takes a domain's pages after its first \
     four last, at 1.5 to 3 pages a second, so one timed trial and one run under strace stand \
     for it where the run asks for more; its figures are that trial's, with no spread";

impl Cell {
    /// Why the cell runs one trial of each pass, timed, under strace or
    /// any other, where the run asks for more; `None` when it runs the
    /// run's.
    pub(crate) fn one_trial(&self) -> Option<&'static str> {
        (self.scenario == Scenario::Polite && self.tool == Tool::Pagesift)
            .then_some(PAGESIFT_POLITE_ONE_TRIAL)
    }

    /// The cell's runs in a pass that asks for `asked`: one at most when it
    /// runs [`Cell::one_trial`], otherwise `asked`.
    pub(crate) fn runs(&self, asked: u32) -> u32 {
        if self.one_trial().is_some() {
            asked.min(1)
        } else {
            asked
        }
    }

    /// Whether the cell runs Foxhound with `filter_lists.ads`,
    /// `filter_lists.trackers` and `filter_lists.annoyances` off.
    pub(crate) fn filter_lists_off(&self) -> bool {
        self.scenario == Scenario::Extraction && self.name == FILTER_LISTS_OFF
    }

    /// What the output check holds the cell's pages to, in the extraction
    /// scenario: Foxhound's row with its Filter lists off with the ad
    /// allowed, every other cell to the whole right answer. A Foxhound
    /// trial that differs is refused; a competitor's keeps its figures,
    /// labelled. The crawl-like scenario crawls the same site at scenario
    /// 1's settings, which extract nothing.
    pub(crate) fn output_check(&self) -> Option<OutputCheck> {
        match (self.scenario, self.status) {
            (Scenario::Extraction, _) => Some(if self.filter_lists_off() {
                OutputCheck::AdAllowed
            } else {
                OutputCheck::Whole
            }),
            (Scenario::Handled, Status::Measured(_)) => Some(OutputCheck::Fields),
            _ => None,
        }
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
    running().filter(|(cell, _)| cell.scenario.reading() == Reading::Timed)
}

/// Each cell of the hostile pages' passes with its configuration: each
/// whose trials count the rules its tool broke (#2439).
pub(crate) fn ruled() -> impl Iterator<Item = (&'static Cell, Config)> {
    running().filter(|(cell, _)| cell.scenario.reading() == Reading::Rules)
}

/// Each cell of the auto cell's scenario with its configuration: each
/// whose runs read its tool's rate, pages rendered and pages wrong against
/// the rungs site's manifest (#2411).
pub(crate) fn auto() -> impl Iterator<Item = (&'static Cell, Config)> {
    running().filter(|(cell, _)| cell.scenario.reading() == Reading::Manifest)
}

#[cfg(test)]
mod tests;
