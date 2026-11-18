//! A crawl through the engine's public interface over the in-memory network
//! (spec #213's Seam 3), on the test `Clock` and `Entropy` with blocking
//! work inline: `crawl.start` with seeds, each work item delivered to a
//! scripted handler as a `crawl_request.handle` request, its outputs
//! committed once in the Run's own file, failures and lost leases retried
//! under the Run's policy, and `run.finish` and `run.abort` answering the
//! Run report's three axes. What the tests read of the run file and the Run
//! index is what the engine wrote.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059); its SQL statements are its own, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod admission;
mod finishing;
mod given_back;
mod keep_alive;
mod labels;
mod limits;
mod lost_outcome;
mod masking;
mod metrics;
mod model;
mod needs_you;
mod panics;
mod progress;
mod report;
mod resume;
mod safe_point;
mod sent_from;
mod staging;
mod verdict;
mod wal;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::time::Instant;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::policy::CrawlPolicy;

/// The policy of the tests' crawls: a one-second handler, so a lost lease
/// ends after 11 s of the paused clock.
pub(crate) const POLICY: CrawlPolicy = CrawlPolicy {
    handler_timeout: Duration::from_secs(1),
    retry_max: 2,
    retry_handler_failed: false,
    retry_engine_panicked: true,
    handler_concurrency: 4,
};

/// What the scripted handler does with one attempt.
#[derive(Clone, Debug)]
pub(crate) enum Reply {
    /// Answers these Records into the Dataset `pages`.
    Records(Vec<Value>),
    /// Fails with this Error code and retryable flag.
    Fails(&'static str, bool),
    /// Fails with this Error code, retryable flag and details.
    FailsWith(&'static str, bool, Value),
    /// Skips its request with this message, as `ctx.skip` does.
    Skips(&'static str),
    /// Fails with `blocked` and this Block cause, as a page the site
    /// refused raises it.
    Blocked(&'static str),
    /// Never answers.
    Silent,
    /// Answers as the inner reply says once this much of the paused clock
    /// passed, as a handler that took that long.
    After(Duration, Box<Reply>),
    /// Sends these calls to the engine from inside the attempt, each
    /// method with its params, waiting for each reply, which
    /// [`Client::replies`] keeps, then answers as the inner reply says.
    Calls(Vec<(&'static str, Value)>, Box<Reply>),
}

/// The Client's end of the engine's Connection.
pub(crate) struct Client {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
    next_id: i64,
    /// The handler requests the engine sent, as they came.
    pub(crate) handled: Vec<Value>,
    /// The ids of the requests left unanswered.
    pub(crate) silent: Vec<Value>,
    /// Each request and notification the engine sent, as it came, with
    /// the paused clock's reading when it did.
    pub(crate) arrivals: Vec<(Value, Instant)>,
    /// The Records a request left unanswered is answered with once the
    /// engine cancels it: a late commit.
    pub(crate) late: Option<Vec<Value>>,
    /// The replies to the calls a handler sent from inside its attempts,
    /// in order.
    pub(crate) replies: Vec<Value>,
    /// What came while a handler's call from inside its attempt waited for
    /// its reply, read next.
    early: std::collections::VecDeque<Value>,
}

impl Client {
    pub(crate) async fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).await.unwrap();
    }

    async fn next(&mut self) -> Option<Value> {
        if let Some(early) = self.early.pop_front() {
            return Some(early);
        }
        self.read().await
    }

    /// The next message off the Connection, past what came early.
    async fn read(&mut self) -> Option<Value> {
        let line = self.lines.next_line().await.unwrap()?;
        Some(serde_json::from_str(&line).unwrap())
    }

    /// The reply a handler request gets once `reply`'s calls were sent,
    /// each waiting for its own reply, and its waits waited.
    async fn answering(&mut self, mut reply: Reply) -> Reply {
        loop {
            reply = match reply {
                Reply::Calls(calls, then) => {
                    for (method, params) in calls {
                        let inside = format!("inside-{}", self.replies.len());
                        self.send(&json!({ "jsonrpc": "2.0", "id": inside, "method": method, "params": params }))
                            .await;
                        loop {
                            let message = self
                                .read()
                                .await
                                .expect("the engine closed before replying");
                            if message["id"] == json!(inside) {
                                self.replies.push(message);
                                break;
                            }
                            self.early.push_back(message);
                        }
                    }
                    *then
                }
                Reply::After(took, then) => {
                    tokio::time::sleep(took).await;
                    *then
                }
                reply => return reply,
            };
        }
    }

    /// Sends the call `method` and answers the engine's handler requests
    /// with `handler` until the call's reply arrives, which it answers. A
    /// `crawl.start` that declares no Dataset declares `pages`, which the
    /// harness pushes every Record into.
    pub(crate) async fn call(
        &mut self,
        method: &str,
        params: Value,
        handler: &mut impl FnMut(&Value) -> Reply,
    ) -> Value {
        let params = as_sent(method, params);
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let message = self
                .next()
                .await
                .expect("the engine closed before replying");
            if message.get("method").is_some() {
                self.arrivals.push((message.clone(), Instant::now()));
            }
            if message.get("method").and_then(Value::as_str) == Some("$/cancel") {
                let cancelled = &message["params"]["id"];
                if let Some(records) = self.late.clone()
                    && self.silent.contains(cancelled)
                {
                    let records: Vec<Value> = records
                        .into_iter()
                        .map(|data| json!({ "dataset": "pages", "data": data }))
                        .collect();
                    self.send(&json!({ "jsonrpc": "2.0", "id": cancelled, "result": { "records": records } }))
                        .await;
                }
            } else if message.get("method").and_then(Value::as_str) == Some("crawl_request.handle")
            {
                self.handled.push(message["params"].clone());
                let reply = self.answering(handler(&message["params"])).await;
                match reply {
                    Reply::Records(records) => {
                        let records: Vec<Value> = records
                            .into_iter()
                            .map(|data| json!({ "dataset": "pages", "data": data }))
                            .collect();
                        self.send(&json!({ "jsonrpc": "2.0", "id": message["id"], "result": { "records": records } }))
                            .await;
                    }
                    Reply::Fails(code, retryable) => {
                        self.send(&json!({
                            "jsonrpc": "2.0",
                            "id": message["id"],
                            "error": { "code": -32003, "message": "the handler failed", "data": { "code": code, "retryable": retryable } },
                        }))
                        .await;
                    }
                    Reply::FailsWith(code, retryable, details) => {
                        let mut data = json!({ "code": code, "retryable": retryable });
                        if let (Some(data), Value::Object(details)) =
                            (data.as_object_mut(), details)
                        {
                            data.extend(details);
                        }
                        self.send(&json!({
                            "jsonrpc": "2.0",
                            "id": message["id"],
                            "error": { "code": -32005, "message": "the handler failed", "data": data },
                        }))
                        .await;
                    }
                    Reply::Skips(text) => {
                        self.send(&json!({ "jsonrpc": "2.0", "id": message["id"], "result": { "skipped": { "message": text } } }))
                            .await;
                    }
                    Reply::Blocked(cause) => {
                        self.send(&json!({
                            "jsonrpc": "2.0",
                            "id": message["id"],
                            "error": { "code": -32003, "message": "the site refused the page", "data": { "code": "blocked", "retryable": false, "cause": cause } },
                        }))
                        .await;
                    }
                    Reply::Silent => self.silent.push(message["id"].clone()),
                    Reply::Calls(..) | Reply::After(..) => {
                        unreachable!("invariant: the calls were sent and the waits waited above")
                    }
                }
            } else if message["id"] == json!(id) {
                return message;
            }
        }
    }

    pub(crate) async fn close(&mut self) {
        self.writer.shutdown().await.unwrap();
        while self.next().await.is_some() {}
    }
}

/// Writes `policy` as the user configuration of the test's home, the way a
/// test sets short durations (ADR 0059).
pub(crate) fn configure(env: &TestEnv, policy: CrawlPolicy) {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("configuration.toml"),
        format!(
            "version = \"0.1\"\n\n[settings]\ntimeout.handler = {}\ncrawl.retry.max = {}\ncrawl.retry.handler_failed = {}\ncrawl.retry.engine_panicked = {}\ncrawl.handler.concurrency = {}\n",
            policy.handler_timeout.as_millis(),
            policy.retry_max,
            policy.retry_handler_failed,
            policy.retry_engine_panicked,
            policy.handler_concurrency,
        ),
    )
    .unwrap();
}

/// What the tests' engine runs on: the test's home, configured with
/// [`POLICY`], the test `Clock` and `Entropy`, and blocking work inline.
/// Made on the runtime the engine runs on, as the paused `Clock` must be.
pub(crate) fn dependencies(env: &TestEnv, run_state: RunStateOptions) -> Dependencies {
    configure(env, POLICY);
    default_dependencies(env, run_state)
}

/// The same, with the settings' defaults: a 60 s handler, so a 70 s lease.
pub(crate) fn default_dependencies(env: &TestEnv, run_state: RunStateOptions) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::paused(env.start_time()),
        entropy: Entropy::seeded(env.seed()),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state,
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// The address the harness maps each seed's host to: a public one, which
/// the egress policy lets through with nothing weakened, and which no
/// connection reaches, since the engine's fetch tiers serve from memory
/// ([`engine`]).
const SITE_ADDRESS: &str = "93.184.216.34";

thread_local! {
    /// The site settings the test's last `crawl.start` went out with, which
    /// its later resumes go out with too: a resume's settings are the
    /// Run's from then on, so it names the forced tier again.
    static SITE_SETTINGS: std::cell::RefCell<Option<(Value, Value)>> =
        const { std::cell::RefCell::new(None) };
}

/// `params` of a call of `method` as the harness sends it. A `crawl.start`
/// that declares no Dataset declares `pages`, which the harness pushes
/// every Record into; one that sets no `fetch.tier` forces `http`, the one
/// tier an engine with no browser pages serves every request on; and one
/// that sets no `network.resolve` maps each seed's host to
/// [`SITE_ADDRESS`], so no name is looked up. A `run.resume` or a
/// `run.retry_failed` that sets neither goes out with the last
/// `crawl.start`'s.
pub(crate) fn as_sent(method: &str, mut params: Value) -> Value {
    match method {
        "crawl.start" => {}
        "run.resume" | "run.retry_failed" => {
            if let Some((tier, resolve)) = SITE_SETTINGS.with_borrow(Clone::clone) {
                if params["settings"]["fetch"].get("tier").is_none() {
                    params["settings"]["fetch"]["tier"] = tier;
                }
                if params["settings"]["network"].get("resolve").is_none() {
                    params["settings"]["network"]["resolve"] = resolve;
                }
            }
            return params;
        }
        _ => return params,
    }
    if params.get("datasets").is_none() {
        params["datasets"] = json!([{ "name": "pages" }]);
    }
    if params["settings"]["fetch"].get("tier").is_none() {
        params["settings"]["fetch"]["tier"] = json!("http");
    }
    if params["settings"]["network"].get("resolve").is_none() {
        let hosts: serde_json::Map<String, Value> = params["seeds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|seed| seed["url"].as_str().or_else(|| seed["sitemap"].as_str()))
            .filter_map(|url| url.split("://").nth(1)?.split(['/', ':', '?', '#']).next())
            .map(|host| (host.to_owned(), json!([SITE_ADDRESS])))
            .collect();
        params["settings"]["network"]["resolve"] = Value::Object(hosts);
    }
    SITE_SETTINGS.set(Some((
        params["settings"]["fetch"]["tier"].clone(),
        params["settings"]["network"]["resolve"].clone(),
    )));
    params
}

/// The path of the one sitemap [`Site`] serves, which lists
/// the first three pages [`seeds`] names.
pub(crate) const SITEMAP: &str = "/sitemap.xml";

/// The path whose fetch panics, inside the attempt that fetches it, as a
/// bug in the engine would (issue 2285).
pub(crate) const PANICS: &str = "/panics";

/// The site every crawl of the harness fetches from memory, through the
/// fetch tiers' test seam: every URL but [`SITEMAP`] and [`PANICS`]
/// answers one small document, and no site has a robots.txt.
struct Site;

/// A body sent whole in one chunk.
struct Whole(Option<bytes::Bytes>);

impl goodfetch_fetch_tier::Body for Whole {
    fn chunk(&mut self) -> goodfetch_fetch_tier::Sending<'_, Option<bytes::Bytes>> {
        Box::pin(std::future::ready(Ok(self.0.take())))
    }
}

impl goodfetch_fetch_tier::Transport for Site {
    fn send<'a>(
        &'a self,
        hop: &'a goodfetch_fetch_tier::Hop,
        _reuse: goodfetch_fetch_tier::Reuse,
    ) -> goodfetch_fetch_tier::Sending<'a, goodfetch_fetch_tier::Sent> {
        let (status, content_type, body) = match hop.url.path() {
            PANICS => panic!("the test's site panics in the attempt that fetches {PANICS}"),
            "/robots.txt" => (404, "text/html; charset=utf-8", ""),
            SITEMAP => (200, "application/xml", SITEMAP_XML),
            _ => (
                200,
                "text/html; charset=utf-8",
                "<!doctype html><html><head><title>Page</title></head><body><h1>Page</h1></body></html>",
            ),
        };
        Box::pin(std::future::ready(Ok(goodfetch_fetch_tier::Sent {
            status,
            headers: vec![
                ("content-type".to_owned(), content_type.to_owned()),
                ("content-length".to_owned(), body.len().to_string()),
            ],
            body: Box::new(Whole(Some(bytes::Bytes::from_static(body.as_bytes())))),
            reused: false,
            decoded: false,
            certificate_exception: None,
        })))
    }
}

/// The sitemap at [`SITEMAP`]: the first three pages [`seeds`] names.
const SITEMAP_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#,
    "<url><loc>https://shop.test/page/1</loc></url>",
    "<url><loc>https://shop.test/page/2</loc></url>",
    "<url><loc>https://shop.test/page/3</loc></url>",
    "</urlset>",
);

/// The fetch tiers of the harness's engine, serving [`Site`] from memory.
pub(crate) fn tiers() -> goodfetch_fetch_tier::FetchTiers {
    goodfetch_fetch_tier::FetchTiers::serving(std::sync::Arc::new(Site))
}

/// A Foxhound engine composed of the engine's operations, the Frontier's
/// per-domain queues, the Session pool's and the Datasets' tables and the
/// crawl's, fetching [`Site`] from memory.
pub(crate) fn engine() -> Engine {
    let identity = Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_frontier::contribution())
        .with(goodfetch_session_pool::contribution())
        .with(goodfetch_dataset::contribution(crate::url_of))
        .with(super::contribution(tiers()));
    Engine::new(identity, &composition).unwrap()
}

/// Runs the engine on the `Dependencies` that `dependencies` makes on the
/// runtime while `talk` drives its Client from `hello` on, then closes the
/// Connection. The test `tracing` subscriber, timed by their `Clock`, writes
/// the dispatch records into the test's exercise record (ADR 0072).
pub(crate) fn crawl<T>(
    dependencies: impl FnOnce() -> Dependencies,
    talk: impl AsyncFnOnce(&mut Client) -> T,
) -> T {
    crawl_fenced(dependencies, talk).0
}

/// [`crawl`], which also answers the fencing model's transitions `lease`
/// traced, each as its fields, in order.
pub(crate) fn crawl_fenced<T>(
    dependencies: impl FnOnce() -> Dependencies,
    talk: impl AsyncFnOnce(&mut Client) -> T,
) -> (T, Vec<Value>) {
    crawl_on(&engine(), dependencies, talk)
}

/// [`crawl_fenced`], on `engine`.
pub(crate) fn crawl_on<T>(
    engine: &Engine,
    dependencies: impl FnOnce() -> Dependencies,
    talk: impl AsyncFnOnce(&mut Client) -> T,
) -> (T, Vec<Value>) {
    let (layer, transitions) = Transitions::new();
    let answer = goodfetch_test_support::runtime().block_on(async {
        let dependencies = dependencies();
        let clock = dependencies.clock;
        let _subscriber = tracing::subscriber::set_default(
            goodfetch_test_support::subscriber(move || clock.wall()).with(layer),
        );
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
                next_id: 0,
                handled: Vec::new(),
                silent: Vec::new(),
                arrivals: Vec::new(),
                late: None,
                replies: Vec::new(),
                early: std::collections::VecDeque::new(),
            };
            let hello = client
                .call(
                    "hello",
                    json!({
                        "protocol_version": "0.1",
                        "surface": "rust",
                        "needs": "foxhound",
                        "product_version": "1.4.0",
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            assert_eq!(hello["result"]["product"], "foxhound");
            let answer = talk(&mut client).await;
            client.close().await;
            answer
        };
        let (served, answer) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        served.unwrap();
        answer
    });
    (answer, transitions.try_iter().collect())
}

/// The `tracing` layer that keeps each event under `lease`'s target as a
/// map of its fields.
pub(crate) struct Transitions(mpsc::Sender<Value>);

impl Transitions {
    /// The layer, and where its events go.
    pub(crate) fn new() -> (Self, mpsc::Receiver<Value>) {
        let (sender, receiver) = mpsc::channel();
        (Self(sender), receiver)
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Transitions {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != crate::lease::TARGET {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let _ = self.0.send(Value::Object(fields.0));
    }
}

#[derive(Default)]
struct Fields(serde_json::Map<String, Value>);

impl tracing::field::Visit for Fields {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_owned(), json!(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), json!(value));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_owned(), json!(format!("{value:?}")));
    }
}

/// The transitions `transitions` holds for the lease of epoch `epoch`, as
/// `transition` and, when one is given, `reason`.
pub(crate) fn of_epoch(transitions: &[Value], epoch: u64) -> Vec<String> {
    transitions
        .iter()
        .filter(|transition| transition["lease_epoch"] == json!(epoch))
        .map(|transition| match transition["reason"].as_str() {
            Some(reason) => format!("{} {reason}", transition["transition"].as_str().unwrap()),
            None => transition["transition"].as_str().unwrap().to_owned(),
        })
        .collect()
}

pub(crate) fn seeds(count: usize) -> Value {
    let seeds: Vec<Value> = (1..=count)
        .map(|page| json!({ "url": format!("https://shop.test/page/{page}"), "label": "page" }))
        .collect();
    Value::Array(seeds)
}

/// The run file of `run_id` in the test's home, as the engine left it.
pub(crate) fn run_file(env: &TestEnv, run_id: &str) -> Connection {
    Connection::open(run_folder(env, run_id).join("run.sqlite")).unwrap()
}

pub(crate) fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home().join("1").join("runs").join(run_id)
}

/// Each committed Record's Dataset, number, work item and data, in order.
pub(crate) fn records(connection: &Connection) -> Vec<(String, i64, String, Value)> {
    let mut statement = connection
        .prepare("SELECT dataset, number, work_item, data FROM records ORDER BY dataset, number")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                serde_json::from_str(&row.get::<_, String>(3)?).unwrap(),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Each work item's ID, status, attempts and last Error code, in order.
pub(crate) fn work_items(connection: &Connection) -> Vec<(String, String, i64, Option<String>)> {
    let mut statement = connection
        .prepare("SELECT id, status, attempts, error_code FROM work_items ORDER BY position")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

pub(super) fn page_of(params: &Value) -> String {
    params["crawl_request"]["url"].as_str().unwrap().to_owned()
}

/// Each delivered work item's page and attempt, in order.
pub(super) fn delivered(client: &Client) -> Vec<(String, u64)> {
    client
        .handled
        .iter()
        .map(|params| (page_of(params), params["attempt"].as_u64().unwrap()))
        .collect()
}

/// Answers each page with one Record naming it.
pub(super) fn answer_all(params: &Value) -> Reply {
    Reply::Records(vec![json!({ "page": page_of(params) })])
}

/// Answers the first seed's page with its Record, leaving every other
/// page unanswered.
pub(super) fn first_only(params: &Value) -> Reply {
    if page_of(params).ends_with("/1") {
        answer_all(params)
    } else {
        Reply::Silent
    }
}

/// Starts the crawl of `start`, whose first seed's page alone is answered,
/// and pauses its Run once that page's Record is committed, resuming it
/// until then; answers the pause that held.
pub(super) async fn paused_after_its_first_page(
    client: &mut Client,
    env: &TestEnv,
    start: Value,
) -> Value {
    let run_id = start["run_id"].as_str().unwrap().to_owned();
    client.call("crawl.start", start, &mut first_only).await;
    loop {
        let paused = client
            .call("run.pause", json!({ "run_id": run_id }), &mut first_only)
            .await;
        if records(&run_file(env, &run_id)).len() == 1 {
            return paused;
        }
        client
            .call("run.resume", json!({ "run_id": run_id }), &mut first_only)
            .await;
    }
}

/// Covers: story:3-41, operation:crawl.start, operation:run.finish
#[test]
fn a_crawls_seeds_reach_the_handler_and_their_records_commit_once() {
    let env = TestEnv::new();

    let (started, report) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({ "run_id": "shop", "seeds": seeds(3), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler =
                |params: &Value| Reply::Records(vec![json!({ "page": page_of(params) })]);
            let report = client
                .call("run.finish", json!({ "run_id": "shop" }), &mut handler)
                .await;
            (started, report)
        },
    );

    let file = run_file(&env, "shop");
    assert_eq!(
        (
            &started["result"],
            (
                &report["result"]["lifecycle"],
                &report["result"]["verdict"],
                &report["result"]["completeness"]
            ),
            records(&file),
        ),
        (
            &json!({ "run_id": "shop", "started_as": "new", "lifecycle": "running", "needs": "foxhound" }),
            (&json!("finished"), &json!("verified"), &json!("complete")),
            vec![
                (
                    "pages".to_owned(),
                    1,
                    "1".to_owned(),
                    json!({ "page": "https://shop.test/page/1" })
                ),
                (
                    "pages".to_owned(),
                    2,
                    "2".to_owned(),
                    json!({ "page": "https://shop.test/page/2" })
                ),
                (
                    "pages".to_owned(),
                    3,
                    "3".to_owned(),
                    json!({ "page": "https://shop.test/page/3" })
                ),
            ],
        )
    );
}

/// Covers: story:3-41
#[test]
fn a_handler_failure_that_is_retryable_is_tried_again_and_commits_once() {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "retry", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| match params["attempt"].as_u64() {
                Some(1) => Reply::Fails("retry_requested", true),
                _ => Reply::Records(vec![json!({ "attempt": "second" })]),
            };
            client
                .call("run.finish", json!({ "run_id": "retry" }), &mut handler)
                .await
        },
    );

    let file = run_file(&env, "retry");
    assert_eq!(
        (records(&file), work_items(&file)),
        (
            vec![(
                "pages".to_owned(),
                1,
                "1".to_owned(),
                json!({ "attempt": "second" })
            )],
            vec![("1".to_owned(), "succeeded".to_owned(), 2, None)],
        )
    );
}

/// A handler's own failure is a bug that fails the same way each time, so
/// it is not retried by default (ADR 0093).
///
/// Covers: error:handler_failed
#[test]
fn a_handlers_own_failure_commits_nothing_and_is_not_retried() {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "bug", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| Reply::Fails("handler_failed", true);
            client
                .call("run.finish", json!({ "run_id": "bug" }), &mut handler)
                .await
        },
    );

    let file = run_file(&env, "bug");
    assert_eq!(
        (records(&file), work_items(&file)),
        (
            Vec::new(),
            vec![(
                "1".to_owned(),
                "failed".to_owned(),
                1,
                Some("handler_failed".to_owned())
            )],
        )
    );
}

/// A handler that never answers loses its lease; the work item is tried
/// again, and the late answer of the lost attempt commits nothing.
///
/// Covers: story:3-41
#[test]
fn a_handler_that_never_answers_loses_its_lease_and_its_late_answer_commits_nothing() {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "hung", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| match params["attempt"].as_u64() {
                Some(1) => Reply::Silent,
                _ => Reply::Records(vec![json!({ "answer": "on time" })]),
            };
            let started = client.call("run.finish", json!({ "run_id": "hung" }), &mut handler);
            let report = started.await;
            let late = client.silent[0].clone();
            client
            .send(&json!({ "jsonrpc": "2.0", "id": late, "result": { "records": [{ "dataset": "pages", "data": { "answer": "late" } }] } }))
            .await;
            report
        },
    );

    let file = run_file(&env, "hung");
    assert_eq!(
        (records(&file), work_items(&file)),
        (
            vec![(
                "pages".to_owned(),
                1,
                "1".to_owned(),
                json!({ "answer": "on time" })
            )],
            vec![("1".to_owned(), "succeeded".to_owned(), 2, None)],
        )
    );
}

/// With the settings' 60 s handler, a lease lasts 70 s of the paused clock:
/// when it ends, the engine cancels the attempt's request and asks again
/// under the next lease epoch. The cancelled attempt's answer that still
/// comes is a late commit, refused with `lease_lost`, its Records
/// discarded; the retried attempt's commit is the one that counts. The
/// `goodfetch::lease` trace the run fencing model describes is the same at
/// `batch`, the default, whose claims reach the run file in batches, and at
/// `full`, which writes each claim (#2174).
///
/// Covers: story:3-40, story:3-41, error:lease_lost, setting:run_state.durability
#[rstest::rstest]
fn a_late_commit_after_a_default_lease_ended_is_refused_and_the_retry_commits(
    #[values("batch", "full")] level: &str,
) {
    let env = TestEnv::new();

    let (arrivals, transitions) = crawl_fenced(
        || default_dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "late",
                        "seeds": seeds(1),
                        "settings": { "run_state": { "durability": level } },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client.late = Some(vec![json!({ "answer": "late" })]);
            let mut handler = |params: &Value| match params["attempt"].as_u64() {
                Some(1) => Reply::Silent,
                _ => Reply::Records(vec![json!({ "answer": "on time" })]),
            };
            client
                .call("run.finish", json!({ "run_id": "late" }), &mut handler)
                .await;
            client.arrivals.clone()
        },
    );

    let (first, first_at) = &arrivals[0];
    let timeline: Vec<(String, Value, Duration)> = arrivals
        .iter()
        .map(|(message, at)| {
            (
                message["method"].as_str().unwrap().to_owned(),
                message["params"]["lease_epoch"]
                    .as_u64()
                    .map_or_else(|| message["params"]["id"].clone(), |epoch| json!(epoch)),
                at.duration_since(*first_at),
            )
        })
        .collect();
    assert_eq!(
        timeline,
        vec![
            ("crawl_request.handle".to_owned(), json!(1), Duration::ZERO),
            (
                "$/cancel".to_owned(),
                first["id"].clone(),
                Duration::from_secs(70)
            ),
            (
                "crawl_request.handle".to_owned(),
                json!(2),
                Duration::from_secs(70)
            ),
        ]
    );
    assert_eq!(
        (of_epoch(&transitions, 1), of_epoch(&transitions, 2)),
        (
            vec![
                "grant".to_owned(),
                "expire".to_owned(),
                "refuse_commit lease_lost".to_owned()
            ],
            vec![
                "grant".to_owned(),
                "commit".to_owned(),
                "release".to_owned()
            ],
        )
    );
    let file = run_file(&env, "late");
    assert_eq!(
        (records(&file), work_items(&file)),
        (
            vec![(
                "pages".to_owned(),
                1,
                "1".to_owned(),
                json!({ "answer": "on time" })
            )],
            vec![("1".to_owned(), "succeeded".to_owned(), 2, None)],
        )
    );
}

/// A handler that never answers any attempt fails its work item with
/// `handler_lost` once `crawl.retry.max`'s 2 retries are spent. The Error
/// code is the work item's, never a reply's, so the run file shows it.
///
/// Covers: error:handler_lost
#[test]
fn a_handler_that_never_answers_any_attempt_fails_its_work_item_with_handler_lost() {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "gone", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": "gone" }), &mut |_| {
                    Reply::Silent
                })
                .await
        },
    );

    let file = run_file(&env, "gone");
    assert_eq!(
        (records(&file), work_items(&file)),
        (
            Vec::new(),
            vec![(
                "1".to_owned(),
                "failed".to_owned(),
                3,
                Some("handler_lost".to_owned())
            )],
        )
    );
}

/// At each Durability level (ADR 0142), the file an abort leaves holds its
/// work items pending, as a resume reads them.
///
/// Covers: operation:run.abort, setting:run_state.durability
#[rstest::rstest]
fn an_aborted_crawl_stops_incomplete_and_its_work_items_return_to_pending(
    #[values("batch", "full", "fast", "periodic", "off")] level: &str,
) {
    let env = TestEnv::new();

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "aborted",
                        "seeds": seeds(2),
                        "settings": { "run_state": { "durability": level } },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call("run.abort", json!({ "run_id": "aborted" }), &mut |_| {
                    Reply::Silent
                })
                .await
        },
    );

    let file = run_file(&env, "aborted");
    assert_eq!(
        (
            (
                &report["result"]["lifecycle"],
                &report["result"]["completeness"],
                &report["result"]["stop_reason"]
            ),
            work_items(&file),
        ),
        (
            (
                &json!("stopped"),
                &json!("incomplete"),
                &json!({ "kind": "abort", "by": "client" })
            ),
            vec![
                ("1".to_owned(), "pending".to_owned(), 0, None),
                ("2".to_owned(), "pending".to_owned(), 0, None),
            ],
        )
    );
}

/// ADR 0142's "Nothing removed" row of a crawl that writes nothing to its
/// run file until it stops: at `off` with no threshold, an answered page's
/// Records are not in the file while the crawl runs, and its clean stop
/// writes every one.
///
/// Covers: setting:run_state.durability, setting:run_state.batch.pages,
/// setting:run_state.batch.interval, setting:run_state.batch.bytes
#[test]
fn a_crawl_at_off_with_no_threshold_writes_its_records_only_as_it_stops() {
    let env = TestEnv::new();
    let mut while_running = None;

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "unwritten",
                        "seeds": seeds(2),
                        "settings": {
                            "crawl": { "handler": { "concurrency": 1 } },
                            "run_state": {
                                "durability": "off",
                                "batch": { "pages": "none", "interval": "none", "bytes": "none" },
                            },
                        },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                // One page at a time: the second's request comes once the
                // first's outcome was answered.
                if page_of(params).ends_with("/2") {
                    while_running = Some(records(&run_file(&env, "unwritten")).len());
                }
                answer_all(params)
            };
            client
                .call("run.finish", json!({ "run_id": "unwritten" }), &mut handler)
                .await
        },
    );

    assert_eq!(
        (
            while_running,
            &report["result"]["lifecycle"],
            records(&run_file(&env, "unwritten")).len(),
        ),
        (Some(0), &json!("finished"), 2)
    );
}

/// Covers: error:run_conflict
#[test]
fn a_start_under_a_crawls_run_id_with_other_inputs_conflicts_and_one_with_the_same_returns_it() {
    let env = TestEnv::new();

    let answers = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut silent = |_: &Value| Reply::Silent;
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "same", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut silent,
                )
                .await;
            let other_seeds = client
                .call(
                    "crawl.start",
                    json!({ "run_id": "same", "seeds": seeds(2), "labels": { "page": {} } }),
                    &mut silent,
                )
                .await;
            let a_collie_run = client
                .call("run.start", json!({ "run_id": "same" }), &mut silent)
                .await;
            let same = client
                .call(
                    "crawl.start",
                    json!({ "run_id": "same", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut silent,
                )
                .await;
            (other_seeds, a_collie_run, same)
        },
    );

    assert_eq!(
        (
            (
                &answers.0["error"]["data"]["code"],
                &answers.0["error"]["data"]["differences"]
            ),
            (
                &answers.1["error"]["data"]["code"],
                &answers.1["error"]["data"]["differences"]
            ),
            &answers.2["result"]["started_as"],
        ),
        (
            (&json!("run_conflict"), &json!(["seeds"])),
            (&json!("run_conflict"), &json!(["product", "kind", "seeds"])),
            &json!("existing"),
        )
    );
}

/// The Run index lists the crawl as Foxhound's, with its tags, and the
/// Run's own state records the Product it needs (ADR 0098).
///
/// Covers: story:3-28
#[test]
fn a_crawl_needs_foxhound_in_its_run_state_and_its_run_index_row() {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "tagged", "tags": ["nightly", "shop"], "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| Reply::Records(Vec::new());
            client
                .call("run.finish", json!({ "run_id": "tagged" }), &mut handler)
                .await
        },
    );

    let index = Connection::open(env.home().join("1").join("run-index.sqlite")).unwrap();
    let row: (String, String, String, String, String) = index
        .query_row(
            "SELECT location, kind, needs, tags, lifecycle FROM runs WHERE run_id = 'tagged'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    let needs: String = run_file(&env, "tagged")
        .query_row("SELECT needs FROM run", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (row, needs),
        (
            (
                "runs/tagged".to_owned(),
                "crawl".to_owned(),
                "foxhound".to_owned(),
                r#"["nightly","shop"]"#.to_owned(),
                "finished".to_owned(),
            ),
            "foxhound".to_owned(),
        )
    );
}

/// A crawl's own settings, the run layer, shape its attempts.
///
/// Covers: story:3-86, setting:crawl.retry.max
#[test]
fn a_crawls_settings_shape_its_attempts() {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "once",
                        "seeds": seeds(1),
                        "settings": { "crawl": { "retry": { "max": 0 } } },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| Reply::Fails("retry_requested", true);
            client
                .call("run.finish", json!({ "run_id": "once" }), &mut handler)
                .await
        },
    );

    assert_eq!(
        work_items(&run_file(&env, "once")),
        vec![(
            "1".to_owned(),
            "failed".to_owned(),
            1,
            Some("retry_requested".to_owned())
        )]
    );
}

/// An Operator policy's cap tightened while a crawl goes on reaches the next
/// crawl, not the one going: a Run resolves its settings once, as it opens
/// (ADR 0138), so the crawl going makes the retries its settings allowed,
/// and a crawl started after the change makes none.
///
/// Covers: story:3-88, story:12-98, setting:crawl.retry.max, setting:crawl.handler.concurrency, setting:crawl.retry.handler_failed, setting:timeout.handler
#[test]
fn an_operator_policy_cap_tightened_during_a_crawl_reaches_the_next_crawl() {
    let env = TestEnv::new();
    let policy = env.home().join("1").join("operator-policy.toml");
    let started =
        |run_id: &str| json!({ "run_id": run_id, "seeds": seeds(1), "labels": { "page": {} } });

    crawl(
        || default_dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call("crawl.start", started("going"), &mut |_| Reply::Silent)
                .await;
            let mut tighten = |_: &Value| {
                std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
                std::fs::write(&policy, "version = \"0.1\"\n\n[cap.crawl.retry]\nmax = 0\n")
                    .unwrap();
                Reply::Fails("retry_requested", true)
            };
            client
                .call("run.finish", json!({ "run_id": "going" }), &mut tighten)
                .await;
            client
                .call("crawl.start", started("after"), &mut |_| Reply::Silent)
                .await;
            let mut fails = |_: &Value| Reply::Fails("retry_requested", true);
            client
                .call("run.finish", json!({ "run_id": "after" }), &mut fails)
                .await
        },
    );

    let failed = |attempts| {
        vec![(
            "1".to_owned(),
            "failed".to_owned(),
            attempts,
            Some("retry_requested".to_owned()),
        )]
    };
    assert_eq!(
        (
            work_items(&run_file(&env, "going")),
            work_items(&run_file(&env, "after"))
        ),
        (failed(3), failed(1))
    );
}

/// An Operator policy's cap loosened while a crawl goes on does not reach
/// it: the crawl keeps the value the cap it began under held it to, and its
/// Run report names the version of the policy it began under, while a crawl
/// started after the change takes the loosened cap (ADR 0031).
///
/// Covers: story:12-98, setting:crawl.retry.max
#[test]
fn an_operator_policy_loosened_cap_applies_to_a_new_crawl_only() {
    let env = TestEnv::new();
    let policy = env.home().join("1").join("operator-policy.toml");
    let tight = "version = \"0.1\"\n\n[cap.crawl.retry]\nmax = 1\n";
    std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
    std::fs::write(&policy, tight).unwrap();
    let started =
        |run_id: &str| json!({ "run_id": run_id, "seeds": seeds(1), "labels": { "page": {} } });

    let (going, after) = crawl(
        || default_dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call("crawl.start", started("going"), &mut |_| Reply::Silent)
                .await;
            let mut loosen = |_: &Value| {
                std::fs::write(&policy, "version = \"0.1\"\n\n[cap.crawl.retry]\nmax = 5\n")
                    .unwrap();
                Reply::Fails("retry_requested", true)
            };
            let going = client
                .call("run.finish", json!({ "run_id": "going" }), &mut loosen)
                .await;
            client
                .call("crawl.start", started("after"), &mut |_| Reply::Silent)
                .await;
            let mut fails = |_: &Value| Reply::Fails("retry_requested", true);
            let after = client
                .call("run.finish", json!({ "run_id": "after" }), &mut fails)
                .await;
            (going, after)
        },
    );

    let attempts = |run_id: &str| work_items(&run_file(&env, run_id))[0].2;
    assert_eq!(
        (
            attempts("going"),
            going["result"]["policy_version"].clone(),
            attempts("after"),
            after["result"]["policy_version"] == going["result"]["policy_version"],
        ),
        (
            2,
            json!(goodfetch_engine::settings::policy_version(tight)),
            3,
            false,
        )
    );
}

/// A crawl whose `fetch.tier` forces a tier this engine does not build,
/// for the Run or for one of its Labels, is refused before its Run begins,
/// naming the setting and the tier (spec #579), and a Label setting a key
/// a Label does not take is refused with `setting_not_applicable`; the
/// conformance fetch suite starts one under `http`.
///
/// Covers: story:5-128, story:5-118, setting:fetch.tier
#[rstest::rstest]
/// Covers: error:capability_unavailable
#[case::by_the_run(
    json!({ "fetch": { "tier": "discovered_api" } }),
    json!({}),
    json!(["capability_unavailable", "fetch.tier", "discovered_api"])
)]
/// The light-browser tier is built, but only a Run that switches
/// `fetch.light_browser` on takes it (ADR 0020).
///
/// Covers: error:capability_unavailable, setting:fetch.light_browser, story:5-120
#[case::light_browser_the_run_does_not_allow(
    json!({ "fetch": { "tier": "light_browser" } }),
    json!({}),
    json!(["capability_unavailable", "fetch.light_browser", "light_browser"])
)]
/// Covers: error:capability_unavailable
#[case::by_a_label(
    json!({}),
    json!({ "list": { "settings": { "fetch": { "tier": "discovered_api" } } } }),
    json!(["capability_unavailable", "fetch.tier", "discovered_api"])
)]
/// Covers: error:setting_not_applicable
#[case::a_key_a_label_does_not_take(
    json!({}),
    json!({ "list": { "settings": { "crawl": { "max_depth": 2 } } } }),
    json!(["setting_not_applicable", "crawl.max_depth", null])
)]
fn a_crawl_forcing_a_fetch_tier_not_built_is_refused_with_capability_unavailable(
    #[case] settings: Value,
    #[case] labels: Value,
    #[case] expected: Value,
) {
    let env = TestEnv::new();

    let forced = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "forced",
                        "seeds": seeds(1),
                        "labels": labels,
                        "settings": settings,
                    }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );

    let refusal = &forced["error"]["data"];
    assert_eq!(
        (
            json!([refusal["code"], refusal["key"], refusal["kind"]]),
            &refusal["layer"]
        ),
        (expected, &json!("run")),
        "{forced}"
    );
}

/// `fetch.tier_race` switched on while a domain admits one request at a
/// time, where its navigation could never start beside its browser-fetch,
/// is refused before its Run begins with `setting_not_applicable` naming
/// the key and the Run's layer (ADR 0086), as is `fetch.tier_race` on a
/// Label, which the Run alone sets; a `fetch.encoding` that is no WHATWG
/// Encoding label, and a `crawl.content_types` of the Run or a Label that
/// is no list of media ranges (ADR 0119), are refused with
/// `setting_invalid`.
#[rstest::rstest]
/// Covers: story:6-263, setting:fetch.tier_race, error:setting_not_applicable
#[case::a_race_one_request_at_a_time(
    json!({ "fetch": { "tier_race": true }, "politeness": { "domain": { "concurrency": 1 } } }),
    json!({}),
    json!(["setting_not_applicable", "fetch.tier_race", null])
)]
/// Covers: error:setting_not_applicable
#[case::a_race_for_a_label(
    json!({}),
    json!({ "list": { "settings": { "fetch": { "tier_race": true } } } }),
    json!(["setting_not_applicable", "fetch.tier_race", null])
)]
/// Covers: setting:fetch.encoding, error:setting_invalid
#[case::an_unknown_encoding_for_the_run(
    json!({ "fetch": { "encoding": "klingon" } }),
    json!({}),
    json!(["setting_invalid", "fetch.encoding", null])
)]
/// Covers: story:6-264, setting:crawl.content_types, error:setting_invalid
#[case::content_types_that_are_no_media_ranges_for_the_run(
    json!({ "crawl": { "content_types": ["*/html"] } }),
    json!({}),
    json!(["setting_invalid", "crawl.content_types", null])
)]
/// Covers: story:6-264, setting:crawl.content_types, error:setting_invalid
#[case::content_types_that_are_no_media_ranges_for_a_label(
    json!({}),
    json!({ "pdf": { "settings": { "crawl": { "content_types": ["pdf"] } } } }),
    json!(["setting_invalid", "crawl.content_types", null])
)]
fn a_crawl_setting_a_fetch_key_it_cannot_apply_is_refused_before_it_begins(
    #[case] settings: Value,
    #[case] labels: Value,
    #[case] expected: Value,
) {
    a_crawl_forcing_a_fetch_tier_not_built_is_refused_with_capability_unavailable(
        settings, labels, expected,
    );
}

/// A setting that can never serve is refused, not ignored (ADRs 0110 and
/// 0120): a Label keeping responses (`fetch.kept_responses.urls`) on a
/// forced `http` or `browser_fetch` tier, its own or the Run's, and one
/// keeping its URLs' fragments (`crawl.keep_url_fragment`, its own or the
/// Run's) on a forced document tier, fail the start with
/// `setting_not_applicable` naming the key before the Run begins, ahead of
/// the tier's own `capability_unavailable`; responses kept on a tier that
/// could serve them are refused with `capability_unavailable` until the
/// engine keeps them.
///
/// Covers: story:6-262
#[rstest::rstest]
/// Covers: setting:fetch.kept_responses.urls, error:setting_not_applicable
#[case::responses_kept_on_a_labels_forced_http(
    json!({}),
    json!({ "api": { "settings": { "fetch": { "tier": "http", "kept_responses": { "urls": ["**/api/*"] } } } } }),
    json!(["setting_not_applicable", "fetch.kept_responses.urls", null])
)]
/// Covers: setting:fetch.kept_responses.urls, error:setting_not_applicable
#[case::responses_kept_on_a_labels_forced_browser_fetch(
    json!({}),
    json!({ "api": { "settings": { "fetch": { "tier": "browser_fetch", "kept_responses": { "urls": ["**/api/*"] } } } } }),
    json!(["setting_not_applicable", "fetch.kept_responses.urls", null])
)]
/// Covers: setting:fetch.kept_responses.urls, error:setting_not_applicable
#[case::responses_kept_by_a_label_on_the_runs_forced_http(
    json!({ "fetch": { "tier": "http" } }),
    json!({ "api": { "settings": { "fetch": { "kept_responses": { "urls": ["**/api/*"] } } } } }),
    json!(["setting_not_applicable", "fetch.kept_responses.urls", null])
)]
/// Covers: story:6-221, setting:crawl.keep_url_fragment, error:setting_not_applicable
#[case::fragments_kept_on_a_labels_forced_http(
    json!({}),
    json!({ "app": { "settings": { "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "http" } } } }),
    json!(["setting_not_applicable", "crawl.keep_url_fragment", null])
)]
/// Covers: story:6-221, setting:crawl.keep_url_fragment, error:setting_not_applicable
#[case::fragments_kept_on_a_labels_forced_discovered_api(
    json!({}),
    json!({ "app": { "settings": { "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "discovered_api" } } } }),
    json!(["setting_not_applicable", "crawl.keep_url_fragment", null])
)]
/// Covers: story:6-221, setting:crawl.keep_url_fragment, error:setting_not_applicable
#[case::fragments_kept_by_the_run_on_a_labels_forced_browser_fetch(
    json!({ "crawl": { "keep_url_fragment": true } }),
    json!({ "app": { "settings": { "fetch": { "tier": "browser_fetch" } } } }),
    json!(["setting_not_applicable", "crawl.keep_url_fragment", null])
)]
/// Covers: story:6-221, setting:crawl.keep_url_fragment, error:setting_not_applicable
#[case::fragments_kept_by_the_run_on_its_forced_http(
    json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "http" } }),
    json!({}),
    json!(["setting_not_applicable", "crawl.keep_url_fragment", null])
)]
/// Covers: setting:fetch.kept_responses.urls, error:capability_unavailable
#[case::responses_kept_on_the_browser(
    json!({}),
    json!({ "api": { "settings": { "fetch": { "tier": "browser", "kept_responses": { "urls": ["**/api/*"] } } } } }),
    json!(["capability_unavailable", "fetch.kept_responses.urls", "kept_responses"])
)]
fn a_label_whose_settings_can_never_be_served_is_refused_at_the_runs_start(
    #[case] settings: Value,
    #[case] labels: Value,
    #[case] expected: Value,
) {
    a_crawl_forcing_a_fetch_tier_not_built_is_refused_with_capability_unavailable(
        settings, labels, expected,
    );
}

/// A crawl's start checks a field's CSS selector on the engine's Counted
/// work, the one its Run compiles the selector through: a start refused for
/// a later field compiled the selector there once, where a Run reading it
/// would find it kept, rather than on Counted work of the check's own
/// (#2605).
#[test]
fn a_start_checks_a_fields_selector_on_the_engines_counted_work() {
    let env = TestEnv::new();
    let metrics = goodfetch_engine::metrics::Metrics::none();
    let work = metrics.counted_work().clone();
    let engine = engine().with_metrics(metrics);

    let (refused, _) = crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "refused",
                        "seeds": seeds(1),
                        "datasets": [],
                        "extraction": {
                            "formats": { "markdown": true },
                            "fields": {
                                "lead": { "locator": { "kind": "css", "selector": "p.lead" } },
                                "unread": { "locator": { "kind": "xpath", "selector": "count(//h1)" } },
                            },
                        },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            work.read(goodfetch_counted_work::Row::PageSelectorCompilations)
        ),
        (&json!("invalid_params"), 1),
        "{refused}"
    );
}

/// A Saved identity's Session source, which gives the Sessions that open
/// from it a login.
fn shop() -> Value {
    json!({ "kind": "saved_identity", "name": "shop" })
}

/// A pool whose Sessions hold a login, from a `session.source` (#2418),
/// never sends from a Session's own transport, so an explicit
/// `fetch.http_from: transport` where its Sessions open, the Run's for the
/// Run's pool and a Label's own for its requests, conflicts with the
/// identity `session.source` fixed: the start is refused with
/// `setting_locked`, whose `limit` names `session.source`, its value and
/// the layer that set it, and the Run's `settings` as its origin when the
/// Run set it (ADR 0083).
///
/// Covers: setting:fetch.http_from, setting:session.source, error:setting_locked
#[rstest::rstest]
#[case::a_labels_own_login_pool(
    json!({}),
    json!({ "account": { "settings": { "session": { "source": shop() }, "fetch": { "http_from": "transport" } } } }),
    json!({ "key": "session.source", "value": shop(), "layer": "run" })
)]
#[case::the_runs_login_pool(
    json!({ "session": { "source": shop() }, "fetch": { "http_from": "transport" } }),
    json!({}),
    json!({ "key": "session.source", "value": shop(), "layer": "run", "origin": "settings" })
)]
#[case::a_label_on_the_runs_login_pool(
    json!({ "session": { "source": shop() } }),
    json!({ "list": { "settings": { "fetch": { "http_from": "transport" } } } }),
    json!({ "key": "session.source", "value": shop(), "layer": "run", "origin": "settings" })
)]
fn transport_on_a_pool_that_holds_a_login_is_refused_at_the_runs_start(
    #[case] settings: Value,
    #[case] labels: Value,
    #[case] limit: Value,
) {
    let env = TestEnv::new();

    let refused = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "login",
                        "seeds": seeds(1),
                        "labels": labels,
                        "settings": settings,
                    }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );

    let refusal = &refused["error"]["data"];
    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert_eq!(
        (
            json!([refusal["code"], refusal["key"], refusal["layer"]]),
            &refusal["limit"],
            message.contains("`fetch.http_from`") && message.contains("`session.source`"),
        ),
        (
            json!(["setting_locked", "fetch.http_from", "run"]),
            &limit,
            true,
        ),
        "{refused}"
    );
}

/// An inherited `fetch.http_from` yields to a pool's login (ADR 0083): a
/// Label whose own pool holds one under the Run's `transport`, and a
/// login pool under `learned`, start, their requests sent from the
/// browser (#2418).
///
/// Covers: setting:fetch.http_from, setting:session.source
#[rstest::rstest]
#[case::a_labels_login_pool_under_the_runs_transport(
    json!({ "fetch": { "http_from": "transport" } }),
    json!({ "account": { "settings": { "session": { "source": shop() } } }, "page": {} })
)]
#[case::the_runs_login_pool_under_learned(
    json!({ "session": { "source": shop() }, "fetch": { "http_from": "learned" } }),
    json!({ "page": {} })
)]
#[case::a_labels_login_pool_under_its_own_learned(
    json!({}),
    json!({ "account": { "settings": { "session": { "source": shop() }, "fetch": { "http_from": "learned" } } }, "page": {} })
)]
fn a_login_pool_under_a_value_that_yields_starts(#[case] settings: Value, #[case] labels: Value) {
    let env = TestEnv::new();

    let started = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "login",
                        "seeds": seeds(1),
                        "labels": labels,
                        "settings": settings,
                    }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );

    assert_eq!(started["result"]["run_id"], "login", "{started}");
}

/// A Label keeping its URLs' fragments on a tier that runs the page's
/// scripts, forced or chosen by the engine, starts, as does a Label
/// dropping them on a forced `http` tier (ADR 0120).
///
/// Covers: story:6-221, setting:crawl.keep_url_fragment
#[rstest::rstest]
#[case::on_a_forced_browser(json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "browser" } }))]
#[case::under_auto(json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "auto" } }))]
#[case::dropped_on_a_forced_http(json!({ "crawl": { "keep_url_fragment": false }, "fetch": { "tier": "http" } }))]
fn a_label_keeping_fragments_on_a_tier_that_serves_them_starts(#[case] label: Value) {
    let env = TestEnv::new();

    let started = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "fragments",
                        "seeds": seeds(1),
                        "labels": { "app": { "settings": label }, "page": {} },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );

    assert_eq!(started["result"]["run_id"], "fragments", "{started}");
}

/// A crawl request's identity follows `crawl.keep_url_fragment`, which is
/// fixed for a Run's life (ADRs 0102 and 0120): a start under the Run's ID
/// that changes it, for the Run or for a Label, is refused with
/// `setting_not_applicable` naming the key, while one that keeps it goes
/// on with the Run.
///
/// Covers: story:6-221, setting:crawl.keep_url_fragment, error:setting_not_applicable
#[test]
fn a_start_under_a_runs_id_that_changes_keep_url_fragment_is_refused() {
    let env = TestEnv::new();
    let start = |settings: Value, labels: Value| {
        json!({
            "run_id": "fixed",
            "seeds": seeds(1),
            "labels": labels,
            "settings": settings,
        })
    };
    let keeping = json!({ "app": { "settings": { "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "browser" } } }, "page": {} });
    let dropping = json!({ "app": { "settings": { "fetch": { "tier": "browser" } } }, "page": {} });

    let answers = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut answers = Vec::new();
            for params in [
                start(json!({}), keeping.clone()),
                start(json!({}), dropping.clone()),
                start(
                    json!({ "crawl": { "keep_url_fragment": true } }),
                    keeping.clone(),
                ),
                start(json!({}), keeping.clone()),
            ] {
                answers.push(
                    client
                        .call("crawl.start", params, &mut |_| Reply::Silent)
                        .await,
                );
            }
            answers
        },
    );

    let read: Vec<Value> = answers
        .iter()
        .map(|answer| {
            answer.get("error").map_or_else(
                || answer["result"]["run_id"].clone(),
                |error| json!([error["data"]["code"], error["data"]["key"]]),
            )
        })
        .collect();
    assert_eq!(
        read,
        vec![
            json!("fixed"),
            json!(["setting_not_applicable", "crawl.keep_url_fragment"]),
            json!(["setting_not_applicable", "crawl.keep_url_fragment"]),
            json!("fixed"),
        ]
    );
}

/// A crawl whose `fetch.redirect.max` is outside 1 to 19, as many
/// redirects as the browser follows, is refused before its Run begins
/// (ADR 0118): following none is `crawl.handle_statuses`'s to say.
///
/// Covers: story:5-124, setting:fetch.redirect.max, error:setting_invalid
#[rstest::rstest]
#[case::none(0)]
#[case::one_past_the_browsers(20)]
fn a_crawl_whose_redirect_count_the_browser_cannot_keep_is_refused(#[case] max: u64) {
    let env = TestEnv::new();

    let refused = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "redirects",
                        "seeds": seeds(1),
                        "settings": { "fetch": { "redirect": { "max": max } } },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );

    let refusal = &refused["error"]["data"];
    assert_eq!(
        (&refusal["code"], &refusal["key"], &refusal["layer"]),
        (
            &json!("setting_invalid"),
            &json!("fetch.redirect.max"),
            &json!("run")
        ),
        "{refused}"
    );
}
