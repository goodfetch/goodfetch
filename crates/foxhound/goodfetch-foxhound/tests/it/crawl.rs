//! A crawl on Foxhound's executable over stdio (spec #213's Seam 1; ADR
//! 0094: work items are Foxhound's): `crawl.start` with seeds, each work
//! item sent to the Client's handler as a `crawl_request.handle` request,
//! the handler's Records committed once in the Run's own file in the
//! Goodfetch home's per-major folder, a retryable failure tried again, and
//! `$/cancel` ending a `run.finish` that waits. Read through `StrictStdout`
//! with the engine's logs at `trace`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::Write as _;
use std::process::{ChildStdin, Stdio};
use std::time::Duration;

use goodfetch_protocol::jsonrpc::Message;
use goodfetch_test_support::{Process, StrictStdout, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

/// A running Attached engine of Foxhound's executable.
struct Attached {
    process: Process,
    stdin: Option<ChildStdin>,
    stdout: StrictStdout,
}

impl Attached {
    fn start(env: &TestEnv) -> Self {
        let mut command = env.process_command(env!("CARGO_BIN_EXE_goodfetch-foxhound"));
        command
            .arg("attached-engine")
            .env("GOODFETCH_LOG", "trace")
            .stderr(Stdio::piped());
        let process = env.spawn(&mut command).unwrap();
        let stdin = process.child().stdin.take();
        let stdout = StrictStdout::of(&process).unwrap();
        let mut engine = Self {
            process,
            stdin,
            stdout,
        };
        engine.send(&json!({
            "jsonrpc": "2.0", "id": "hello", "method": "hello",
            "params": { "protocol_version": "0.1", "surface": "sdk_python", "needs": "foxhound", "product_version": "0.0.0" },
        }));
        assert_eq!(engine.next().unwrap()["result"]["product"], "foxhound");
        engine
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        stdin.write_all(&line).unwrap();
        stdin.flush().unwrap();
    }

    fn next(&mut self) -> Option<Value> {
        let message = self.stdout.try_next().unwrap()?;
        serde_json::from_value::<Message>(message.clone()).unwrap();
        Some(message)
    }

    /// Sends the call `id`, answers each handler request with `handler`
    /// until its reply comes, and answers that reply.
    fn call(
        &mut self,
        id: &str,
        method: &str,
        params: &Value,
        handler: &mut impl FnMut(&Value) -> Option<Value>,
    ) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next().expect("the engine closed before replying");
            if message["method"] == "crawl_request.handle" {
                if let Some(answer) = handler(&message["params"]) {
                    let mut reply = answer;
                    reply["jsonrpc"] = json!("2.0");
                    reply["id"] = message["id"].clone();
                    self.send(&reply);
                }
            } else if message["id"] == id {
                return message;
            }
        }
    }

    /// Whether the engine writes nothing on its stdout for `quiet`; a
    /// message written meanwhile is kept for the next read.
    fn is_quiet_for(&mut self, quiet: Duration) -> bool {
        self.stdout.is_quiet_for(quiet)
    }

    /// Ends the input and waits for the engine to exit.
    fn finish(mut self) -> bool {
        drop(self.stdin.take());
        while self.next().is_some() {}
        self.process.child().wait().unwrap().success()
    }
}

fn seeds() -> Value {
    json!([
        { "url": "https://shop.test/a", "label": "page" },
        { "url": "https://shop.test/b", "label": "page", "payload": { "flaky": true } },
    ])
}

/// Covers: story:3-25, story:3-28, story:3-41, operation:crawl.start
#[test]
fn a_crawl_on_foxhounds_executable_commits_its_handlers_records_once_in_its_own_run_file() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);

    let started = engine.call(
        "start",
        "crawl.start",
        &json!({ "run_id": "shop", "seeds": seeds() }),
        &mut |_| None,
    );
    let mut handler = |params: &Value| {
        let flaky = params["crawl_request"]["payload"]["flaky"] == true;
        Some(if flaky && params["attempt"] == 1 {
            json!({ "error": { "code": -32003, "message": "try again", "data": { "code": "retry_requested", "retryable": true } } })
        } else {
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": params["crawl_request"]["url"] } }] } })
        })
    };
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "shop" }),
        &mut handler,
    );
    let exited = engine.finish();

    let file = Connection::open(
        env.home()
            .join("0.0")
            .join("runs")
            .join("shop")
            .join("run.sqlite"),
    )
    .unwrap();
    let records: Vec<String> = file
        .prepare("SELECT data FROM records ORDER BY number")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let journal_mode: String = file
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(
        (
            &started["result"]["needs"],
            (
                &report["result"]["lifecycle"],
                &report["result"]["completeness"]
            ),
            records,
            journal_mode,
            exited,
        ),
        (
            &json!("foxhound"),
            (&json!("finished"), &json!("complete")),
            vec![
                r#"{"url":"https://shop.test/a"}"#.to_owned(),
                r#"{"url":"https://shop.test/b"}"#.to_owned(),
            ],
            "wal".to_owned(),
            true,
        )
    );
}

/// `run.finish` on a crawl waits for its work items; `$/cancel` ends only
/// the wait, with `call_cancelled`, and the Connection goes on answering.
///
/// Covers: story:3-6, error:call_cancelled
#[test]
fn cancel_of_run_finish_in_flight_answers_call_cancelled() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.call(
        "start",
        "crawl.start",
        &json!({ "run_id": "slow", "seeds": seeds() }),
        &mut |_| None,
    );

    engine.send(&json!({ "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": "slow" } }));
    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": "finish" } }));
    let cancelled = loop {
        let message = engine.next().unwrap();
        if message["id"] == "finish" {
            break message;
        }
    };
    let aborted = engine.call(
        "abort",
        "run.abort",
        &json!({ "run_id": "slow" }),
        &mut |_| None,
    );

    assert_eq!(
        (
            &cancelled["error"]["data"]["code"],
            &aborted["result"]["lifecycle"],
            engine.finish(),
        ),
        (&json!("call_cancelled"), &json!("stopped"), true)
    );
}

/// How many handler requests a crawl keeps in flight by default
/// (`crawl.handler.concurrency`).
const HANDLER_CONCURRENCY: usize = 16;

/// A slow handler holds the crawl back: no more than the handler
/// concurrency's requests wait for it, and once it answers, every work
/// item's Records are committed once. A subscriber that grants no credits
/// holds back nothing only Foxhound produces, since Records are read from
/// run state (ADR 0103), and its queue still gets every axis change.
///
/// Covers: story:3-8, event:run.axes_changed
#[test]
fn a_slow_handler_holds_the_crawl_back_and_a_slow_subscriber_loses_no_record_or_axis_change() {
    const SEEDS: usize = HANDLER_CONCURRENCY + 4;
    let seed_count = i64::try_from(SEEDS).unwrap();
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(
        &json!({ "jsonrpc": "2.0", "id": "events", "method": "events.subscribe", "params": {} }),
    );
    let seeds: Vec<Value> = (0..SEEDS)
        .map(|index| json!({ "url": format!("https://shop.test/{index}"), "label": "page" }))
        .collect();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "held", "seeds": seeds },
    }));

    let (mut started, mut waiting, mut most_waiting) = (false, Vec::new(), 0);
    while !started || waiting.len() < HANDLER_CONCURRENCY {
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            waiting.push(message);
        } else if message["id"] == "start" {
            started = true;
        }
    }
    let held = engine.is_quiet_for(Duration::from_millis(500));
    let mut handled = 0;
    while handled < SEEDS {
        let request = if waiting.is_empty() {
            let message = engine.next().unwrap();
            assert_eq!(message["method"], "crawl_request.handle", "{message}");
            message
        } else {
            waiting.remove(0)
        };
        most_waiting = most_waiting.max(waiting.len() + 1);
        engine.send(&json!({
            "jsonrpc": "2.0", "id": request["id"],
            "result": { "records": [{ "dataset": "pages", "data": { "url": request["params"]["crawl_request"]["url"] } }] },
        }));
        handled += 1;
    }
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "held" }),
        &mut |_| None,
    );
    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "events", "credits": 10 } }));
    let mut events = Vec::new();
    while events.len() < 2 {
        let message = engine.next().unwrap();
        if message["method"] == "$/events" {
            events.extend(message["params"]["events"].as_array().unwrap().clone());
        }
    }
    let exited = engine.finish();

    let file = Connection::open(
        env.home()
            .join("0.0")
            .join("runs")
            .join("held")
            .join("run.sqlite"),
    )
    .unwrap();
    let (records, work_items): (i64, i64) = file
        .query_row(
            "SELECT count(*), count(DISTINCT work_item) FROM records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let lifecycles: Vec<&Value> = events
        .iter()
        .map(|event| &event["data"]["lifecycle"])
        .collect();
    assert_eq!(
        (
            held,
            most_waiting,
            &report["result"]["lifecycle"],
            (records, work_items),
            lifecycles,
            exited,
        ),
        (
            true,
            HANDLER_CONCURRENCY,
            &json!("finished"),
            (seed_count, seed_count),
            vec![&json!("running"), &json!("finished")],
            true,
        )
    );
}
