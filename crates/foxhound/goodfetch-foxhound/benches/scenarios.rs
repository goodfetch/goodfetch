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

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use bytes::Bytes;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::ledger::Ledger;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_fetch_tier::{Body, FetchTiers, Hop, Reuse, Sending, Sent, Transport};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::{LimitScope, LimitSetter};
use goodfetch_protocol::usage::{INSTALL, Limit, REQUESTS};
use jiff::Timestamp;
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

/// What the crawl answered: each Record of `pages`, and the body bytes the
/// site sent.
pub(crate) struct Crawled {
    pub(crate) records: Vec<Value>,
    pub(crate) body_bytes: u64,
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
    /// The amount of a Limit of `requests` on the install, which each
    /// request of the crawl charges (ADR 0140's E3).
    pub(crate) limit: Option<u64>,
}

/// [`crawl`], with `options`.
pub(crate) fn crawl_with(home: &Path, start: SystemTime, seed: u64, options: Options) -> Crawled {
    let Options {
        metrics,
        settings,
        plant,
        limit,
    } = options;
    if let Some(amount) = limit {
        Ledger::at(Home::new(home.to_owned()).usage_ledger(), None)
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
    let records = goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut lines = BufReader::new(reader).lines();
            send(&mut writer, HELLO).await;
            send(&mut writer, &crawl_start(settings).to_string()).await;
            let mut records = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value =
                    serde_json::from_str(&line).expect("invariant: the engine writes JSON");
                if message["id"] == "start" {
                    assert!(message.get("result").is_some(), "{message}");
                    send(&mut writer, FINISH).await;
                } else if message["id"] == "finish" {
                    assert!(message.get("result").is_some(), "{message}");
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
            records
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
        let (served, records) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        served.expect("invariant: the engine got its Connection");
        records
    });
    Crawled {
        records,
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
fn crawl_start(mut settings: serde_json::Map<String, Value>) -> Value {
    settings.extend([
        ("fetch".to_owned(), json!({ "tier": "http" })),
        ("politeness".to_owned(), json!({ "domain": { "delay": 0 } })),
        (
            "network".to_owned(),
            json!({ "resolve": { HOST: ["127.0.0.1"] } }),
        ),
        ("egress".to_owned(), json!({ "private_ranges": [HOST] })),
    ]);
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
                "fields": { "title": { "locator": { "kind": "css", "selector": "h1" } } },
            },
            "settings": settings,
        },
    })
}

/// Writes `line` and its newline.
async fn send(writer: &mut ConnectionWriter, line: &str) {
    writer
        .write_all(line.as_bytes())
        .await
        .expect("invariant: in memory");
    writer.write_all(b"\n").await.expect("invariant: in memory");
}
