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
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::time::Duration;

use goodfetch_protocol::jsonrpc::Message;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{AnsweredCalls, Process, StrictStdout, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

/// The Datasets a crawl these tests start declares when its `crawl.start`
/// names none (ADR 0103): those their handlers push Records into, with no
/// schema or key.
const PUSHED_INTO: [&str; 4] = ["pages", "books", "items", "links"];

/// A running Attached engine of Foxhound's executable. When it ends
/// without exiting by itself, killed or left to the test's teardown, it
/// records the calls it read answered in the test's answer record: the
/// killed engine's log loses the dispatch records its writer's thread had
/// not written yet (ADR 0122), which the exercise audit reads (#1779).
pub(crate) struct Attached {
    process: Process,
    stdin: Option<ChildStdin>,
    stdout: StrictStdout,
    /// The engine instance its `hello` named.
    engine_instance: Value,
    /// The test's folder, which holds its answer record.
    folder: PathBuf,
    answered: AnsweredCalls,
    /// Whether the engine exited by itself, its log written whole.
    exited: bool,
}

impl Attached {
    pub(crate) fn start(env: &TestEnv) -> Self {
        Self::start_with(env, None, |_| {})
    }

    /// [`Attached::start`], `prepare` setting the engine's command first,
    /// such as the browsers' folder its pages launch from, and each wait
    /// for its output bounded by `deadline` when given.
    pub(crate) fn start_with(
        env: &TestEnv,
        deadline: Option<Duration>,
        prepare: impl FnOnce(&mut Command),
    ) -> Self {
        env.log_level("trace");
        let mut command = env.process_command(env!("CARGO_BIN_EXE_goodfetch-foxhound"));
        command.arg("attached-engine");
        prepare(&mut command);
        Self::spawned(env, deadline, &mut command)
    }

    /// An Attached engine `command` starts, such as Foxhound's executable
    /// run in a mount namespace of its own, each wait for its output
    /// bounded by `deadline` when given.
    pub(crate) fn spawned(
        env: &TestEnv,
        deadline: Option<Duration>,
        command: &mut Command,
    ) -> Self {
        env.log_level("trace");
        command.stderr(Stdio::piped());
        let process = env.spawn(command).unwrap();
        let stdin = process.child().stdin.take();
        let stdout = StrictStdout::of(&process).unwrap();
        let stdout = match deadline {
            Some(deadline) => stdout.within(deadline),
            None => stdout,
        };
        let mut engine = Self {
            process,
            stdin,
            stdout,
            engine_instance: Value::Null,
            folder: env.folder().to_owned(),
            answered: AnsweredCalls::default(),
            exited: false,
        };
        engine.send(&json!({
            "jsonrpc": "2.0", "id": "hello", "method": "hello",
            "params": { "protocol_version": "0.1", "surface": "sdk_python", "needs": "foxhound", "product_version": "0.0.0" },
        }));
        let opened = engine.next().unwrap();
        assert_eq!(opened["result"]["product"], "foxhound");
        engine.engine_instance = opened["result"]["engine_instance"].clone();
        engine
    }

    /// Sends `message`; a `crawl.start` that declares no Dataset declares
    /// [`PUSHED_INTO`], those the tests' handlers push their Records into.
    pub(crate) fn send(&mut self, message: &Value) {
        let mut message = message.clone();
        if message["method"] == "crawl.start" && message["params"].get("datasets").is_none() {
            message["params"]["datasets"] = PUSHED_INTO
                .iter()
                .map(|name| json!({ "name": name }))
                .collect();
        }
        self.answered.sent(&message);
        let stdin = self.stdin.as_mut().unwrap();
        let mut line = serde_json::to_vec(&message).unwrap();
        line.push(b'\n');
        stdin.write_all(&line).unwrap();
        stdin.flush().unwrap();
    }

    pub(crate) fn next(&mut self) -> Option<Value> {
        let message = self.stdout.try_next().unwrap()?;
        serde_json::from_value::<Message>(message.clone()).unwrap();
        self.answered.read(&message);
        Some(message)
    }

    /// Sends the call `id`, answers each handler request with `handler`
    /// until its reply comes, and answers that reply.
    pub(crate) fn call(
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
    pub(crate) fn is_quiet_for(&mut self, quiet: Duration) -> bool {
        self.stdout.is_quiet_for(quiet)
    }

    /// Subscribes to the event stream as the call `id`, from the start of
    /// the engine instance its `hello` named: it reads every event, even
    /// those of calls that answered before it was armed.
    pub(crate) fn subscribe_from_start(&mut self, id: &str) {
        let after = json!({ "engine_instance": self.engine_instance, "position": 0 });
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "events.subscribe", "params": { "after": after },
        }));
    }

    /// The engine's process ID.
    pub(crate) fn pid(&self) -> u32 {
        self.process.child().id()
    }

    /// Kills the engine (`kill -9`, `TerminateProcess` on Windows) and
    /// waits for it to exit; the calls it answered go to the test's answer
    /// record.
    pub(crate) fn kill(self) {
        self.process.stop().unwrap();
    }

    /// Ends the input and waits for the engine to exit.
    pub(crate) fn finish(mut self) -> bool {
        drop(self.stdin.take());
        while self.next().is_some() {}
        self.exited = self.process.child().wait().unwrap().success();
        self.exited
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        // A folder already gone means the test's teardown ran, copying the
        // record that nothing can add to any more.
        if self.exited || !self.folder.is_dir() {
            return;
        }
        if let Err(error) = self.answered.record_in(&self.folder)
            && !std::thread::panicking()
        {
            panic!("the test's answer record cannot be written: {error}");
        }
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

    let file = Connection::open(HomeLayout::of(env.home()).run_file("shop")).unwrap();
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
/// run state (ADR 0103), and its queue still gets every axis change. It
/// subscribes once `crawl.start` answered, from the start of the engine
/// instance its `hello` named: the engine runs a Connection's calls
/// concurrently, so a subscription sent before `crawl.start` may still be
/// armed after the Run's first axis change.
///
/// Covers: story:3-8, event:run.axes_changed
#[test]
fn a_slow_handler_holds_the_crawl_back_and_a_slow_subscriber_loses_no_record_or_axis_change() {
    const SEEDS: usize = HANDLER_CONCURRENCY + 4;
    let seed_count = i64::try_from(SEEDS).unwrap();
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
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
            engine.subscribe_from_start("events");
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
    // The axis changes, lossless; the Run's progress values beside them are
    // telemetry, coalesced.
    let mut events = Vec::new();
    while events.len() < 2 {
        let message = engine.next().unwrap();
        if message["method"] == "$/events" {
            events.extend(
                message["params"]["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|event| event["event"] == "run.axes_changed")
                    .cloned(),
            );
        }
    }
    let exited = engine.finish();

    let file = Connection::open(HomeLayout::of(env.home()).run_file("held")).unwrap();
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

/// Each ending of a crawl, read back on Foxhound's executable once the Run
/// ended: `run.outcomes` in commit order with a filter run in the engine,
/// and `run.failed_requests` with `blocked`, which lists the failed and the
/// blocked requests and never the skipped one.
///
/// Covers: story:3-149, story:3-150, operation:run.outcomes, operation:run.failed_requests
#[test]
fn a_crawls_outcomes_and_failed_requests_are_read_on_foxhounds_executable() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    let seeds = json!([
        { "url": "https://shop.test/kept", "label": "page" },
        { "url": "https://shop.test/skipped", "label": "page" },
        { "url": "https://shop.test/blocked", "label": "page" },
        { "url": "https://shop.test/failed", "label": "page" },
    ]);
    engine.call(
        "start",
        "crawl.start",
        &json!({ "run_id": "endings", "seeds": seeds }),
        &mut |_| None,
    );
    let mut handler = |params: &Value| {
        let failure = |code: &str, extra: Value| {
            let mut data = json!({ "code": code, "retryable": false });
            data.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            json!({ "error": { "code": -32003, "message": code, "data": data } })
        };
        Some(match params["crawl_request"]["url"].as_str().unwrap() {
            "https://shop.test/kept" => {
                json!({ "result": { "records": [{ "dataset": "pages", "data": {} }] } })
            }
            "https://shop.test/skipped" => {
                json!({ "result": { "skipped": { "message": "out of season" } } })
            }
            "https://shop.test/blocked" => failure("blocked", json!({ "cause": "captcha" })),
            _ => failure("handler_failed", json!({})),
        })
    };
    engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "endings" }),
        &mut handler,
    );

    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "endings", "kinds": ["succeeded", "skipped", "blocked"] }),
        &mut |_| None,
    );
    let failed = engine.call(
        "failed",
        "run.failed_requests",
        &json!({ "run_id": "endings", "blocked": true }),
        &mut |_| None,
    );
    let exited = engine.finish();

    // The handler takes the four at once, so their commit order is the
    // runtime's: the in-process tests name it, this one sorts by URL.
    let endings = |answer: &Value, list: &str| -> Vec<(String, Value)> {
        let mut endings: Vec<(String, Value)> = answer["result"][list]
            .as_array()
            .unwrap()
            .iter()
            .map(|outcome| {
                (
                    outcome["request"]["url"].as_str().unwrap().to_owned(),
                    outcome["kind"].clone(),
                )
            })
            .collect();
        endings.sort_by(|left, right| left.0.cmp(&right.0));
        endings
    };
    assert_eq!(
        (
            endings(&outcomes, "outcomes"),
            &outcomes["result"]["ended"],
            endings(&failed, "requests"),
            exited,
        ),
        (
            vec![
                ("https://shop.test/blocked".to_owned(), json!("blocked")),
                ("https://shop.test/kept".to_owned(), json!("succeeded")),
                ("https://shop.test/skipped".to_owned(), json!("skipped")),
            ],
            &json!(true),
            vec![
                ("https://shop.test/blocked".to_owned(), json!("blocked")),
                ("https://shop.test/failed".to_owned(), json!("failed")),
            ],
            true,
        )
    );
}

/// The handler of the keep-alive crawls: one Record naming each page.
fn record_the_page(params: &Value) -> Value {
    json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": params["crawl_request"]["url"] } }] } })
}

/// Starts the keep-alive crawl `run_id` of one seed with `timeout.run`
/// when given.
fn start_keep_alive(engine: &mut Attached, run_id: &str, timeout: Option<u64>) {
    let mut settings = json!({ "crawl": { "keep_alive": true } });
    if let Some(timeout) = timeout {
        settings["timeout"] = json!({ "run": timeout });
    }
    engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": [{ "url": "https://shop.test/a", "label": "page" }],
            "settings": settings,
        }),
        &mut |_| None,
    );
}

/// A keep-alive crawl on Foxhound's executable (RT-12): once its one seed
/// is handled, it waits for new work. `crawl.serve` waits for its end
/// without letting it finish, so its short `timeout.run` stops it; a crawl
/// without `crawl.keep_alive` would have finished at once.
///
/// Covers: story:3-128, setting:crawl.keep_alive, operation:crawl.serve
#[test]
fn a_keep_alive_crawl_on_foxhounds_executable_waits_for_new_work_until_its_run_timeout() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    start_keep_alive(&mut engine, "waiting", Some(1_000));

    let report = engine.call(
        "serve",
        "crawl.serve",
        &json!({ "run_id": "waiting" }),
        &mut |params| Some(record_the_page(params)),
    );
    let exited = engine.finish();

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            &report["result"]["stop_reason"],
            exited,
        ),
        (
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "timeout" }),
            true,
        )
    );
}

/// `run.finish` lets an idle keep-alive crawl on Foxhound's executable
/// end, `finished` and `complete`.
///
/// Covers: story:3-128, setting:crawl.keep_alive, operation:run.finish
#[test]
fn run_finish_ends_a_keep_alive_crawl_on_foxhounds_executable_once_drained() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    start_keep_alive(&mut engine, "served", None);
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "served" }),
        &mut |params| Some(record_the_page(params)),
    );

    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "served" }),
        &mut |params| Some(record_the_page(params)),
    );
    let exited = engine.finish();

    assert_eq!(
        (
            &outcomes["result"]["outcomes"][0]["kind"],
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
            exited,
        ),
        (
            &json!("succeeded"),
            &json!("finished"),
            &json!("complete"),
            true
        )
    );
}
