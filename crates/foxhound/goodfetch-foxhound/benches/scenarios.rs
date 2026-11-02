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
#![expect(
    clippy::disallowed_types,
    reason = "the site's one counter of the bytes it sent, read once the crawl ended: no lock and no order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_fetch_tier::{Body, FetchTiers, Hop, Reuse, Sending, Sent, Transport};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

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

/// One page of the site: its head and how many paragraphs follow it.
struct Page {
    head: String,
    paragraphs: usize,
}

/// The page at `path`, `None` for a path the site does not serve.
fn page(path: &str) -> Option<Page> {
    if path == "/" {
        let mut links = String::new();
        for number in 0..PAGES.len() {
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
    let paragraphs = *PAGES.get(number)?;
    let next = (number + 1) % PAGES.len();
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
        .filter_map(|path| page(&path))
        .map(|page| u64::try_from(page.len()).unwrap_or(u64::MAX))
        .sum()
}

/// The site, served from memory, counting the body bytes it sends.
struct Site {
    sent: Arc<AtomicU64>,
    /// Run once per page it serves: the benchmark's own extra unit of work.
    plant: Option<Plant>,
}

/// One extra unit of a row's Counted work, done once per page served with
/// that page's head, which `xtask counts` must fail on (ADR 0140).
pub(crate) type Plant = Arc<dyn Fn(&str) + Send + Sync>;

impl Transport for Site {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        let sent = Arc::clone(&self.sent);
        let page = page(hop.url.path());
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
        | Row::PagePluginAnswerParses => Arc::new(move |_: &str| work.count(row)),
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
        Row::RunTraceFileOpens => Arc::new(move |_: &str| {
            let exe = std::env::current_exe().expect("invariant: a running executable has a path");
            work.open_file(row, OpenOptions::new().read(true), &exe)
                .expect("invariant: a running executable can be read");
        }),
        Row::RunTraceSpawns | Row::RunTraceFormattingOff => {
            Arc::new(move |_: &str| work.count(row))
        }
        _ => return None,
    };
    Some(plant)
}

/// What the crawl answered: each Record of `pages`, how many requests a
/// handler handled, and the body bytes the site sent.
pub(crate) struct Crawled {
    pub(crate) records: Vec<Value>,
    pub(crate) handled: u64,
    pub(crate) body_bytes: u64,
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
const HELLO: &str = r#"{"jsonrpc":"2.0","id":"hello","method":"hello","params":{"protocol_version":"0.1","surface":"rust","needs":"foxhound","product_version":"1.4.0"}}"#;

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
    /// A handler serves each page, on its document tier, in place of the
    /// code-free extraction: it waits on the page's `h1` as attached, then as
    /// visible, which the tier answers from the page's own document.
    pub(crate) handler: bool,
}

/// [`crawl`], with `options`.
pub(crate) fn crawl_with(home: &Path, start: SystemTime, seed: u64, options: Options) -> Crawled {
    let Options {
        metrics,
        settings,
        plant,
        fields,
        handler,
    } = options;
    let sent = Arc::new(AtomicU64::new(0));
    let tiers = FetchTiers::serving(Arc::new(Site {
        sent: Arc::clone(&sent),
        plant,
    }));
    let identity = Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("1.4.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    // Foxhound's composition (`composition::composition`), its tiers
    // serving from memory: Collie's, whose page crate reads a served
    // page's formats, then the crawl's crates.
    let composition = goodfetch_collie::composition_with(tiers.clone())
        .with(goodfetch_frontier::contribution())
        .with(goodfetch_session_pool::contribution())
        .with(goodfetch_dataset::contribution())
        .with(goodfetch_crawl::contribution(tiers));
    let engine = Engine::new(identity, &composition)
        .expect("invariant: one handler each")
        .with_metrics(metrics);
    let (records, handled) = goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut lines = BufReader::new(reader).lines();
            send(&mut writer, HELLO).await;
            send(
                &mut writer,
                &crawl_start(settings, fields, handler).to_string(),
            )
            .await;
            let mut records = Vec::new();
            let mut waits = Waits::default();
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value =
                    serde_json::from_str(&line).expect("invariant: the engine writes JSON");
                if waits.read(&message, &mut writer).await {
                    continue;
                }
                if message["id"] == "start" {
                    assert!(message.get("result").is_some(), "{message}");
                    send(&mut writer, FINISH).await;
                } else if message["id"] == "finish" {
                    assert!(message.get("result").is_some(), "{message}");
                    if handler {
                        break;
                    }
                    send(&mut writer, RECORDS).await;
                    send(&mut writer, CREDITS).await;
                } else if message["method"] == "$/record" {
                    records.push(message["params"]["record"]["value"].clone());
                } else if message["id"] == "records" {
                    assert!(message.get("result").is_some(), "{message}");
                    break;
                }
            }
            writer.shutdown().await.expect("invariant: in memory");
            while let Ok(Some(_)) = lines.next_line().await {}
            (records, waits.answered)
        };
        let dependencies = Dependencies {
            environment: std::collections::BTreeMap::new(),
            keychain: goodfetch_engine::secrets::Keychain::absent(),
            home: Home::new(home.to_owned()),
            clock: Clock::paused(start),
            entropy: Entropy::seeded(seed),
            blocking: BlockingPolicy::Inline,
            run_state_location: RunStateLocation::PerMajor,
            run_state: RunStateOptions::default(),
            politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
        };
        let (served, read) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        served.expect("invariant: the engine got its Connection");
        read
    });
    Crawled {
        records,
        handled,
        body_bytes: sent.load(Ordering::Relaxed),
    }
}

/// `run.finish`, which answers once the crawl's work items are done.
const FINISH: &str =
    r#"{"jsonrpc":"2.0","id":"finish","method":"run.finish","params":{"run_id":"pages"}}"#;

/// `run.records` of `pages`, once the Run ended: it answers once every
/// Record is sent.
const RECORDS: &str = r#"{"jsonrpc":"2.0","id":"records","method":"run.records","params":{"run_id":"pages","dataset":"pages"}}"#;

/// Enough credits for every Record.
const CREDITS: &str =
    r#"{"jsonrpc":"2.0","method":"$/credit","params":{"id":"records","credits":1000}}"#;

/// The code-free `crawl.start` from the site's index: every in-scope link
/// followed, each page's `h1` its Record's `title`, no Politeness delay,
/// and the site's host mapped to an address the egress policy lets
/// through, which no connection reaches.
fn crawl_start(
    mut settings: serde_json::Map<String, Value>,
    mut fields: serde_json::Map<String, Value>,
    handler: bool,
) -> Value {
    settings.extend([
        ("fetch".to_owned(), json!({ "tier": "http" })),
        ("politeness".to_owned(), json!({ "domain": { "delay": 0 } })),
        (
            "network".to_owned(),
            json!({ "resolve": { HOST: ["127.0.0.1"] } }),
        ),
        ("egress".to_owned(), json!({ "private_ranges": [HOST] })),
    ]);
    if handler {
        settings.insert(
            "crawl".to_owned(),
            json!({ "handler": { "concurrency": 1 } }),
        );
        let seeds: Vec<Value> = std::iter::once("/".to_owned())
            .chain((0..PAGES.len()).map(|number| format!("/page-{number}")))
            .map(|path| json!({ "url": format!("http://{HOST}{path}"), "label": "page" }))
            .collect();
        return json!({
            "jsonrpc": "2.0",
            "id": "start",
            "method": "crawl.start",
            "params": {
                "run_id": "pages",
                "seeds": seeds,
                "labels": { "page": {} },
                "settings": settings,
            },
        });
    }
    fields.insert(
        "title".to_owned(),
        json!({ "locator": { "kind": "css", "selector": "h1" } }),
    );
    json!({
        "jsonrpc": "2.0",
        "id": "start",
        "method": "crawl.start",
        "params": {
            "run_id": "pages",
            "seeds": [{ "url": format!("http://{HOST}/"), "label": "page" }],
            "datasets": [],
            "extraction": {
                "formats": {},
                "follow": "scope",
                "fields": fields,
            },
            "settings": settings,
        },
    })
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

/// Writes `line` and its newline.
async fn send(writer: &mut ConnectionWriter, line: &str) {
    writer
        .write_all(line.as_bytes())
        .await
        .expect("invariant: in memory");
    writer.write_all(b"\n").await.expect("invariant: in memory");
}
