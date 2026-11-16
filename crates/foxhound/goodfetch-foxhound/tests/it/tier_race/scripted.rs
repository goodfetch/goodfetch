//! A tier race whose browser is scripted (spec #582; ADRs 0040 and 0093):
//! Foxhound's composition in process, its fetch tiers handed a
//! `BrowserPages` that answers browser-fetch and opens the race's
//! navigation each after the time its case sets on the tests' paused
//! clock, so a case puts the navigation's answer exactly where it needs it,
//! such as while browser-fetch's served page is still being judged, which
//! no real browser times. A program handler answers each request with no
//! Record, and on a page of the Label `home` first enqueues the page's
//! links under the Label `list`. Judged by what the scripted browser was
//! asked, what the Run answered, and whether the engine ended. The
//! scripted browser opens a Session from any `session.source`, so a pool
//! that holds a login crawls here as no installed engine yet lets it
//! (#2418).
#![expect(
    clippy::disallowed_methods,
    reason = "the tests run on the test Clock and Entropy (ADR 0059)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the scripted browser's one record of what it was asked, read once the Run ended: no order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::runs::Run;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::tls::ExceptionUsed;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{CallContext, CallError, Channel, Dependencies, Engine, Identity};
use goodfetch_fetch_tier::{
    Body, BuiltIn, FetchTiers, Hop, PooledSession, Reuse, Sending, Sent, Transport,
};
use goodfetch_page::blocks::Block;
use goodfetch_page::document_request::{DocumentRequest, Unread};
use goodfetch_page::page_value::{
    BrowserPages, EngineFetch, EngineFetched, FetchedBody, HandlerPages as _, Opening, PageValue,
    ServedFormatsAhead,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::foxhound::report::BROWSER_VERSION_UNREAD;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{
    FormatsRequest, Navigated, PageFormats, PageId, PageResponse, SessionId,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_client::Hello;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};

/// The host the scripted site is served on.
const HOST: &str = "race.test";

/// How long the race's navigation takes to open, once it is sent.
const NAVIGATION: Duration = Duration::from_millis(300);

/// How the scripted site answers a path, on browser-fetch and on a
/// Session's own transport: its status, its headers beside its type (on
/// browser-fetch), its markup, and how long it takes.
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
    /// Each path a Session's own transport sent: a forced `http` page,
    /// or a site file (#2422).
    sent: Vec<String>,
    /// Each path a navigation opened, the race's or a browser run's.
    opened: Vec<String>,
    /// Each path browser-fetch sent or a navigation opened in a Session
    /// that opened from a `session.source`.
    logged_in: Vec<String>,
    /// Each path browser-fetch sent or a navigation opened, with the
    /// Session it was asked of.
    in_session: Vec<(String, SessionId)>,
    /// How many times the document request was asked for.
    document_requests: usize,
    /// How many times a crawl started reading what its served documents'
    /// formats take beside their markup (#2573).
    prepared: usize,
}

/// The scripted browser.
struct Scripted {
    site: Site,
    /// Why its document request was not read, as an installed browser
    /// whose version could not be; none where it was.
    unread: Option<Unread>,
    asked: Mutex<Asked>,
    pages: Mutex<u64>,
}

impl Scripted {
    fn ask(&self, ask: impl FnOnce(&mut Asked)) {
        ask(&mut self.asked.lock().unwrap());
    }
}

/// Whether `pooled` opened from a `session.source`.
fn logged_in(pooled: &PooledSession) -> bool {
    pooled.source.as_ref().is_some_and(Option::is_some)
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
        pooled: &'a PooledSession,
        url: &'a str,
        _context: &'a CallContext,
    ) -> Opening<'a, Result<(PageId, Navigated), CallError>> {
        Box::pin(async move {
            self.ask(|asked| {
                asked.opened.push(path_of(url));
                asked.in_session.push((path_of(url), pooled.id.clone()));
                if logged_in(pooled) {
                    asked.logged_in.push(path_of(url));
                }
            });
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

    fn document_request<'a>(
        &'a self,
        _run: &'a Run,
    ) -> Opening<'a, Result<Option<DocumentRequest>, Unread>> {
        self.ask(|asked| asked.document_requests += 1);
        let read = self.unread.clone().map_or(Ok(None), Err);
        Box::pin(std::future::ready(read))
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
        pooled: &'a PooledSession,
        _value: PageValue,
        request: &'a EngineFetch,
        _context: &'a CallContext,
    ) -> Opening<'a, Result<EngineFetched, CallError>> {
        Box::pin(async move {
            let path = path_of(&request.url);
            self.ask(|asked| {
                asked.fetched.push(path.clone());
                asked.in_session.push((path.clone(), pooled.id.clone()));
                if logged_in(pooled) {
                    asked.logged_in.push(path.clone());
                }
            });
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

    fn served_formats<'a>(
        &'a self,
        _run: &'a Run,
        _request: FormatsRequest,
        _label: Option<BuiltIn>,
    ) -> Opening<'a, Option<Arc<dyn ServedFormatsAhead>>> {
        Box::pin(std::future::ready(None))
    }

    fn prepare_served_formats(&self, _run: &Run) {
        self.ask(|asked| asked.prepared += 1);
    }

    fn classify(&self, _response: &PageResponse, _markup: &str) -> Option<Block> {
        None
    }
}

/// The HTTP tier's site, for a Label forced onto `http` and the site
/// files every tier reads from a Session's own transport (#2422): the same
/// pages, after the same time.
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
        let path = hop.url.path();
        self.0.ask(|asked| asked.sent.push(path.to_owned()));
        let answer = self.0.site.get(path);
        let (status, html, after) = answer.map_or((404, String::new(), Duration::ZERO), |answer| {
            (answer.status, answer.html.clone(), answer.after)
        });
        Box::pin(async move {
            tokio::time::sleep(after).await;
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
    /// What the scripted browser and the transport were asked.
    fetched: Vec<String>,
    sent: Vec<String>,
    opened: Vec<String>,
    logged_in: Vec<String>,
    in_session: Vec<(String, SessionId)>,
    document_requests: usize,
    prepared: usize,
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

/// The Labels a crawl declares unless its case says: `home`, forced onto
/// `http`, and `list`.
fn labels() -> Value {
    json!({ "home": { "settings": { "fetch": { "tier": "http" } } }, "list": {} })
}

/// The `crawl.start` of `seeds` of `labels`, with `settings` over a racing Run's: one
/// sample per Label on a domain, one handler at a time, two requests at a
/// time on a domain with no delay, so a race's navigation has room beside
/// its browser-fetch, and the site's host mapped to an address the egress
/// policy lets through, which no connection reaches.
fn start((seeds, labels): (&[Value], &Value), settings: &Value) -> Value {
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
            "labels": labels,
            "datasets": [{ "name": "pages" }],
            "settings": racing,
        },
    })
}

/// Crawls `seeds` on a Foxhound engine whose fetch tiers send from inside
/// the scripted browser serving `site`, with `settings` over a racing
/// Run's, then finishes the Run.
fn crawl(env: &TestEnv, site: Site, seeds: &[Value], settings: &Value) -> Ended {
    crawl_unread(env, site, None, (seeds, &labels()), settings)
}

/// As [`crawl`], the scripted browser's document request not read for
/// `unread`, where some, and the seeds of `seeding` of its Labels.
fn crawl_unread(
    env: &TestEnv,
    site: Site,
    unread: Option<Unread>,
    seeding: (&[Value], &Value),
    settings: &Value,
) -> Ended {
    crawl_started(env, site, unread, &start(seeding, settings))
}

/// As [`crawl_unread`], the Run started by `start`, a `crawl.start` of the
/// Run `race`.
fn crawl_started(env: &TestEnv, site: Site, unread: Option<Unread>, start: &Value) -> Ended {
    let scripted = Arc::new(Scripted {
        site,
        unread,
        asked: Mutex::new(Asked::default()),
        pages: Mutex::new(0),
    });
    let tiers = FetchTiers::serving(Arc::new(Plain(Arc::clone(&scripted))));
    // The fetch tiers keep the first pages they are handed: Collie's, handed
    // as the engine is composed, are ignored.
    tiers.opened(Arc::clone(&scripted) as Arc<dyn BrowserPages>);
    let engine = engine(&tiers);
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
        .filter(|page| tiers.facts(&PageId(page.clone())).is_some())
        .collect();
    Ended {
        fetched: asked.fetched,
        sent: asked.sent,
        opened: asked.opened,
        logged_in: asked.logged_in,
        in_session: asked.in_session,
        document_requests: asked.document_requests,
        prepared: asked.prepared,
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
/// fetched, from a Session's own transport since #2422, before the Run's
/// AI-use signals are weighed. The navigation
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
            ended.sent.contains(&"/licence.xml".to_owned()),
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

/// A site of two pages a crawl forced onto `http` fetches.
fn forced_site() -> Site {
    Site::from([
        ("/one", page("<p>one</p>", Duration::ZERO)),
        ("/two", page("<p>two</p>", Duration::ZERO)),
    ])
}

/// The settings of a crawl forced onto `http`, with no tier race.
fn forced() -> Value {
    json!({ "fetch": { "tier": "http" } })
}

/// A crawl forced onto `http` asks its browser for the document request
/// its Sessions' own transport presents, once for the Run, and for nothing
/// else: no navigation opens and nothing is sent from inside the browser,
/// so no browser is launched (issue 2532).
///
/// Covers: story:5-121
#[test]
fn a_crawl_forced_onto_http_asks_its_browser_for_its_document_request_alone() {
    let env = TestEnv::new();

    let ended = crawl(
        &env,
        forced_site(),
        &[seed("/one", "list"), seed("/two", "list")],
        &forced(),
    );

    assert_eq!(
        (
            ended.document_requests,
            ended.opened.clone(),
            ended.fetched.clone(),
            ended
                .finished
                .as_ref()
                .map(|report| report["lifecycle"].clone()),
        ),
        (1, Vec::new(), Vec::new(), Some(json!("finished"))),
        "{ended:?}"
    );
}

/// The `crawl.start` of the code-free crawl `race` of `seeds` reading
/// `extraction`, forced onto `http` with no delay, the site's host mapped
/// as a racing Run's is.
fn code_free(seeds: &[Value], extraction: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "start",
        "method": "crawl.start",
        "params": {
            "run_id": "race",
            "seeds": seeds,
            "datasets": [],
            "extraction": extraction,
            "settings": {
                "fetch": { "tier": "http" },
                "politeness": { "domain": { "delay": 0 } },
                "network": { "resolve": { HOST: ["127.0.0.1"] } },
                "egress": { "private_ranges": [HOST] },
            },
        },
    })
}

/// A crawl starts reading what its served documents' formats take beside
/// their markup, their Filter lists' cosmetic rules, as it starts when its
/// pages may read a format: a program's, whose handler may read any, and a
/// code-free crawl's that reads one. A code-free crawl that reads none
/// never does, so no page of it loads its lists (#2573). The scripted
/// browser reads no formats, so a code-free crawl's pages fail here; its
/// Run still finishes.
#[rstest]
#[case::a_programs(None, 1)]
#[case::a_code_free_one_reading_markdown(Some(json!({ "formats": { "markdown": true }, "follow": "none" })), 1)]
#[case::a_code_free_one_reading_none(Some(json!({ "formats": {}, "follow": "none" })), 0)]
fn a_crawl_starts_reading_its_filter_lists_only_when_its_pages_may_read_a_format(
    #[case] extraction: Option<Value>,
    #[case] prepared: usize,
) {
    let env = TestEnv::new();
    let start = extraction.map_or_else(
        || {
            let seeds = [seed("/one", "list"), seed("/two", "list")];
            start((&seeds, &labels()), &forced())
        },
        |extraction| code_free(&[seed("/one", "page"), seed("/two", "page")], &extraction),
    );

    let ended = crawl_started(&env, forced_site(), None, &start);

    assert_eq!(
        (
            ended.prepared,
            ended
                .finished
                .as_ref()
                .map(|report| report["lifecycle"].clone()),
        ),
        (prepared, Some(json!("finished"))),
        "{ended:?}"
    );
}

/// Where the installed browser's version was not read, the Run is warned
/// once with `browser_version_unread`, its message why and the fix, and a
/// crawl forced onto `http`, which needs no browser, still crawls to its
/// end (issue 2532).
///
/// Covers: story:5-121
#[test]
fn a_run_whose_browsers_version_was_not_read_is_warned_and_still_crawls() {
    let env = TestEnv::new();
    let unread = Unread::NoVersion {
        executable: PathBuf::from("/usr/bin/chromium"),
    };

    let ended = crawl_unread(
        &env,
        forced_site(),
        Some(unread.clone()),
        (&[seed("/one", "list"), seed("/two", "list")], &labels()),
        &forced(),
    );

    let report = ended.finished.clone().unwrap_or_default();
    let warned: Vec<Value> = report["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|warning| warning["code"] == BROWSER_VERSION_UNREAD)
        .map(|warning| json!([warning["code"], warning["message"]]))
        .collect();
    assert_eq!(
        (warned, report["lifecycle"].clone()),
        (
            vec![json!([BROWSER_VERSION_UNREAD, unread.to_string()])],
            json!("finished")
        ),
        "{ended:?}"
    );
}

/// A pool whose Sessions open from a Saved identity sends its HTTP from
/// the browser under `fetch.http_from: learned` (#2418): its Label's
/// samples compare `browser_fetch` with the browser, so the site gets none
/// of its pages from a Session's own transport, every one of them asked of
/// a Session that holds the login, and it ends on `browser_fetch`. In the
/// same crawl, the Label `list`, on the Run's pool, which holds none, takes
/// the ladder: its sample sends its `http` run from the transport, and its
/// next page is sent from the transport too, on `http`. The tiers are read
/// from what the site was asked: `/account/1`, the sample, is opened and fetched from the
/// browser, and `/account/2` only fetched from it, on `browser_fetch`.
///
/// Covers: setting:fetch.http_from, setting:session.source
#[test]
fn a_login_pool_sends_from_the_browser_while_the_runs_pool_takes_the_ladder() {
    let env = TestEnv::new();
    let site = Site::from([
        (LEARN, page("<p>learn</p>", Duration::ZERO)),
        ("/list/2", page("<p>list</p>", Duration::ZERO)),
        ("/account/1", page("<p>orders</p>", Duration::ZERO)),
        ("/account/2", page("<p>invoices</p>", Duration::ZERO)),
    ]);
    let shop = json!({ "kind": "saved_identity", "name": "shop" });

    let ended = crawl_unread(
        &env,
        site,
        None,
        (
            &[
                seed(LEARN, "list"),
                seed("/list/2", "list"),
                seed("/account/1", "account"),
                seed("/account/2", "account"),
            ],
            &json!({ "list": {}, "account": { "settings": { "session": { "source": shop } } } }),
        ),
        &json!({
            "fetch": { "http_from": "learned", "sample": { "count": 1 } },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
        }),
    );

    let account = |paths: &[String]| -> BTreeSet<String> {
        paths
            .iter()
            .filter(|path| path.starts_with("/account/"))
            .cloned()
            .collect()
    };
    let both = BTreeSet::from(["/account/1".to_owned(), "/account/2".to_owned()]);
    assert_eq!(
        (
            account(&ended.sent),
            [LEARN, "/list/2"].map(|path| ended.sent.contains(&path.to_owned())),
            account(&ended.fetched),
            account(&ended.opened),
            account(&ended.logged_in),
            ended
                .logged_in
                .iter()
                .all(|path| path.starts_with("/account/")),
            ended
                .finished
                .as_ref()
                .map(|report| report["lifecycle"].clone()),
        ),
        (
            BTreeSet::new(),
            [true, true],
            both.clone(),
            BTreeSet::from(["/account/1".to_owned()]),
            both,
            true,
            Some(json!("finished")),
        ),
        "{ended:?}"
    );
}

/// A race of a Label learned onto `browser_fetch` sends its first request
/// from the hop page of the Session its navigation opens in (#2421): under
/// `fetch.http_from: learned`, the Label `account`, whose pool holds a
/// login, learns `browser_fetch` from its sample on `/account/1`, and the
/// race on `/account/2` asks its document of the browser, never of a
/// Session's own transport, then opens its navigation, started the race's
/// delay later while that document is still on its way, in the same
/// Session.
///
/// Covers: setting:fetch.tier_race, setting:fetch.http_from, setting:session.source
#[test]
fn a_race_of_a_label_on_browser_fetch_sends_its_first_request_from_the_navigations_session() {
    let env = TestEnv::new();
    let site = Site::from([
        ("/account/1", page("<p>orders</p>", Duration::ZERO)),
        (
            "/account/2",
            page("<p>invoices</p>", Duration::from_millis(600)),
        ),
    ]);
    let shop = json!({ "kind": "saved_identity", "name": "shop" });

    let ended = crawl_unread(
        &env,
        site,
        None,
        (
            &[seed("/account/1", "account"), seed("/account/2", "account")],
            &json!({ "account": { "settings": { "session": { "source": shop } } } }),
        ),
        &json!({
            "fetch": { "http_from": "learned", "tier_race": true, "sample": { "count": 1 } },
        }),
    );

    let raced: BTreeSet<&SessionId> = ended
        .in_session
        .iter()
        .filter(|(path, _)| path == "/account/2")
        .map(|(_, session)| session)
        .collect();
    let account_2 = "/account/2".to_owned();
    assert_eq!(
        (
            ended.sent.contains(&account_2),
            ended.fetched.contains(&account_2),
            ended.opened.contains(&account_2),
            raced.len(),
            ended
                .finished
                .as_ref()
                .map(|report| report["lifecycle"].clone()),
        ),
        (false, true, true, 1, Some(json!("finished"))),
        "{ended:?}"
    );
}
