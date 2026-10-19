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
