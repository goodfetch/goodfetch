//! Foxhound's crawl benchmark scenario (ADRs 0071 and 0134): a code-free
//! crawl of a site served from memory through the fetch tiers' test seam,
//! [`FetchTiers::serving`], with no network, no sleep and no work whose
//! amount depends on timing. The engine is composed as Foxhound's
//! executable composes it, less the operations no crawl calls, on the
//! tests' paused current-thread runtime with blocking work inline, and
//! driven over the in-memory network: `hello`, then `crawl.start` from the
//! site's index with `follow: scope`, `run.finish`, which answers once the
//! crawl's work items are done, and `run.records`. Each page goes through
//! admission, the fetch, parsing, link discovery and extraction to a
//! Record of `pages`, its `h1` read as the field `title`.
//!
//! The site is generated from the literal [`PAGES`]: an index linking
//! twelve pages of about 1 kB to 1 MB, two of each size, so a copy per
//! page shows as a step. Each page's body is its own head, as many
//! 128-byte paragraphs of `fixtures/paragraphs.html` as its size takes,
//! and a tail, sent in chunks of at most 16 KiB borrowed from the
//! fixture, so serving a page allocates only its head. `pages.source.toml`
//! records the bodies' total bytes, which `xtask perf` divides the crawl's
//! total bytes by (ADR 0134's 2× ceiling).
//!
//! [`Frontier`] adds the outcomes the Frontier's rows count beyond a links
//! batch, for the counts cell alone, so the heap benchmark's crawl stays as it
//! is: a seed that redirects, in two hops for the test that asks, a sitemap
//! seed and a seed with a Payload under its Label's schema, each reaching
//! one small page more; and, for the one cell that asks for them, pages
//! `/item?id=<n>` that differ by one parameter, which the Frontier's
//! learning reads its evidence for.
//!
//! [`start_with_seeds`] starts a crawl from a thousand seeds and aborts it
//! once its start answered: the seed admission of the crawl's Run state,
//! which the `admit_seeds` benchmarks bound their figures to (#2167).
#![expect(
    clippy::disallowed_types,
    reason = "the site's one counter of the bytes it sent, read once the crawl ended: no lock and no order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::path::Path;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::SystemTime;

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::ledger::Ledger;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_fetch_tier::{Body, FetchTiers, Hop, Reuse, Sending, Sent, Transport};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::run::{LimitScope, LimitSetter};
use goodfetch_protocol::usage::{INSTALL, Limit, REQUESTS};
use goodfetch_test_client::Hello;
use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};

/// The paragraphs every page's body is made of, each 128 bytes.
const PARAGRAPHS: &[u8] = include_bytes!("fixtures/paragraphs.html");

/// The bytes of one paragraph of [`PARAGRAPHS`].
const PARAGRAPH: usize = 128;

/// How many paragraphs each page linked from the index holds: about 1 kB,
/// 4 kB, 16 kB, 64 kB, 256 kB and 1 MB, two of each. The index holds as
/// many as the first.
pub(crate) const PAGES: [usize; 12] =
    [8, 8, 31, 31, 125, 125, 500, 500, 2_000, 2_000, 7_812, 7_812];

/// The host the site is served on.
const HOST: &str = "pages.test";

/// The end of every page's body.
const TAIL: &[u8] = b"</main></body></html>\n";

/// The site's robots.txt, when it serves one: every page allowed, so the
/// crawl reads it, parses it and obeys it without skipping a page.
const ROBOTS_TXT: &str = "User-agent: *\nAllow: /\n";

/// One page of the site: its head and how many paragraphs follow it.
struct Page {
    head: String,
    paragraphs: usize,
}

/// The page at `path` of a site linking the first `linked` of [`PAGES`],
/// `None` for a path the site does not serve.
fn page(path: &str, linked: usize) -> Option<Page> {
    if path == "/" {
        let mut links = String::new();
        for number in 0..linked {
            let _ = write!(
                links,
                "<li><a href=\"/page-{number}\">Page {number}</a></li>"
            );
        }
        return Some(Page {
            head: head("Index", &links),
            paragraphs: PAGES[0],
        });
    }
    let number: usize = path.strip_prefix("/page-")?.parse().ok()?;
    let paragraphs = *PAGES[..linked].get(number)?;
    let next = (number + 1) % linked;
    let links = format!(
        "<li><a href=\"/\">Index</a></li><li><a href=\"/page-{next}\">Page {next}</a></li>"
    );
    Some(Page {
        head: head(&format!("Page {number}"), &links),
        paragraphs,
    })
}

/// A page's head: its title, its `h1` and its links.
fn head(title: &str, links: &str) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body><h1>{title}</h1>\n<nav><ul>{links}</ul></nav>\n<main>\n"
    )
}

impl Page {
    fn len(&self) -> usize {
        self.head.len() + self.paragraphs * PARAGRAPH + TAIL.len()
    }
}

/// The total body bytes of every page of the site: the index and each
/// page it links.
pub(crate) fn body_bytes() -> u64 {
    std::iter::once("/".to_owned())
        .chain((0..PAGES.len()).map(|number| format!("/page-{number}")))
        .filter_map(|path| page(&path, PAGES.len()))
        .map(|page| u64::try_from(page.len()).unwrap_or(u64::MAX))
        .sum()
}

/// The site, served from memory, counting the body bytes it sends.
struct Site {
    sent: Arc<AtomicU64>,
    /// The attempts it served a page to.
    attempts: Arc<AtomicU64>,
    /// Run once per page it serves: the benchmark's own extra unit of work.
    plant: Option<Plant>,
    /// Whether it also serves [`Frontier`]'s paths, and how many of its
    /// items with them.
    frontier: Option<usize>,
    /// Whether [`MOVED`] leads to [`MOVED_ON`] first, as
    /// [`Frontier::second_hop`] says.
    second_hop: bool,
    /// Whether each page has no paragraphs, as [`Options::small_pages`] says.
    small_pages: bool,
    hops: Arc<AtomicU64>,
    sitemaps: Arc<AtomicU64>,
    /// The most paragraphs it serves a page.
    largest: Option<usize>,
    /// Whether it serves [`ROBOTS_TXT`], else answers it 404.
    robots_txt: bool,
    /// Every request it was sent, each under a Politeness permit.
    requests: Arc<AtomicU64>,
    /// How many of [`PAGES`] it links and serves.
    linked: usize,
}

/// The seed that redirects, to [`MOVED_TO`].
const MOVED: &str = "/moved";
/// Where [`MOVED`] leads first, and which leads to [`MOVED_TO`], on a site
/// whose chain takes a second hop.
const MOVED_ON: &str = "/moved-on";
/// Where [`MOVED`] leads: its own URL with a trailing slash, so its one
/// hop teaches the Frontier an equivalent form (ADR 0120), work every
/// redirect a crawl follows between two forms does.
const MOVED_TO: &str = "/moved/";
/// The sitemap seed, whose one entry is [`FROM_SITEMAP`].
const SITEMAP: &str = "/sitemap.xml";
/// The sitemap's entry.
const FROM_SITEMAP: &str = "/from-sitemap";
/// The seed with a Payload.
const WITH_PAYLOAD: &str = "/with-payload";

/// The path of [`Frontier`]'s items, `/item?id=<n>`.
const ITEM: &str = "/item";

/// The item `/item?<query>` of a site serving `items` of them, each
/// linking the next: a page of its own title, so no two read alike.
fn item_page(query: Option<&str>, items: usize) -> Option<Page> {
    let number: usize = query?.strip_prefix("id=")?.parse().ok()?;
    let next = number + 1;
    let links = if next < items {
        format!("<li><a href=\"{ITEM}?id={next}\">Item {next}</a></li>")
    } else {
        String::new()
    };
    (number < items).then(|| Page {
        head: head(&format!("Item {number}"), &links),
        paragraphs: 0,
    })
}

/// The page at one of the paths only [`Frontier`] serves.
fn frontier_page(path: &str) -> Option<Page> {
    [MOVED_TO, FROM_SITEMAP, WITH_PAYLOAD]
        .contains(&path)
        .then(|| Page {
            head: head(path, ""),
            paragraphs: 0,
        })
}

/// One answer of no paragraphs: a redirect, or a sitemap.
fn plain(status: u16, headers: Vec<(String, String)>, body: String) -> Sent {
    Sent {
        status,
        headers,
        body: Box::new(Chunks {
            head: Some(Bytes::from(body)),
            left: 0,
            tail: false,
            sent: Arc::new(AtomicU64::new(0)),
        }),
        reused: false,
        decoded: false,
        certificate_exception: None,
    }
}

impl Site {
    /// Where `path` redirects, on a site serving [`Frontier`]'s paths.
    fn moved(&self, path: &str) -> Option<&'static str> {
        self.frontier?;
        match path {
            MOVED if self.second_hop => Some(MOVED_ON),
            MOVED => Some(MOVED_TO),
            MOVED_ON if self.second_hop => Some(MOVED_TO),
            _ => None,
        }
    }
}

/// One extra unit of a row's Counted work, done once per page served with
/// that page's head, which `xtask counts` must fail on (ADR 0140).
pub(crate) type Plant = Arc<dyn Fn(&str) + Send + Sync>;

impl Transport for Site {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        let path = hop.url.path();
        self.requests.fetch_add(1, Ordering::Relaxed);
        if self.robots_txt && path == "/robots.txt" {
            let headers = vec![
                ("content-type".to_owned(), "text/plain".to_owned()),
                ("content-length".to_owned(), ROBOTS_TXT.len().to_string()),
            ];
            return Box::pin(std::future::ready(Ok(plain(
                200,
                headers,
                ROBOTS_TXT.to_owned(),
            ))));
        }
        let path = hop.url.path();
        if let Some(to) = self.moved(path) {
            self.hops.fetch_add(1, Ordering::Relaxed);
            let location = ("location".to_owned(), to.to_owned());
            return Box::pin(std::future::ready(Ok(plain(
                302,
                vec![location, ("content-length".to_owned(), "0".to_owned())],
                String::new(),
            ))));
        }
        if self.frontier.is_some() && path == SITEMAP {
            self.sitemaps.fetch_add(1, Ordering::Relaxed);
            let body = format!(
                "<?xml version=\"1.0\"?><urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>http://{HOST}{FROM_SITEMAP}</loc></url></urlset>"
            );
            let headers = vec![
                ("content-type".to_owned(), "application/xml".to_owned()),
                ("content-length".to_owned(), body.len().to_string()),
            ];
            return Box::pin(std::future::ready(Ok(plain(200, headers, body))));
        }
        // [`body_bytes`] holds the site's own pages, not [`Frontier`]'s.
        let page = page(path, self.linked);
        let sent = if page.is_some() {
            Arc::clone(&self.sent)
        } else {
            Arc::new(AtomicU64::new(0))
        };
        let page = page
            .or_else(|| self.frontier.and_then(|_| frontier_page(path)))
            .or_else(|| (path == ITEM).then(|| item_page(hop.url.query(), self.frontier?))?)
            .map(|page| Page {
                paragraphs: if self.small_pages { 0 } else { page.paragraphs },
                ..page
            })
            .map(|page| {
                self.attempts.fetch_add(1, Ordering::Relaxed);
                Page {
                    paragraphs: self
                        .largest
                        .map_or(page.paragraphs, |largest| page.paragraphs.min(largest)),
                    ..page
                }
            });
        if let (Some(plant), Some(page)) = (&self.plant, &page) {
            plant(&page.head);
        }
        Box::pin(async move {
            Ok(match page {
                Some(page) => Sent {
                    status: 200,
                    headers: vec![
                        (
                            "content-type".to_owned(),
                            "text/html; charset=utf-8".to_owned(),
                        ),
                        ("content-length".to_owned(), page.len().to_string()),
                    ],
                    body: Box::new(Chunks {
                        head: Some(Bytes::from(page.head)),
                        left: page.paragraphs * PARAGRAPH,
                        tail: true,
                        sent,
                    }),
                    reused: false,
                    decoded: false,
                    certificate_exception: None,
                },
                // robots.txt and anything else: no such page.
                None => Sent {
                    status: 404,
                    headers: vec![("content-length".to_owned(), "0".to_owned())],
                    body: Box::new(Chunks {
                        head: None,
                        left: 0,
                        tail: false,
                        sent,
                    }),
                    reused: false,
                    decoded: false,
                    certificate_exception: None,
                },
            })
        })
    }
}

/// A page's body as it arrives: its head, then its paragraphs in chunks of
/// at most the fixture's 16 KiB, then the tail.
struct Chunks {
    head: Option<Bytes>,
    /// The paragraphs' bytes still to send.
    left: usize,
    /// Whether the tail is still to send.
    tail: bool,
    sent: Arc<AtomicU64>,
}

impl Body for Chunks {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let chunk = if let Some(head) = self.head.take() {
            Some(head)
        } else if self.left > 0 {
            let length = self.left.min(PARAGRAPHS.len());
            self.left -= length;
            Some(Bytes::from_static(&PARAGRAPHS[..length]))
        } else if std::mem::take(&mut self.tail) {
            Some(Bytes::from_static(TAIL))
        } else {
            None
        };
        if let Some(chunk) = &chunk {
            self.sent.fetch_add(
                u64::try_from(chunk.len()).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
        }
        Box::pin(std::future::ready(Ok(chunk)))
    }
}

/// The plant of a Page analysis row `row` other than its parses, its work
/// done through its one counted function, or counted at the seam a run of
/// it crosses; none for any other row.
pub(crate) fn page_analysis_plant(row: Row, work: &CountedWork) -> Option<Plant> {
    let work = work.clone();
    let plant: Plant = match row {
        Row::PageSelectorCompilations => Arc::new(move |_: &str| {
            let _ = work.compile_selector(row, "h1");
        }),
        Row::PageFileOpens => Arc::new(move |_: &str| {
            let exe = std::env::current_exe().expect("invariant: a running executable has a path");
            work.open_file(row, OpenOptions::new().read(true), &exe)
                .expect("invariant: a running executable can be read");
        }),
        Row::PageReadabilityRuns
        | Row::PageLocaleFormBuilds
        | Row::PagePluginSerializations
        | Row::PagePluginAnswerParses
        | Row::PageTreeCopies
        | Row::PageShareReadabilityRuns
        | Row::PageBlockClassifications => Arc::new(move |_: &str| work.count(row)),
        _ => return None,
    };
    Some(plant)
}

/// The plant of a Run-trace row `row`: one extra folder or file opened
/// through its one counted function, or one task or formatting counted at
/// the seam it crosses; none for any other row.
pub(crate) fn run_trace_plant(row: Row, work: &CountedWork) -> Option<Plant> {
    let work = work.clone();
    let plant: Plant = match row {
        Row::RunTraceRunFileOpens | Row::RunTraceFileOpens => Arc::new(move |_: &str| {
            let exe = std::env::current_exe().expect("invariant: a running executable has a path");
            work.open_file(row, OpenOptions::new().read(true), &exe)
                .expect("invariant: a running executable can be read");
        }),
        Row::RunTraceRunSpawns | Row::RunTraceSpawns | Row::RunTraceFormattingOff => {
            Arc::new(move |_: &str| work.count(row))
        }
        _ => return None,
    };
    Some(plant)
}

/// What the crawl answered: each Record of `pages`, how many requests a
/// handler handled, the body bytes the site sent and the attempts it served
/// a page to.
pub(crate) struct Crawled {
    pub(crate) records: Vec<Value>,
    pub(crate) handled: u64,
    pub(crate) body_bytes: u64,
    /// The redirects the site answered.
    pub(crate) hops: u64,
    /// The sitemaps it sent.
    pub(crate) sitemaps: u64,
    pub(crate) attempts: u64,
    /// Every request the site was sent: its pages' and robots.txt's.
    pub(crate) requests: u64,
}

/// The entries the Run traces under `home` hold once its crawl ended: one
/// line each of every `.jsonl` file of a `trace` folder, read from the files,
/// so the count is the files' and not the writer's. Apart from the crawl, so
/// the heap benchmark does not measure it.
#[expect(
    clippy::disallowed_methods,
    reason = "the benchmark reads its Run trace once its engine stopped, on no async worker"
)]
pub(crate) fn entries(home: &Path) -> u64 {
    let mut folders = vec![home.to_owned()];
    let mut entries = 0;
    while let Some(folder) = folders.pop() {
        let Ok(children) = std::fs::read_dir(&folder) else {
            continue;
        };
        for child in children.flatten() {
            let path = child.path();
            if path.is_dir() {
                folders.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
                && folder.file_name().is_some_and(|name| name == "trace")
            {
                let lines = std::fs::read_to_string(&path)
                    .expect("invariant: the Run trace's file is readable")
                    .lines()
                    .count();
                entries += u64::try_from(lines).expect("invariant: a few lines");
            }
        }
    }
    entries
}

/// `hello`'s line.
fn hello() -> String {
    Hello::needing(Product::Foxhound)
        .id("hello")
        .surface(Surface::Rust)
        .with(|params| {
            params.product_version =
                ProductVersion::parse("1.4.0").expect("invariant: 1.4.0 is a Product version");
        })
        .message()
        .to_string()
}

/// Crawls the site from its index on a Foxhound engine whose fetch tiers
/// serve it from memory, on the Goodfetch home `home`, its paused clock
/// starting at `start` and its entropy seeded with `seed`, and answers
/// every Record of `pages` and the body bytes the site sent.
pub(crate) fn crawl(home: &Path, start: SystemTime, seed: u64) -> Crawled {
    crawl_with(home, start, seed, Options::default())
}

/// What a crawl runs with beyond the fixture: the counts cells' (ADR 0140).
#[derive(Default)]
pub(crate) struct Options {
    /// Where its engine records, among them its Counted work.
    pub(crate) metrics: Metrics,
    /// The settings the cell sets beside the crawl's own.
    pub(crate) settings: serde_json::Map<String, Value>,
    /// One extra unit of work per page served.
    pub(crate) plant: Option<Plant>,
    /// The fields the crawl reads beside `title`.
    pub(crate) fields: serde_json::Map<String, Value>,
    /// The formats the crawl's extraction asks for: none by default.
    pub(crate) formats: serde_json::Map<String, Value>,
    /// A handler serves each page, on its document tier, in place of the
    /// code-free extraction.
    pub(crate) handler: Option<Handler>,
    /// The Frontier's outcomes beyond a links batch.
    pub(crate) frontier: Option<Frontier>,
    /// Every page without its paragraphs: the same batches, hops and sitemaps
    /// in less time, for a run under Callgrind, which counts what admission
    /// does and not the size of a page.
    pub(crate) small_pages: bool,
    /// The VFS its run file opens through (ADR 0140's sync counts).
    pub(crate) run_state_vfs: Option<String>,
    /// The most paragraphs a page holds, for a count that does not depend on
    /// a page's size and a tool too slow for the whole site (Callgrind).
    pub(crate) largest_page: Option<usize>,
    /// The site serves a robots.txt, which the crawl parses and obeys (ADR
    /// 0140's E6), in place of answering it 404.
    pub(crate) robots_txt: bool,
    /// The amount of a Limit of `requests` on the install, which each
    /// request of the crawl charges (ADR 0140's E3).
    pub(crate) limit: Option<u64>,
    /// The VFS the fixture's own setup, such as its Limit, writes through,
    /// apart from the crawl's: a counting one counts the fixture's syncs, which
    /// are no row's, so `xtask counts --strace` sees each sync counted.
    pub(crate) setup_vfs: Option<String>,
    /// How many of [`PAGES`] the site links and serves, all of them when
    /// `None`: a shorter crawl of the same pages, whose difference from the
    /// whole one is what each page more costs (ADR 0140's `F2.prepares`).
    pub(crate) linked_pages: Option<usize>,
    /// What the crawl is sent once it started.
    pub(crate) after_start: AfterStart,
}

/// The handler that serves each page of a handled crawl, on its document
/// tier.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handler {
    /// Waits on the page's `h1` as attached, then as visible, which the
    /// tier answers from the page's own document, and answers no Record.
    Waits,
    /// The reference handler of `E1.handled_page_wire_share` (#2407): one
    /// `page.read` of [`REFERENCE_FIELDS`] fields, one `crawl_request.push`
    /// of what it read and one `crawl_request.enqueue_links` of the page's
    /// links, then an answer with no Record.
    Reference,
}

/// What a crawl is sent once it started.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum AfterStart {
    /// `run.finish` at once.
    #[default]
    Finish,
    /// [`BARRIER_CALLS`] calls that no page will make again, each a barrier
    /// at a level that batches (ADR 0142): a `key_value_store.set` outside a
    /// handler and a `run.add_seed` of its index, a duplicate. Its crawl is
    /// `crawl.keep_alive`, so it is still running when they come, and
    /// `run.finish` ends it once both are answered.
    BarrierCalls,
}

/// The calls a crawl sent [`AfterStart::BarrierCalls`] is made: the batches
/// they may each write are `F2.batches`' own term.
pub(crate) const BARRIER_CALLS: u64 = 2;

/// How the crawl meets the Frontier's outcomes beyond a links batch.
#[derive(Clone, Copy)]
pub(crate) struct Frontier {
    /// Whether the seed's Payload has a schema to fit.
    pub(crate) schema: bool,
    /// How many pages `/item?id=<n>` the whole site serves beside its own,
    /// the first a seed and each linking the next: URLs that differ by one
    /// parameter, each page its own, which the Frontier's learning reads
    /// its evidence for (`F2.evidence_reads`).
    pub(crate) items: usize,
    /// Whether the seed that redirects takes a second hop before its page,
    /// a chain of two, which the in-memory crawl's test counts hop by hop.
    pub(crate) second_hop: bool,
}

impl Frontier {
    /// The items a site linking `linked` of [`PAGES`] serves: as large a
    /// share of them as of its pages, so a shorter site's crawl has fewer
    /// of each.
    pub(crate) fn items_on(self, linked: usize) -> usize {
        self.items * linked / PAGES.len()
    }
}

/// A Limit of `amount` `requests` on the install, set in `home`'s ledger at
/// `start` through `vfs`, which each request of the crawl charges (ADR 0140's
/// E3).
fn set_limit(home: &Path, start: SystemTime, amount: u64, vfs: Option<String>) {
    Ledger::at(Home::new(home.to_owned()).usage_ledger(), vfs)
        .add_limit(
            &Limit {
                scope: LimitScope {
                    kind: INSTALL.to_owned(),
                    id: String::new(),
                },
                usage_kind: REQUESTS.to_owned(),
                currency: None,
                period: None,
                amount,
                must_understand: true,
                set_by: LimitSetter::Person,
            },
            Timestamp::try_from(start).expect("invariant: the fixture's start"),
        )
        .expect("invariant: a new home's ledger is writable");
}

/// Foxhound's engine on `tiers`, counting on `metrics`.
fn foxhound(tiers: FetchTiers, metrics: Metrics) -> Engine {
    let identity = Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("1.4.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    // Foxhound's composition, its tiers serving from memory.
    let composition = goodfetch_foxhound::composition_with(tiers);
    Engine::new(identity, &composition)
        .expect("invariant: one handler each")
        .with_metrics(metrics)
}

/// [`crawl`], with `options`.
pub(crate) fn crawl_with(home: &Path, start: SystemTime, seed: u64, options: Options) -> Crawled {
    let Options {
        metrics,
        settings,
        plant,
        fields,
        formats,
        handler,
        frontier,
        small_pages,
        run_state_vfs,
        largest_page,
        limit,
        robots_txt,
        setup_vfs,
        linked_pages,
        after_start,
    } = options;
    let mut settings = settings;
    if after_start == AfterStart::BarrierCalls {
        let crawl = settings.entry("crawl").or_insert_with(|| json!({}));
        crawl["keep_alive"] = json!(true);
    }
    let linked = linked_pages.unwrap_or(PAGES.len());
    if let Some(amount) = limit {
        set_limit(home, start, amount, setup_vfs);
    }
    let (sent, attempts) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let (hops, sitemaps) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let requests = Arc::new(AtomicU64::new(0));
    let site = Arc::new(Site {
        sent: Arc::clone(&sent),
        attempts: Arc::clone(&attempts),
        plant,
        frontier: frontier.map(|frontier| frontier.items_on(linked)),
        second_hop: frontier.is_some_and(|frontier| frontier.second_hop),
        small_pages,
        hops: Arc::clone(&hops),
        sitemaps: Arc::clone(&sitemaps),
        largest: largest_page,
        robots_txt,
        requests: Arc::clone(&requests),
        linked,
    });
    let (records, handled) = drive(
        site,
        &crawl_start(settings, (fields, formats), handler, frontier, linked),
        Driven {
            home,
            start,
            seed,
            metrics,
            run_state_vfs,
            handler,
            credits: CREDITS,
            after_start,
        },
    );
    Crawled {
        records,
        handled,
        body_bytes: sent.load(Ordering::Relaxed),
        hops: hops.load(Ordering::Relaxed),
        sitemaps: sitemaps.load(Ordering::Relaxed),
        attempts: attempts.load(Ordering::Relaxed),
        requests: requests.load(Ordering::Relaxed),
    }
}

/// What [`drive`] runs a crawl on: the Goodfetch home `home`, a paused
/// clock starting at `start`, entropy seeded with `seed`, `metrics`, where
/// its engine records, among them its Counted work, and the VFS its run
/// file opens through; the `handler` that serves its pages, if any, so it
/// keeps no Record of the code-free extraction; and the `credits` its read of the Records gives, at least one a
/// Record.
pub(crate) struct Driven<'a> {
    pub(crate) home: &'a Path,
    pub(crate) start: SystemTime,
    pub(crate) seed: u64,
    pub(crate) metrics: Metrics,
    pub(crate) run_state_vfs: Option<String>,
    pub(crate) handler: Option<Handler>,
    pub(crate) credits: u64,
    /// What the crawl is sent once it started.
    pub(crate) after_start: AfterStart,
}

/// Runs the crawl `crawl_start` starts as the Run `pages`, on a Foxhound
/// engine whose fetch tiers send each hop to `site`, and answers every
/// Record of its Dataset `pages` and how many requests a handler handled:
/// what each in-memory benchmark's crawl is driven by, the site and the
/// crawl its own (B1's above, B4's in `examples/counts-extraction.rs`).
pub(crate) fn drive(
    site: Arc<dyn Transport>,
    crawl_start: &Value,
    on: Driven<'_>,
) -> (Vec<Value>, u64) {
    let Driven {
        home,
        start,
        seed,
        metrics,
        run_state_vfs,
        handler,
        credits,
        after_start,
    } = on;
    let engine = foxhound(FetchTiers::serving(site), metrics);
    let credits = json!({
        "jsonrpc": "2.0", "method": "$/credit",
        "params": { "id": "records", "credits": credits },
    })
    .to_string();
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut lines = BufReader::new(reader).lines();
            send(&mut writer, &hello()).await;
            send(&mut writer, &encode(crawl_start)).await;
            let mut records = Vec::new();
            let mut handled = Handled::of(handler);
            let mut calls_out = 0;
            while let Some(line) = next_line(&mut lines).await {
                let message = decode(&line);
                if handled.read(&message, &mut writer).await {
                    continue;
                }
                if message["id"] == "start" && after_start == AfterStart::BarrierCalls {
                    assert!(message.get("result").is_some(), "{message}");
                    send(&mut writer, SET).await;
                    send(&mut writer, &add_seed()).await;
                    calls_out = BARRIER_CALLS;
                } else if message["id"] == "start" {
                    assert!(message.get("result").is_some(), "{message}");
                    send(&mut writer, FINISH).await;
                } else if message["id"] == "set" || message["id"] == "add" {
                    assert!(message.get("result").is_some(), "{message}");
                    calls_out -= 1;
                    if calls_out == 0 {
                        send(&mut writer, FINISH).await;
                    }
                } else if message["id"] == "finish" {
                    assert!(message.get("result").is_some(), "{message}");
                    if handler.is_some() {
                        break;
                    }
                    send(&mut writer, RECORDS).await;
                    send(&mut writer, &credits).await;
                } else if message["method"] == "$/record" {
                    records.push(message["params"]["record"]["value"].clone());
                } else if message["id"] == "records" {
                    assert!(message.get("result").is_some(), "{message}");
                    break;
                }
            }
            writer.shutdown().await.expect("invariant: in memory");
            while let Ok(Some(_)) = lines.next_line().await {}
            (records, handled.answered())
        };
        let (served, read) = tokio::join!(
            engine.run_attached(
                Network::InMemory(network),
                dependencies(home, start, seed, run_state_vfs)
            ),
            client
        );
        served.expect("invariant: the engine got its Connection");
        read
    })
}

/// `count` links of the site as a shop's pages write them: every fourth one a
/// spelling of the one before (tracking parameters, another order, a
/// fragment, a `;jsessionid=`), the rest distinct pages with encoded
/// characters, so the URL key's every rule runs and identities meet.
pub(crate) fn shop_links(count: usize) -> Vec<String> {
    (0..count)
        .map(|link| {
            let page = link / 4 * 3 + link % 4;
            match link % 4 {
                3 => format!(
                    "http://{HOST}/item/{};jsessionid=0A1B?utm_source=mail&size=m&colour=%7eblue#reviews",
                    page - 1
                ),
                2 => format!("http://{HOST}/item/{page}?colour=%c3%a9t%c3%a9&size=m&&gclid=x"),
                _ => format!("http://{HOST}/item/{page}?size=m&colour=%7eblue"),
            }
        })
        .collect()
}

/// Starts a code-free crawl from `urls`, each a seed, on the fixture's
/// engine, home, clock and entropy, and aborts it once its start answered:
/// the Run's start admits every seed through the crawl's Run state as it
/// makes the run file, which the `admit_seeds` benchmark counts. Answers
/// how many seeds it sent.
pub(crate) fn start_with_seeds(
    home: &Path,
    start: SystemTime,
    seed: u64,
    urls: &[String],
) -> usize {
    let tiers = FetchTiers::serving(Arc::new(Site {
        sent: Arc::default(),
        attempts: Arc::default(),
        plant: None,
        frontier: None,
        second_hop: false,
        small_pages: true,
        hops: Arc::default(),
        sitemaps: Arc::default(),
        largest: None,
        robots_txt: false,
        requests: Arc::default(),
        linked: 0,
    }));
    let engine = foxhound(tiers, Metrics::default());
    let seeds: Vec<Value> = urls
        .iter()
        .map(|url| json!({ "url": url, "label": "page" }))
        .collect();
    let started = json!({
        "jsonrpc": "2.0",
        "id": "start",
        "method": "crawl.start",
        "params": {
            "run_id": "pages",
            "seeds": seeds,
            "datasets": [],
            "extraction": {
                "formats": {},
                "follow": "scope",
                "fields": { "title": { "locator": { "kind": "css", "selector": "h1" } } },
            },
            "settings": {
                "fetch": { "tier": "http" },
                "politeness": { "domain": { "delay": 0 } },
                "network": { "resolve": { HOST: ["127.0.0.1"] } },
                "egress": { "private_ranges": [HOST] },
            },
        },
    });
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut lines = BufReader::new(reader).lines();
            send(&mut writer, &hello()).await;
            send(&mut writer, &started.to_string()).await;
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value =
                    serde_json::from_str(&line).expect("invariant: the engine writes JSON");
                if message["id"] == "start" {
                    assert!(message.get("result").is_some(), "{message}");
                    send(&mut writer, ABORT).await;
                } else if message["id"] == "abort" {
                    assert!(message.get("result").is_some(), "{message}");
                    break;
                }
            }
            writer.shutdown().await.expect("invariant: in memory");
            while let Ok(Some(_)) = lines.next_line().await {}
        };
        let (served, ()) = tokio::join!(
            engine.run_attached(
                Network::InMemory(network),
                dependencies(home, start, seed, None)
            ),
            client
        );
        served.expect("invariant: the engine got its Connection");
    });
    urls.len()
}

/// What the engine runs on: the Goodfetch home `home`, a paused clock from
/// `start`, entropy seeded with `seed`, blocking work inline and its run
/// files opened through `run_state_vfs`.
fn dependencies(
    home: &Path,
    start: SystemTime,
    seed: u64,
    run_state_vfs: Option<String>,
) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(home.to_owned()),
        clock: Clock::paused(start),
        entropy: Entropy::seeded(seed),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions {
            vfs: run_state_vfs,
            ..RunStateOptions::default()
        },
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// `run.finish`, which answers once the crawl's work items are done.
const FINISH: &str =
    r#"{"jsonrpc":"2.0","id":"finish","method":"run.finish","params":{"run_id":"pages"}}"#;

/// `run.abort`, which a Run that only had to start ends with.
const ABORT: &str =
    r#"{"jsonrpc":"2.0","id":"abort","method":"run.abort","params":{"run_id":"pages"}}"#;

/// `key_value_store.set` of `pages`, outside any handler: a call no page
/// will make again.
const SET: &str = r#"{"jsonrpc":"2.0","id":"set","method":"key_value_store.set","params":{"run_id":"pages","key":"cursor","value":3}}"#;

/// `run.add_seed` of the site's index to `pages`, outside any attempt: a
/// duplicate of its first seed, which adds no page.
fn add_seed() -> String {
    json!({
        "jsonrpc": "2.0", "id": "add", "method": "run.add_seed",
        "params": { "run_id": "pages", "seed": { "url": format!("http://{HOST}/"), "label": "page" } },
    })
    .to_string()
}

/// `run.records` of `pages`, once the Run ended: it answers once every
/// Record is sent.
const RECORDS: &str = r#"{"jsonrpc":"2.0","id":"records","method":"run.records","params":{"run_id":"pages","dataset":"pages"}}"#;

/// Enough credits for every Record of B1's site.
const CREDITS: u64 = 1000;

/// The code-free `crawl.start` from the site's index: every in-scope link
/// followed, each page's `h1` its Record's `title`, no Politeness delay,
/// and the site's host mapped to an address the egress policy lets
/// through, which no connection reaches. A cell's own settings, `cell`,
/// replace a key of these whole; its `fields` join `title`, and its
/// extraction asks for its `formats`.
fn crawl_start(
    cell: serde_json::Map<String, Value>,
    (mut fields, formats): (
        serde_json::Map<String, Value>,
        serde_json::Map<String, Value>,
    ),
    handler: Option<Handler>,
    frontier: Option<Frontier>,
    linked: usize,
) -> Value {
    let mut settings = serde_json::Map::from_iter([
        ("fetch".to_owned(), json!({ "tier": "http" })),
        ("politeness".to_owned(), json!({ "domain": { "delay": 0 } })),
        (
            "network".to_owned(),
            json!({ "resolve": { HOST: ["127.0.0.1"] } }),
        ),
        ("egress".to_owned(), json!({ "private_ranges": [HOST] })),
    ]);
    settings.extend(cell);
    let mut seeds = Vec::new();
    let mut labels = json!({});
    if let Some(frontier) = frontier {
        seeds.extend([
            json!({ "url": format!("http://{HOST}{MOVED}"), "label": "page" }),
            json!({
                "sitemap": format!("http://{HOST}{SITEMAP}"),
                "rules": [{ "pattern": "**", "label": "page" }],
            }),
            json!({
                "url": format!("http://{HOST}{WITH_PAYLOAD}"),
                "label": "typed",
                "payload": { "isbn": "1" },
            }),
        ]);
        if frontier.items_on(linked) > 0 {
            seeds.push(json!({ "url": format!("http://{HOST}{ITEM}?id=0"), "label": "page" }));
        }
        if frontier.schema {
            labels = json!({ "typed": { "payload": {
                "type": "object",
                "required": ["isbn"],
                "properties": { "isbn": { "type": "string" } },
            } } });
        }
    }
    if let Some(handler) = handler {
        settings.insert(
            "crawl".to_owned(),
            json!({ "handler": { "concurrency": 1 } }),
        );
        let pages: Vec<Value> = std::iter::once("/".to_owned())
            .chain((0..linked).map(|number| format!("/page-{number}")))
            .map(|path| json!({ "url": format!("http://{HOST}{path}"), "label": "page" }))
            .collect();
        seeds.splice(0..0, pages);
        labels["page"] = json!({});
        let mut params = json!({
            "run_id": "pages",
            "seeds": seeds,
            "labels": labels,
            "settings": settings,
        });
        // The reference handler pushes what it read into a Dataset of its
        // own.
        if handler == Handler::Reference {
            params["datasets"] = json!([{ "name": "pages" }]);
        }
        return json!({
            "jsonrpc": "2.0",
            "id": "start",
            "method": "crawl.start",
            "params": params,
        });
    }
    fields.insert(
        "title".to_owned(),
        json!({ "locator": { "kind": "css", "selector": "h1" } }),
    );
    seeds.insert(
        0,
        json!({ "url": format!("http://{HOST}/"), "label": "page" }),
    );
    json!({
        "jsonrpc": "2.0",
        "id": "start",
        "method": "crawl.start",
        "params": {
            "run_id": "pages",
            "seeds": seeds,
            "labels": labels,
            "datasets": [],
            "extraction": {
                "formats": formats,
                "follow": "scope",
                "fields": fields,
            },
            "settings": settings,
        },
    })
}

/// The handler a crawl's pages are served by, and what it holds between
/// its messages.
enum Handled {
    None,
    Waits(Waits),
    Reference(Reference),
}

impl Handled {
    fn of(handler: Option<Handler>) -> Self {
        match handler {
            None => Self::None,
            Some(Handler::Waits) => Self::Waits(Waits::default()),
            Some(Handler::Reference) => Self::Reference(Reference::default()),
        }
    }

    /// Whether `message` was the handler's, having done its next step.
    async fn read(&mut self, message: &Value, writer: &mut ConnectionWriter) -> bool {
        match self {
            Self::None => false,
            Self::Waits(waits) => waits.read(message, writer).await,
            Self::Reference(reference) => reference.read(message, writer).await,
        }
    }

    /// The requests it answered.
    fn answered(&self) -> u64 {
        match self {
            Self::None => 0,
            Self::Waits(waits) => waits.answered,
            Self::Reference(reference) => reference.answered,
        }
    }
}

/// The handler of a crawl: on each `crawl_request.handle`, waits on its page's
/// `h1` as attached, then as visible, then answers no Record.
#[derive(Default)]
struct Waits {
    /// Each request a wait is out for: its ID and its page's.
    waiting: BTreeMap<String, (Value, Value)>,
    answered: u64,
}

impl Waits {
    /// Whether `message` was the handler's, having done its next step.
    async fn read(&mut self, message: &Value, writer: &mut ConnectionWriter) -> bool {
        let id = message["id"].as_str().unwrap_or_default();
        if message["method"] == "crawl_request.handle" {
            let key = self.waiting.len().to_string();
            let page = message["params"]["page_id"].clone();
            send(
                writer,
                &wait_for(&format!("attached-{key}"), &page, "attached"),
            )
            .await;
            self.waiting.insert(key, (message["id"].clone(), page));
        } else if let Some(key) = id.strip_prefix("attached-") {
            assert!(message.get("result").is_some(), "{message}");
            let (_, page) = &self.waiting[key];
            send(
                writer,
                &wait_for(&format!("visible-{key}"), page, "visible"),
            )
            .await;
        } else if let Some(key) = id.strip_prefix("visible-") {
            assert!(message.get("result").is_some(), "{message}");
            let (request, _) = &self.waiting[key];
            let answer = json!({ "jsonrpc": "2.0", "id": request, "result": { "records": [] } });
            send(writer, &answer.to_string()).await;
            self.answered += 1;
        } else {
            return false;
        }
        true
    }
}

/// `locator.wait_for` on the served page `page`'s `h1`, reaching `state`.
fn wait_for(id: &str, page: &Value, state: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "locator.wait_for",
        "params": {
            "locator": { "page_id": page, "selector": "h1" },
            "state": state,
        },
    })
    .to_string()
}

/// The fields the reference handler reads of each page in one `page.read`.
pub(crate) const REFERENCE_FIELDS: usize = 20;

/// The paragraphs of the fixture's smallest page, each of which the
/// reference handler's fields read in turn: every page holds as many.
const READ_PARAGRAPHS: usize = PAGES[0];

/// The reference handler (#2407): on each `crawl_request.handle`, one
/// `page.read` of [`REFERENCE_FIELDS`] fields, then one `crawl_request.push`
/// of the values it read, then one `crawl_request.enqueue_links` of the
/// page's links, which the crawl's seeds already hold, then an answer with
/// no Record.
#[derive(Default)]
struct Reference {
    /// Each request a step is out for: the handle's params.
    handling: BTreeMap<String, Value>,
    answered: u64,
}

impl Reference {
    /// Whether `message` was the handler's, having done its next step.
    async fn read(&mut self, message: &Value, writer: &mut ConnectionWriter) -> bool {
        let id = message["id"].as_str().unwrap_or_default();
        if message["method"] == "crawl_request.handle" {
            let key = self.handling.len().to_string();
            let handle = &message["params"];
            let read = request(
                &format!("read-{key}"),
                "page.read",
                &json!({ "page_id": handle["page_id"], "fields": reference_fields() }),
            );
            send(writer, &encode(&read)).await;
            let mut handling = handle.clone();
            handling["request"] = message["id"].clone();
            self.handling.insert(key, handling);
        } else if let Some(key) = id.strip_prefix("read-") {
            let values = &message["result"]["values"];
            assert_eq!(
                values.as_object().map(serde_json::Map::len),
                Some(REFERENCE_FIELDS),
                "{message}"
            );
            let handle = &self.handling[key];
            let push = request(
                &format!("push-{key}"),
                "crawl_request.push",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "dataset": "pages",
                    "records": [values],
                }),
            );
            send(writer, &encode(&push)).await;
        } else if let Some(key) = id.strip_prefix("push-") {
            assert!(message.get("result").is_some(), "{message}");
            let handle = &self.handling[key];
            let enqueue = request(
                &format!("enqueue-{key}"),
                "crawl_request.enqueue_links",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "locator": { "page_id": handle["page_id"], "selector": "nav a" },
                    "label": "page",
                }),
            );
            send(writer, &encode(&enqueue)).await;
        } else if let Some(key) = id.strip_prefix("enqueue-") {
            assert!(message.get("result").is_some(), "{message}");
            let answer = json!({
                "jsonrpc": "2.0",
                "id": self.handling[key]["request"],
                "result": { "records": [] },
            });
            send(writer, &encode(&answer)).await;
            self.answered += 1;
        } else {
            return false;
        }
        true
    }
}

/// The request `method` with `params`, sent under `id`.
fn request(id: &str, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// The reference handler's [`REFERENCE_FIELDS`] fields: the page's title,
/// its heading, its first link's `href`, and its paragraphs' texts in turn
/// for the rest.
fn reference_fields() -> Value {
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });
    let mut fields = serde_json::Map::from_iter([
        ("title".to_owned(), json!({ "locator": css("title") })),
        ("heading".to_owned(), json!({ "locator": css("h1") })),
        (
            "link".to_owned(),
            json!({ "locator": css("nav a"), "attribute": "href" }),
        ),
    ]);
    for field in fields.len()..REFERENCE_FIELDS {
        let paragraph = field % READ_PARAGRAPHS + 1;
        fields.insert(
            format!("paragraph_{field}"),
            json!({ "locator": css(&format!("main > p:nth-of-type({paragraph})")) }),
        );
    }
    Value::Object(fields)
}

/// The client's wire, each step in a function of its own that Callgrind
/// names (`E1.handled_page_wire_share`): `message` encoded as its line.
#[inline(never)]
pub(crate) fn encode(message: &Value) -> String {
    message.to_string()
}

/// The message `line` holds, decoded.
#[inline(never)]
pub(crate) fn decode(line: &str) -> Value {
    serde_json::from_str(line).expect("invariant: the engine writes JSON")
}

/// The next line the engine wrote, `None` once it wrote its last.
async fn next_line(lines: &mut Lines<BufReader<ConnectionReader>>) -> Option<String> {
    Wire(pin!(lines.next_line())).await.ok().flatten()
}

/// Writes `line` and its newline.
async fn send(writer: &mut ConnectionWriter, line: &str) {
    Wire(pin!(async {
        writer
            .write_all(line.as_bytes())
            .await
            .expect("invariant: in memory");
        writer.write_all(b"\n").await.expect("invariant: in memory");
    }))
    .await;
}

/// The client reading or writing its Connection, `future`, each poll in a
/// function of its own that Callgrind names (`E1.handled_page_wire_share`).
struct Wire<'a, F>(Pin<&'a mut F>);

impl<F: Future> Future for Wire<'_, F> {
    type Output = F::Output;

    #[inline(never)]
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<F::Output> {
        self.0.as_mut().poll(context)
    }
}
