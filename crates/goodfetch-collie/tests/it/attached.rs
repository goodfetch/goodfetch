//! An Attached engine over stdio: `hello`, refused before any other message
//! and then closed, `engine.info`, `$/cancel` and every failure's shape,
//! read through `StrictStdout` with the engine's logs at `trace`, so a log
//! line on stdout fails the test (ADRs 0024, 0066 and 0095).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::Write as _;
use std::process::{ChildStdin, Command, Stdio};

use goodfetch_protocol::jsonrpc::Message;
use goodfetch_test_support::{Process, StrictStdout, TestEnv, record_command};
use serde_json::{Value, json};

/// A running Attached engine of Collie's executable, started in a test's
/// setup.
pub(crate) struct Attached<'env> {
    env: &'env TestEnv,
    process: Process,
    stdin: Option<ChildStdin>,
    stdout: StrictStdout,
}

impl<'env> Attached<'env> {
    pub(crate) fn start(env: &'env TestEnv) -> Self {
        Self::start_with(env, |_| {})
    }

    /// Starts it after `configure` changed its command, such as its
    /// environment.
    pub(crate) fn start_with(env: &'env TestEnv, configure: impl FnOnce(&mut Command)) -> Self {
        let mut command = env.process_command(env!("CARGO_BIN_EXE_goodfetch-collie"));
        command
            .arg("attached-engine")
            .env("GOODFETCH_LOG", "trace")
            .stderr(Stdio::piped());
        configure(&mut command);
        let process = env.spawn(&mut command).unwrap();
        let stdin = process.child().stdin.take();
        let stdout = StrictStdout::of(&process).unwrap();
        Self {
            env,
            process,
            stdin,
            stdout,
        }
    }

    pub(crate) fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        stdin.write_all(&line).unwrap();
        stdin.flush().unwrap();
    }

    /// The next message, read as one of the protocol's (ADR 0038).
    pub(crate) fn next(&mut self) -> Option<Value> {
        let message = self.stdout.try_next().unwrap()?;
        serde_json::from_value::<Message>(message.clone()).unwrap();
        Some(message)
    }

    /// Whether the engine writes nothing on its stdout for `quiet`; a
    /// message written meanwhile is kept for the next read.
    pub(crate) fn is_quiet_for(&mut self, quiet: std::time::Duration) -> bool {
        self.stdout.is_quiet_for(quiet)
    }

    /// Ends the input and waits for the engine to exit, returning whatever
    /// it still sent.
    /// Records `attached-engine` and its exit code in the test's command
    /// record, for the exercise audit (ADR 0072).
    pub(crate) fn finish(mut self) -> (Vec<Value>, bool) {
        drop(self.stdin.take());
        let mut rest = Vec::new();
        while let Some(message) = self.next() {
            rest.push(message);
        }
        let status = self.process.child().wait().unwrap();
        record_command(self.env, &["attached-engine"], status.code()).unwrap();
        (rest, status.success())
    }
}

pub(crate) fn hello(needs: &str, protocol_version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": protocol_version,
            "surface": "sdk_python",
            "needs": needs,
            "product_version": "0.0.0",
        },
    })
}

fn request(id: i64, method: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} })
}

/// Covers: story:3-1, story:3-3, command:attached-engine
#[test]
fn an_attached_engine_answers_hello_over_stdio_and_stops_at_the_end_of_its_input() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);

    engine.send(&hello("collie", "0.1"));
    let reply = engine.next().unwrap();

    assert_eq!(
        reply,
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "result": { "protocol_version": "0.1", "product": "collie", "product_version": "0.0.0" },
        })
    );
    assert_eq!(engine.finish(), (vec![], true));
}

/// Covers: story:3-132, error:protocol_version_unsupported
#[test]
fn a_hello_of_a_newer_minor_is_refused_before_any_other_message_and_stdout_closes() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);

    engine.send(&hello("collie", "0.2"));
    let reply = engine.next().unwrap();
    let after = engine.next();

    assert_eq!(
        (&reply["error"]["data"]["code"], after),
        (&json!("protocol_version_unsupported"), None)
    );
    assert_eq!(engine.finish(), (vec![], true));
}

/// Covers: story:3-135, error:not_in_product
#[test]
fn collies_engine_refuses_a_client_that_needs_foxhound_and_stdout_closes() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);

    engine.send(&hello("foxhound", "0.1"));
    let reply = engine.next().unwrap();
    let after = engine.next();

    assert_eq!(
        (
            &reply["error"]["data"]["code"],
            &reply["error"]["data"]["needs"],
            after
        ),
        (&json!("not_in_product"), &json!("foxhound"), None)
    );
    assert_eq!(engine.finish(), (vec![], true));
}

/// Covers: operation:engine.info
#[test]
fn engine_info_answers_collie_and_its_versions() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();

    engine.send(&request(1, "engine.info"));
    let reply = engine.next().unwrap();

    assert_eq!(
        reply["result"],
        json!({
            "product": "collie",
            "product_version": "0.0.0",
            "protocol_version": "0.1",
            "product_major": 0,
        })
    );
    let _ = engine.finish();
}

/// Covers: story:3-136, error:not_in_product
#[test]
fn an_operation_only_foxhound_carries_is_refused_naming_foxhound_and_its_install() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();

    engine.send(&request(1, "sample.crawl"));
    let reply = engine.next().unwrap();

    assert_eq!(
        reply["error"],
        json!({
            "code": -32006,
            "message": "`sample.crawl` is part of Foxhound, and this engine is Collie 0.0.0.",
            "data": {
                "code": "not_in_product",
                "retryable": false,
                "product": "collie",
                "product_version": "0.0.0",
                "needs": "foxhound",
                "requested": "sample.crawl",
                "fix": "Run it with Foxhound, which contains everything Collie does (`cargo install goodfetch-foxhound --locked --features executable`), or connect to a Foxhound engine.",
            },
        })
    );
    let _ = engine.finish();
}

/// Covers: story:3-6
#[test]
fn cancel_of_a_finished_call_leaves_the_connection_answering() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    engine.send(&request(1, "engine.info"));
    let _ = engine.next();

    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 1 } }));
    engine.send(&request(2, "engine.info"));
    let reply = engine.next().unwrap();

    assert_eq!(
        (&reply["id"], &reply["result"]["product"]),
        (&json!(2), &json!("collie"))
    );
    assert_eq!(engine.finish(), (vec![], true));
}

/// Covers: story:3-7
#[test]
fn a_failure_carries_its_error_code_and_retryable_flag_under_its_json_rpc_class() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();

    engine.send(&request(1, "page.levitate"));
    let reply = engine.next().unwrap();

    assert_eq!(
        reply["error"],
        json!({
            "code": -32601,
            "message": "no Product carries the operation `page.levitate`",
            "data": { "code": "method_not_found", "retryable": false },
        })
    );
    let _ = engine.finish();
}
