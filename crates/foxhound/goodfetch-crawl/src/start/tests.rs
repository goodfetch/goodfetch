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
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod admission;
mod keep_alive;
mod limits;
mod model;
mod report;
mod resume;
mod safe_point;

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
}

impl Client {
    pub(crate) async fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).await.unwrap();
    }

    async fn next(&mut self) -> Option<Value> {
        let line = self.lines.next_line().await.unwrap()?;
        Some(serde_json::from_str(&line).unwrap())
    }

    /// Sends the call `method` and answers the engine's handler requests
    /// with `handler` until the call's reply arrives, which it answers.
    pub(crate) async fn call(
        &mut self,
        method: &str,
        params: Value,
        handler: &mut impl FnMut(&Value) -> Reply,
    ) -> Value {
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
                match handler(&message["params"]) {
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
            "version = \"0.1\"\n\n[settings]\ntimeout.handler = {}\ncrawl.retry.max = {}\ncrawl.retry.handler_failed = {}\ncrawl.handler.concurrency = {}\n",
            policy.handler_timeout.as_millis(),
            policy.retry_max,
            policy.retry_handler_failed,
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
        home: Home::new(env.home().to_owned()),
        clock: Clock::paused(env.start_time()),
        entropy: Entropy::seeded(env.seed()),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state,
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// A Foxhound engine composed of the engine's operations, the Frontier's
/// per-domain queues, the Session pool's tables and the crawl's.
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
        .with(super::contribution());
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
    let engine = engine();
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
                    json!({ "run_id": "shop", "seeds": seeds(3) }),
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
                    json!({ "run_id": "retry", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| match params["attempt"].as_u64() {
                Some(1) => Reply::Fails("retry_requested", true),
                _ => Reply::Records(vec![json!("second attempt")]),
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
                json!("second attempt")
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
                    json!({ "run_id": "bug", "seeds": seeds(1) }),
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
                    json!({ "run_id": "hung", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| match params["attempt"].as_u64() {
                Some(1) => Reply::Silent,
                _ => Reply::Records(vec![json!("on time")]),
            };
            let started = client.call("run.finish", json!({ "run_id": "hung" }), &mut handler);
            let report = started.await;
            let late = client.silent[0].clone();
            client
            .send(&json!({ "jsonrpc": "2.0", "id": late, "result": { "records": [{ "dataset": "pages", "data": "late" }] } }))
            .await;
            report
        },
    );

    let file = run_file(&env, "hung");
    assert_eq!(
        (records(&file), work_items(&file)),
        (
            vec![("pages".to_owned(), 1, "1".to_owned(), json!("on time"))],
            vec![("1".to_owned(), "succeeded".to_owned(), 2, None)],
        )
    );
}

/// With the settings' 60 s handler, a lease lasts 70 s of the paused clock:
/// when it ends, the engine cancels the attempt's request and asks again
/// under the next lease epoch. The cancelled attempt's answer that still
/// comes is a late commit, refused with `lease_lost`, its Records
/// discarded; the retried attempt's commit is the one that counts.
///
/// Covers: story:3-40, story:3-41, error:lease_lost
#[test]
fn a_late_commit_after_a_default_lease_ended_is_refused_and_the_retry_commits() {
    let env = TestEnv::new();

    let (arrivals, transitions) = crawl_fenced(
        || default_dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "late", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client.late = Some(vec![json!("late")]);
            let mut handler = |params: &Value| match params["attempt"].as_u64() {
                Some(1) => Reply::Silent,
                _ => Reply::Records(vec![json!("on time")]),
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
            vec![("pages".to_owned(), 1, "1".to_owned(), json!("on time"))],
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
                    json!({ "run_id": "gone", "seeds": seeds(1) }),
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

/// Covers: operation:run.abort
#[test]
fn an_aborted_crawl_stops_incomplete_and_its_work_items_return_to_pending() {
    let env = TestEnv::new();

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "aborted", "seeds": seeds(2) }),
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
                    json!({ "run_id": "same", "seeds": seeds(1) }),
                    &mut silent,
                )
                .await;
            let other_seeds = client
                .call(
                    "crawl.start",
                    json!({ "run_id": "same", "seeds": seeds(2) }),
                    &mut silent,
                )
                .await;
            let a_collie_run = client
                .call("run.start", json!({ "run_id": "same" }), &mut silent)
                .await;
            let same = client
                .call(
                    "crawl.start",
                    json!({ "run_id": "same", "seeds": seeds(1) }),
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
                    json!({ "run_id": "tagged", "tags": ["nightly", "shop"], "seeds": seeds(1) }),
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

/// An Operator policy's cap tightened while a crawl goes on reaches it at
/// its next admission point: the retry the crawl's settings allowed is not
/// made (ADR 0028).
///
/// Covers: story:3-88, setting:crawl.retry.max, setting:crawl.handler.concurrency, setting:crawl.retry.handler_failed, setting:timeout.handler
#[test]
fn an_operator_policy_tightened_cap_reaches_a_crawl_already_going() {
    let env = TestEnv::new();
    let policy = env.home().join("1").join("operator-policy.toml");

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "capped", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| {
                std::fs::write(&policy, "version = \"0.1\"\n\n[cap.crawl.retry]\nmax = 0\n")
                    .unwrap();
                Reply::Fails("retry_requested", true)
            };
            client
                .call("run.finish", json!({ "run_id": "capped" }), &mut handler)
                .await
        },
    );

    assert_eq!(
        work_items(&run_file(&env, "capped")),
        vec![(
            "1".to_owned(),
            "failed".to_owned(),
            1,
            Some("retry_requested".to_owned())
        )]
    );
}
