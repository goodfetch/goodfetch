//! `workers=N` on Foxhound's executable (spec #213's Seam 1; ADRs 0024,
//! 0031 and 0094: work items are Foxhound's): a handler process joins the
//! crawl another Client started on an Attached engine, through the private
//! endpoint the engine reported to that Client, and both lease work items
//! of the same Run, each at most `crawl.handler.concurrency` at once; a
//! handler process whose Connection ends leaves its attempt `client_gone`,
//! not `handler_failed`, and the work item is retried on the Connection
//! still serving.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the private endpoint's token file and the run file themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::{Attached, DaemonClient, Hello, HomeLayout};
use goodfetch_test_support::{DaemonEndpoint, OUTCOME_TARGET, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::FOXHOUND;
use crate::governor::in_flight_whatever_the_machine;

/// How long a handler process waits for each message from the engine.
const DEADLINE: Duration = Duration::from_secs(10);

fn hello(token: Option<&str>) -> Value {
    let hello = Hello::needing(Product::Foxhound).id("hello");
    match token {
        Some(token) => hello.token(token),
        None => hello,
    }
    .message()
}

/// One end of a Connection to the engine: the stdio of an Attached engine,
/// or a handler process's Connection to its private endpoint.
trait Peer {
    fn send(&mut self, message: &Value);
    /// The next message; `None` once the engine closed the Connection.
    fn next(&mut self) -> Option<Value>;

    /// Reads on until the engine sends a `crawl_request.handle` request,
    /// keeping every other message in `kept`.
    fn request(&mut self, kept: &mut Vec<Value>) -> Value {
        loop {
            let message = self.next().expect("the engine closed before its request");
            if message["method"] == "crawl_request.handle" {
                return message;
            }
            kept.push(message);
        }
    }

    /// Answers `request` with one Record naming its URL.
    fn answer(&mut self, request: &Value) {
        let url = &request["params"]["crawl_request"]["url"];
        self.send(&json!({
            "jsonrpc": "2.0", "id": request["id"],
            "result": { "records": [{ "dataset": "pages", "data": { "url": url } }] },
        }));
    }

    /// Reads on until the reply to `id`, among `kept` or what comes.
    fn reply(&mut self, id: &str, kept: &mut Vec<Value>) -> Value {
        if let Some(place) = kept.iter().position(|message| message["id"] == id) {
            return kept.remove(place);
        }
        loop {
            let message = self.next().expect("the engine closed before replying");
            if message["id"] == id && message.get("method").is_none() {
                return message;
            }
        }
    }
}

/// Starts the engine with two HTTP admissions whatever the machine:
/// each test holds a request on the Client while the handler process
/// waits for its own, and a governor at its floor of one under a
/// loaded machine's memory pressure sent the handler process none
/// (fix-queue 130).
fn started_engine(env: &TestEnv) -> Attached {
    env.user_configuration(&in_flight_whatever_the_machine(2));
    Attached::start(env, FOXHOUND)
}

/// An Attached engine of Foxhound's executable, its stdio the Client's.
impl Peer for Attached {
    fn send(&mut self, message: &Value) {
        Attached::send(self, message);
    }

    fn next(&mut self) -> Option<Value> {
        Attached::next(self)
    }
}

/// Connects to the private endpoint `reported` names and says `hello`
/// with the token of its token file, as a handler process does.
fn joined(reported: &Value) -> DaemonClient {
    let address: DaemonEndpoint = serde_json::from_value(reported["address"].clone()).unwrap();
    let token = std::fs::read_to_string(reported["token_file"].as_str().unwrap()).unwrap();
    let mut handler_process = DaemonClient::connect_within(&address, DEADLINE);
    handler_process.send(&hello(Some(&token)));
    let answered = handler_process.reply("hello", &mut Vec::new());
    assert_eq!(answered["result"]["product"], "foxhound", "{answered}");
    handler_process
}

/// A handler process's Connection to the engine's private endpoint.
impl Peer for DaemonClient {
    fn send(&mut self, message: &Value) {
        DaemonClient::send(self, message);
    }

    fn next(&mut self) -> Option<Value> {
        DaemonClient::next(self)
    }
}

fn seeds(count: usize) -> Value {
    (1..=count)
        .map(|n| json!({ "url": format!("https://shop.test/{n}"), "label": "page" }))
        .collect()
}

fn url(request: &Value) -> String {
    request["params"]["crawl_request"]["url"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Starts the crawl `run_id` of `count` seeds, one work item at a time on
/// each Connection, and answers the private endpoint the engine reported
/// and the first request the Client got.
fn started(
    engine: &mut Attached,
    run_id: &str,
    count: usize,
    kept: &mut Vec<Value>,
) -> (Value, Value) {
    engine.send(&hello(None));
    let reported = engine.reply("hello", kept)["result"]["private_endpoint"].clone();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": {
            "run_id": run_id, "seeds": seeds(count), "datasets": [{ "name": "pages" }],
            "settings": { "crawl": { "handler": { "concurrency": 1 } } },
        },
    }));
    let first = engine.request(kept);
    (reported, first)
}

fn work_items(env: &TestEnv, run_id: &str) -> Vec<(String, String, i64, Option<String>)> {
    Connection::open(HomeLayout::of(env.home()).run_file(run_id))
        .unwrap()
        .prepare("SELECT url, status, attempts, error_code FROM work_items ORDER BY position")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The Error codes the engine `pid` recorded in its Runs' state, in order,
/// as its JSON log carries them (ADR 0072).
fn outcomes(env: &TestEnv, pid: u32) -> Vec<String> {
    let log = env
        .home()
        .join("logs")
        .join(format!("goodfetch-foxhound-{pid}.jsonl"));
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["target"] == OUTCOME_TARGET)
        .map(|record| record["fields"]["error_code"].as_str().unwrap().to_owned())
        .collect()
}

fn records(env: &TestEnv, run_id: &str) -> Vec<String> {
    Connection::open(HomeLayout::of(env.home()).run_file(run_id))
        .unwrap()
        .prepare("SELECT data FROM records ORDER BY data")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Covers: story:3-17, operation:crawl.serve, operation:crawl.start
#[test]
fn a_handler_process_on_the_private_endpoint_leases_work_items_from_the_same_run() {
    let env = TestEnv::new();
    let mut engine = started_engine(&env);
    let mut kept = Vec::new();
    let (reported, first) = started(&mut engine, "shared", 4, &mut kept);
    let mut handler_process = joined(&reported);
    handler_process.send(&json!({ "jsonrpc": "2.0", "id": "serve", "method": "crawl.serve", "params": { "run_id": "shared" } }));
    let mut handler_process_kept = Vec::new();
    let second = handler_process.request(&mut handler_process_kept);

    engine.answer(&first);
    let third = engine.request(&mut kept);
    handler_process.answer(&second);
    let fourth = handler_process.request(&mut handler_process_kept);
    engine.answer(&third);
    handler_process.answer(&fourth);
    let served = handler_process.reply("serve", &mut handler_process_kept);
    engine.send(&json!({ "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": "shared" } }));
    let finished = engine.reply("finish", &mut kept);
    drop(handler_process);
    let exited = engine.finished();

    assert_eq!(
        (
            [url(&first), url(&third)],
            [url(&second), url(&fourth)],
            served["result"]["lifecycle"].clone(),
            finished["result"]["completeness"].clone(),
            records(&env, "shared").len(),
            exited,
        ),
        (
            [
                "https://shop.test/1".to_owned(),
                "https://shop.test/3".to_owned()
            ],
            [
                "https://shop.test/2".to_owned(),
                "https://shop.test/4".to_owned()
            ],
            json!("finished"),
            json!("complete"),
            4,
            true,
        )
    );
}

/// Covers: story:3-16, error:client_gone
#[test]
fn a_handler_process_whose_connection_ends_leaves_its_attempt_client_gone_retried_on_another() {
    let env = TestEnv::new();
    let mut engine = started_engine(&env);
    let pid = engine.pid();
    let mut kept = Vec::new();
    let (reported, first) = started(&mut engine, "dropped", 2, &mut kept);
    let mut handler_process = joined(&reported);
    handler_process.send(&json!({ "jsonrpc": "2.0", "id": "serve", "method": "crawl.serve", "params": { "run_id": "dropped" } }));
    let lost = handler_process.request(&mut Vec::new());

    drop(handler_process);
    engine.answer(&first);
    let retried = engine.request(&mut kept);
    engine.answer(&retried);
    engine.send(&json!({ "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": "dropped" } }));
    let finished = engine.reply("finish", &mut kept);
    let exited = engine.finished();

    assert_eq!(
        (
            url(&lost),
            retried["params"]["attempt"].clone(),
            url(&retried),
            finished["result"]["completeness"].clone(),
            work_items(&env, "dropped"),
            exited,
            outcomes(&env, pid),
        ),
        (
            "https://shop.test/2".to_owned(),
            json!(2),
            "https://shop.test/2".to_owned(),
            json!("complete"),
            vec![
                (
                    "https://shop.test/1".to_owned(),
                    "succeeded".to_owned(),
                    1,
                    None
                ),
                (
                    "https://shop.test/2".to_owned(),
                    "succeeded".to_owned(),
                    2,
                    None
                ),
            ],
            true,
            vec!["client_gone".to_owned()],
        )
    );
}

/// Covers: operation:crawl.serve, error:run_not_found
#[test]
fn crawl_serve_of_a_run_the_engine_runs_no_crawl_of_answers_run_not_found() {
    let env = TestEnv::new();
    let mut engine = started_engine(&env);
    let mut kept = Vec::new();
    engine.send(&hello(None));
    engine.reply("hello", &mut kept);

    engine.send(&json!({ "jsonrpc": "2.0", "id": "serve", "method": "crawl.serve", "params": { "run_id": "elsewhere" } }));
    let answered = engine.reply("serve", &mut kept);

    assert_eq!(
        (answered["error"]["data"]["code"].clone(), engine.finished()),
        (json!("run_not_found"), true)
    );
}
