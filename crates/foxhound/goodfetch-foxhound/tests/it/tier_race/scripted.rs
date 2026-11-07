//! A tier race whose browser is scripted (spec #582; ADRs 0040 and 0093):
//! Foxhound's composition in process, its fetch tiers handed a
//! `BrowserPages` that answers browser-fetch and opens the race's
//! navigation each after the time its case sets on the tests' paused
//! clock, so a case puts the navigation's answer exactly where it needs it,
//! such as while browser-fetch's served page is still being judged, which
//! no real browser times. A program handler answers each request with no
//! Record, and on a page of the Label `home` first enqueues the page's
//! links under the Label `list`. Judged by what the scripted browser was
//! asked, what the Run answered, and whether the engine ended.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests run on the test Clock and Entropy (ADR 0059)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the scripted browser's one record of what it was asked, read once the Run ended: no order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::tls::ExceptionUsed;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{CallContext, CallError, Channel, Dependencies, Engine, Identity};
use goodfetch_fetch_tier::{Body, FetchTiers, Hop, PooledSession, Reuse, Sending, Sent, Transport};
use goodfetch_page::blocks::Block;
use goodfetch_page::page_value::{
    BrowserPages, EngineFetch, EngineFetched, FetchedBody, HandlerPages as _, Opening, PageValue,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{
    FormatsRequest, Navigated, PageFormats, PageId, PageResponse, SessionId,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_client::Hello;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};

/// The host the scripted site is served on.
const HOST: &str = "race.test";

/// How long the race's navigation takes to open, once it is sent.
const NAVIGATION: Duration = Duration::from_millis(300);

/// How the scripted site answers a path, on browser-fetch: its status,
/// its headers beside its type, its markup, and how long it takes.
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    html: String,
    after: Duration,
}

/// The scripted site: each path's answer, by path; any other path is a
/// 404 at once.
type Site = BTreeMap<&'static str, Answer>;

/// What the scripted browser was asked, in order.
#[derive(Debug, Default)]
struct Asked {
    /// Each path browser-fetch sent.
    fetched: Vec<String>,
    /// Each path a navigation opened, the race's or a browser run's.
    opened: Vec<String>,
}

/// The scripted browser.
struct Scripted {
    site: Site,
    asked: Mutex<Asked>,
    pages: Mutex<u64>,
}

impl Scripted {
    fn ask(&self, ask: impl FnOnce(&mut Asked)) {
        ask(&mut self.asked.lock().unwrap());
    }
}

/// `url`'s path.
fn path_of(url: &str) -> String {
    goodfetch_frontier::Url::parse(url)
        .map(|url| url.path().to_owned())
        .unwrap_or_default()
}

impl BrowserPages for Scripted {
    fn open<'a>(
        &'a self,
        _run_id: &'a RunId,
        _pooled: &'a PooledSession,
        url: &'a str,
        _context: &'a CallContext,
    ) -> Opening<'a, Result<(PageId, Navigated), CallError>> {
        Box::pin(async move {
            self.ask(|asked| asked.opened.push(path_of(url)));
            tokio::time::sleep(NAVIGATION).await;
            let number = {
                let mut pages = self.pages.lock().unwrap();
                *pages += 1;
                *pages
            };
            let response = PageResponse {
                url: url.to_owned(),
                status: 200,
                status_text: "OK".to_owned(),
                headers: BTreeMap::from([(
                    "content-type".to_owned(),
                    "text/html; charset=utf-8".to_owned(),
                )]),
                encoding: Some("UTF-8".to_owned()),
                request_id: format!("request-{number}"),
                from_service_worker: false,
            };
            Ok((
                PageId(format!("browser-{number}")),
                Navigated {
                    response: Some(response),
                },
            ))
        })
    }

    fn close<'a>(&'a self, _page_id: &'a PageId) -> Opening<'a, ()> {
        Box::pin(async {})
    }

    fn certificate_exception<'a>(
        &'a self,
        _page_id: &'a PageId,
        _host: &'a str,
    ) -> Opening<'a, Option<ExceptionUsed>> {
        Box::pin(async { None })
    }

    fn retire<'a>(
        &'a self,
        _run_id: &'a RunId,
        _pooled: &'a SessionId,
        _context: &'a CallContext,
    ) -> Opening<'a, Result<(), CallError>> {
        Box::pin(async { Ok(()) })
    }

    fn fetch<'a>(
        &'a self,
        _run_id: &'a RunId,
        _pooled: &'a PooledSession,
        _value: PageValue,
        request: &'a EngineFetch,
        _context: &'a CallContext,
    ) -> Opening<'a, Result<EngineFetched, CallError>> {
        Box::pin(async move {
            let path = path_of(&request.url);
            self.ask(|asked| asked.fetched.push(path.clone()));
            let Some(answer) = self.site.get(path.as_str()) else {
                return Ok(EngineFetched {
                    url: request.url.clone(),
                    status: 404,
                    headers: Vec::new(),
                    body: FetchedBody::Whole(Vec::new()),
                });
            };
            tokio::time::sleep(answer.after).await;
            let mut headers = vec![(
                "content-type".to_owned(),
                "text/html; charset=utf-8".to_owned(),
            )];
            headers.extend(answer.headers.iter().cloned());
            Ok(EngineFetched {
                url: request.url.clone(),
                status: answer.status,
                headers,
                body: FetchedBody::Whole(answer.html.clone().into_bytes()),
            })
        })
    }

    fn read<'a>(
        &'a self,
        _page_id: &'a PageId,
        _function: &'static str,
        _argument: Value,
        _operation: &'static str,
    ) -> Opening<'a, Result<Option<Value>, CallError>> {
        Box::pin(async { Ok(None) })
    }

    fn formats<'a>(
        &'a self,
        _page_id: &'a PageId,
        _request: FormatsRequest,
    ) -> Opening<'a, Result<PageFormats, CallError>> {
        Box::pin(async {
            Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                "The scripted browser reads no formats.",
            ))
        })
    }

    fn classify(&self, _response: &PageResponse, _markup: &str) -> Option<Block> {
        None
    }
}

/// The HTTP tier's site, for a Label forced onto `http`: the same pages,
/// at once.
struct Plain(Arc<Scripted>);

/// A body sent whole.
struct Whole(Option<Bytes>);

impl Body for Whole {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(std::future::ready(Ok(self.0.take())))
    }
}

impl Transport for Plain {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        let answer = self.0.site.get(hop.url.path());
        let (status, html) = answer.map_or((404, String::new()), |answer| {
            (answer.status, answer.html.clone())
        });
        Box::pin(async move {
            Ok(Sent {
                status,
                headers: vec![
                    (
                        "content-type".to_owned(),
                        "text/html; charset=utf-8".to_owned(),
                    ),
                    ("content-length".to_owned(), html.len().to_string()),
                ],
                body: Box::new(Whole(Some(Bytes::from(html)))),
                reused: false,
                decoded: false,
                certificate_exception: None,
            })
        })
    }
}

/// How a scripted crawl ended.
#[derive(Debug)]
pub(super) struct Ended {
    /// What the scripted browser was asked.
    fetched: Vec<String>,
    opened: Vec<String>,
    /// `run.finish`'s result, none when it never answered.
    finished: Option<Value>,
    /// Whether the engine ended once its Connection closed.
    exited: bool,
    /// The pages the fetch tiers still serve, by ID, once the Run ended.
    left_open: Vec<String>,
}

/// `hello`'s line.
fn hello() -> String {
    Hello::needing(Product::Foxhound)
        .id("hello")
        .surface(Surface::Rust)
        .with(|params| params.product_version = ProductVersion::parse("1.4.0").unwrap())
        .message()
        .to_string()
}

/// How long of the paused clock `run.finish`, and the engine's end after
/// its Connection closed, may take: far past any case's last answer.
const PATIENCE: Duration = Duration::from_secs(120);

/// Foxhound's engine on its composition with the tiers `tiers`.
fn engine(tiers: &FetchTiers) -> Engine {
    let identity = Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = goodfetch_foxhound::composition_with(tiers.clone());
    Engine::new(identity, &composition).unwrap()
}

/// The `crawl.start` of `seeds`, with `settings` over a racing Run's: one
/// sample per Label on a domain, one handler at a time, two requests at a
/// time on a domain with no delay, so a race's navigation has room beside
/// its browser-fetch, and the site's host mapped to an address the egress
/// policy lets through, which no connection reaches.
fn start(seeds: &[Value], settings: &Value) -> Value {
    let mut racing = json!({
        "fetch": { "tier_race": true, "sample": { "count": 1 } },
        "politeness": { "domain": { "concurrency": 2, "delay": 0 } },
        "crawl": { "handler": { "concurrency": 1 } },
        "network": { "resolve": { HOST: ["127.0.0.1"] } },
        "egress": { "private_ranges": [HOST] },
    });
    if let (Some(racing), Some(more)) = (racing.as_object_mut(), settings.as_object()) {
        racing.extend(more.clone());
    }
    json!({
        "jsonrpc": "2.0",
        "id": "start",
        "method": "crawl.start",
        "params": {
            "run_id": "race",
            "seeds": seeds,
            "labels": { "home": { "settings": { "fetch": { "tier": "http" } } }, "list": {} },
            "datasets": [{ "name": "pages" }],
            "settings": racing,
        },
    })
}

/// Crawls `seeds` on a Foxhound engine whose fetch tiers send from inside
/// the scripted browser serving `site`, with `settings` over a racing
/// Run's, then finishes the Run.
fn crawl(env: &TestEnv, site: Site, seeds: &[Value], settings: &Value) -> Ended {
    let scripted = Arc::new(Scripted {
        site,
        asked: Mutex::new(Asked::default()),
        pages: Mutex::new(0),
    });
    let tiers = FetchTiers::serving(Arc::new(Plain(Arc::clone(&scripted))));
    // The fetch tiers keep the first pages they are handed: Collie's, handed
    // as the engine is composed, are ignored.
    tiers.opened(Arc::clone(&scripted) as Arc<dyn BrowserPages>);
    let engine = engine(&tiers);
    let start = start(seeds, settings);
    let (finished, exited) = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(env.start_time());
        let _subscriber =
            tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || {
                clock.wall()
            }));
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await.unwrap().split();
            let mut lines = BufReader::new(reader).lines();
            send(&mut writer, &hello()).await;
            send(&mut writer, &start.to_string()).await;
            let finished = tokio::time::timeout(PATIENCE, talk(&mut lines, &mut writer))
                .await
                .ok()
                .flatten();
            writer.shutdown().await.unwrap();
            while let Ok(Some(_)) = lines.next_line().await {}
            finished
        };
        let dependencies = Dependencies {
            environment: BTreeMap::new(),
            keychain: goodfetch_engine::secrets::Keychain::absent(),
            home: Home::new(env.home().to_owned()),
            clock,
            entropy: Entropy::seeded(env.seed()),
            blocking: BlockingPolicy::Inline,
            run_state_location: RunStateLocation::PerMajor,
            run_state: RunStateOptions::default(),
            politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
        };
        let served = tokio::time::timeout(
            PATIENCE,
            engine.run_attached(Network::InMemory(network), dependencies),
        );
        let (served, finished) = tokio::join!(served, client);
        (finished, served.is_ok_and(|served| served.is_ok()))
    });
    let asked = std::mem::take(&mut *scripted.asked.lock().unwrap());
    let left_open = (1..=16)
        .map(|number| format!("document-{number}"))
        .filter(|page| tiers.response(&PageId(page.clone())).is_some())
        .collect();
    Ended {
        fetched: asked.fetched,
        opened: asked.opened,
        finished,
        exited,
        left_open,
    }
}

/// The Client's side, once `crawl.start` is sent: `run.finish` once the
/// crawl started, and each handler request answered with no Record, one of
/// the Label `home` once its page's links are enqueued under the Label
/// `list`. Answers `run.finish`'s result, none when the Connection closed
/// first.
async fn talk(
    received: &mut Lines<BufReader<ConnectionReader>>,
    writer: &mut ConnectionWriter,
) -> Option<Value> {
    let mut waiting: Option<Value> = None;
    while let Ok(Some(line)) = received.next_line().await {
        let message: Value = serde_json::from_str(&line).unwrap();
        if message["id"] == "start" {
            assert!(message.get("result").is_some(), "{message}");
            send(writer, r#"{"jsonrpc":"2.0","id":"finish","method":"run.finish","params":{"run_id":"race"}}"#).await;
        } else if message["id"] == "finish" {
            return Some(message["result"].clone());
        } else if message["id"] == "links" {
            let handle = waiting.take().unwrap();
            answer(writer, &handle).await;
        } else if message["method"] == "crawl_request.handle" {
            let params = &message["params"];
            if params["crawl_request"]["label"] == "home" {
                let enqueue = json!({
                    "jsonrpc": "2.0",
                    "id": "links",
                    "method": "crawl_request.enqueue_links",
                    "params": {
                        "run_id": params["run_id"],
                        "crawl_request_id": params["crawl_request"]["id"],
                        "lease_epoch": params["lease_epoch"],
                        "locator": { "page_id": params["page_id"], "selector": "a" },
                        "label": "list",
                    },
                });
                send(writer, &enqueue.to_string()).await;
                waiting = Some(message.clone());
            } else {
                answer(writer, &message).await;
            }
        }
    }
    None
}

/// Answers the handler request `handle` with no Record.
async fn answer(writer: &mut ConnectionWriter, handle: &Value) {
    let reply = json!({ "jsonrpc": "2.0", "id": handle["id"], "result": { "records": [] } });
    send(writer, &reply.to_string()).await;
}

/// Writes `line` and its newline.
async fn send(writer: &mut ConnectionWriter, line: &str) {
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.write_all(b"\n").await.unwrap();
}

/// A page of `html`, answered `after` its request.
fn page(html: &str, after: Duration) -> Answer {
    Answer {
        status: 200,
        headers: Vec::new(),
        html: format!(
            "<!doctype html><html><head><title>t</title></head><body>{html}</body></html>"
        ),
        after,
    }
}

/// The page the Label `list` first learns its tier on, by a sample.
const LEARN: &str = "/learn";

/// A seed of `label` at `path`.
fn seed(path: &str, label: &str) -> Value {
    json!({ "url": format!("http://{HOST}{path}"), "label": label })
}

/// The race's navigation answers while browser-fetch's served page is
/// still judged: here, while the RSL license its response names is
/// fetched, before the Run's AI-use signals are weighed. The navigation
/// wins, and browser-fetch's page, dropped with its future, is closed as
/// it drops: no served page is left, `run.finish` answers, and the engine
/// ends once its Connection closes. Today the page stays served, and holds
/// its Run's run-state writer open, so neither ever comes (verify-582).
///
/// Covers: story:6-263
#[test]
fn a_page_browser_fetch_served_is_closed_when_the_navigation_wins_while_it_is_judged() {
    let env = TestEnv::new();
    // Served before the navigation opens, at most the race's delay and its
    // opening after the start, then judged past it.
    let mut licensed = page("<p>a</p>", Duration::from_millis(100));
    licensed.headers.push((
        "link".to_owned(),
        r#"<http://race.test/licence.xml>; rel="license"; type="application/rsl+xml""#.to_owned(),
    ));
    let site = Site::from([
        (LEARN, page("<p>learn</p>", Duration::ZERO)),
        ("/", licensed),
        (
            "/licence.xml",
            Answer {
                status: 404,
                headers: Vec::new(),
                html: String::new(),
                after: Duration::from_millis(1_000),
            },
        ),
    ]);

    let ended = crawl(
        &env,
        site,
        &[seed(LEARN, "list"), seed("/", "list")],
        &json!({ "site_preferences": { "respect_use_signals": true, "purpose": "collect" } }),
    );

    assert_eq!(
        (
            ended.fetched.contains(&"/licence.xml".to_owned()),
            ended.opened.iter().filter(|path| *path == "/").count(),
            ended
                .finished
                .as_ref()
                .map(|report| report["lifecycle"].clone()),
            ended.exited,
            ended.left_open.clone(),
        ),
        (true, 1, Some(json!("finished")), true, Vec::<String>::new()),
        "{ended:?}"
    );
}

/// A request a hidden link reached, so marked as a suspected trap (ADR
/// 0035), races without its navigation: a browser tier never follows such
/// a link, so the race opens no browser page for it, while a request a
/// visible link reached opens its navigation beside browser-fetch. Each is
/// served slower than the race's delay, so a navigation, once started,
/// would open.
///
/// Covers: story:6-263
#[test]
fn a_race_opens_no_navigation_for_a_request_a_hidden_link_reached() {
    let env = TestEnv::new();
    let slow = Duration::from_millis(2_000);
    let site = Site::from([
        (LEARN, page("<p>learn</p>", Duration::ZERO)),
        (
            "/home",
            page(
                r#"<a href="/shown">shown</a><a href="/hidden" style="display:none">hidden</a>"#,
                Duration::ZERO,
            ),
        ),
        ("/shown", page("<p>shown</p>", slow)),
        ("/hidden", page("<p>hidden</p>", slow)),
    ]);

    let ended = crawl(
        &env,
        site,
        &[seed(LEARN, "list"), seed("/home", "home")],
        &json!({}),
    );

    assert_eq!(
        (
            ended.fetched.contains(&"/hidden".to_owned()),
            ended.opened.contains(&"/shown".to_owned()),
            ended.opened.contains(&"/hidden".to_owned()),
            ended
                .finished
                .as_ref()
                .map(|report| report["lifecycle"].clone()),
        ),
        (true, true, false, Some(json!("finished"))),
        "{ended:?}"
    );
}
