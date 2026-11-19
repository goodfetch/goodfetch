//! `goodfetch-scripted-engine` over stdio (ADR 0069, Seam B): a tiny Client
//! written here reaches it as an SDK reaches an Attached engine, through
//! its Product's engine-path variable in an environment map, and the
//! executable plays the corpus's scripts to it: the shared self-test
//! scripts, and the refused `hello`, the crash mid-call and the garbage on
//! stdout. It fails on a message no step expects and on a step left unused.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ExitStatus, Stdio};
use std::time::Duration;

use goodfetch_test_support::{
    ATTACHED_RECORD, ATTACHED_SCRIPT, Process, StrictStdout, StrictStdoutError,
    StrictStdoutErrorKind, TestEnv, engine_script,
};
use rstest::rstest;
use serde_json::{Value, json};

/// This commit's `goodfetch-scripted-engine`.
const SCRIPTED_ENGINE: &str = env!("CARGO_BIN_EXE_goodfetch-scripted-engine");

/// The environment map an SDK of `product` takes, as the test-support
/// fixtures build it: the cleared map, with the Product's engine-path
/// variable naming the Scripted engine, which plays `script` from the
/// test's Goodfetch home.
fn engine_map(env: &TestEnv, product: &str, script: &str) -> BTreeMap<String, String> {
    let installed = env.home().join(ATTACHED_SCRIPT);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::copy(engine_script(script), &installed).unwrap();
    let mut map = env.env().clone();
    map.insert(
        format!("GOODFETCH_{}_ENGINE_PATH", product.to_uppercase()),
        SCRIPTED_ENGINE.to_owned(),
    );
    map
}

/// A Client of one Product, as small as a test can write: it starts its
/// Attached engine from its own Product's engine-path variable, with
/// exactly its environment map, and speaks newline-delimited JSON-RPC.
struct TinyClient {
    product: &'static str,
    process: Process,
    stdin: Option<ChildStdin>,
    stdout: StrictStdout,
    stderr: ChildStderr,
    next_id: i64,
}

/// What the tiny Client reads from an accepted `hello`.
#[derive(Debug, PartialEq, Eq)]
struct Engine {
    product: String,
    product_version: String,
}

impl TinyClient {
    fn connect(env: &TestEnv, product: &'static str, map: &BTreeMap<String, String>) -> Self {
        let variable = format!("GOODFETCH_{}_ENGINE_PATH", product.to_uppercase());
        let mut command = std::process::Command::new(&map[&variable]);
        command
            .arg("attached-engine")
            .env_clear()
            .envs(map)
            .current_dir(env.folder())
            .stderr(Stdio::piped());
        Self::start(env, product, &mut command)
    }

    fn start(env: &TestEnv, product: &'static str, command: &mut std::process::Command) -> Self {
        let process = env.spawn(command).unwrap();
        let (stdin, stdout, stderr) = {
            let mut child = process.child();
            (
                child.stdin.take(),
                child.stdout.take().unwrap(),
                child.stderr.take().unwrap(),
            )
        };
        Self {
            product,
            process,
            stdin,
            stdout: StrictStdout::new(stdout, std::io::empty()),
            stderr,
            next_id: 0,
        }
    }

    /// The same Client, waiting at most `deadline` for each message, as an
    /// SDK's own short timeout does.
    fn within(self, deadline: Duration) -> Self {
        Self {
            stdout: self.stdout.within(deadline),
            ..self
        }
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        stdin.write_all(&line).unwrap();
        stdin.flush().unwrap();
    }

    /// Sends a request and returns its ID.
    fn request(&mut self, method: &str, params: &Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    /// Says `hello` as an SDK of its Product at `protocol_version`.
    fn hello(&mut self, protocol_version: &str, product_version: &str) -> i64 {
        self.request(
            "hello",
            &json!({
                "protocol_version": protocol_version,
                "surface": "sdk_typescript",
                "form": "typescript",
                "needs": self.product,
                "product_version": product_version,
            }),
        )
    }

    fn next(&mut self) -> Result<Option<Value>, StrictStdoutError> {
        self.stdout.try_next()
    }

    /// The Product and Product version an accepted `hello` names.
    fn engine(reply: &Value) -> Engine {
        Engine {
            product: reply["result"]["product"].as_str().unwrap().to_owned(),
            product_version: reply["result"]["product_version"]
                .as_str()
                .unwrap()
                .to_owned(),
        }
    }

    /// Closes its end and waits for the engine to exit.
    fn close(mut self) -> (ExitStatus, String) {
        drop(self.stdin.take());
        let status = self.process.child().wait().unwrap();
        let mut stderr = String::new();
        self.stderr.read_to_string(&mut stderr).unwrap();
        (status, stderr)
    }
}

/// The failure code the Scripted engine's stderr opens with.
fn failure(stderr: &str) -> &str {
    stderr
        .strip_prefix("goodfetch-scripted-engine: ")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or(stderr)
}

#[rstest]
#[case::collie("self-test/hello-as-collie.json", "collie", "0.1.0")]
#[case::foxhound("self-test/hello-as-foxhound.json", "foxhound", "0.2.0")]
fn a_script_replying_as_a_product_is_read_as_that_products_engine(
    #[case] script: &str,
    #[case] product: &str,
    #[case] product_version: &str,
) {
    let env = TestEnv::new();
    let mut client = TinyClient::connect(&env, "collie", &engine_map(&env, "collie", script));

    client.hello("0.1", "0.1.0");
    let reply = client.next().unwrap().unwrap();

    assert_eq!(
        TinyClient::engine(&reply),
        Engine {
            product: product.to_owned(),
            product_version: product_version.to_owned(),
        }
    );
    assert_eq!(reply["id"], 0);
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_self_test_script_plays_a_call_to_its_end() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/call-answered.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();

    let id = client.request("engine.info", &json!({}));
    let reply = client.next().unwrap().unwrap();

    assert_eq!(
        reply,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "product": "collie", "product_version": "0.1.0", "protocol_version": "0.1", "product_major": 0 },
        })
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_script_refusing_the_clients_product_with_not_in_product_ends_the_exchange_there() {
    let env = TestEnv::new();
    let map = engine_map(
        &env,
        "foxhound",
        "self-test/hello-refused-not-in-product.json",
    );
    let mut client = TinyClient::connect(&env, "foxhound", &map);

    client.hello("0.1", "0.1.0");
    let reply = client.next().unwrap().unwrap();
    let after = client.next().unwrap();

    assert_eq!(
        (&reply["error"]["data"]["code"], after),
        (&json!("not_in_product"), None)
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_script_calls_the_clients_handler_and_sends_an_event_during_a_call() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/handler-call-and-event.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();

    let call = client.request("sample.echo", &json!({ "text": "hello" }));
    let handler = client.next().unwrap().unwrap();
    client.send(&json!({ "jsonrpc": "2.0", "id": handler["id"], "result": { "replaced": false } }));
    let event = client.next().unwrap().unwrap();
    let reply = client.next().unwrap().unwrap();

    assert_eq!(
        (
            &handler["method"],
            &handler["params"],
            &event,
            &reply["id"],
            &reply["result"]
        ),
        (
            &json!("sample_store.put"),
            &json!({ "key": "greeting", "value": "hello" }),
            &json!({ "jsonrpc": "2.0", "method": "sample.echoed", "params": { "text": "hello" } }),
            &json!(call),
            &json!({ "text": "hello" }),
        )
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_script_sends_events_once_the_client_grants_credits_then_closes() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/events-on-credit.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();
    let subscription = client.request("events.subscribe", &json!({}));

    client.send(&json!({
        "jsonrpc": "2.0",
        "method": "$/credit",
        "params": { "id": subscription, "credits": 1 },
    }));
    let batch = client.next().unwrap().unwrap();
    let after = client.next().unwrap();

    assert_eq!(
        (batch, after),
        (
            json!({
                "jsonrpc": "2.0",
                "method": "$/events",
                "params": {
                    "id": 1,
                    "events": [{ "position": 1, "event": "sample.echoed", "data": { "text": "hello" } }],
                },
            }),
            None
        )
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_reply_after_an_expected_cancel_answers_the_call_it_cancels() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/call-cancelled.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();
    let subscription = client.request("events.subscribe", &json!({}));

    client.send(&json!({
        "jsonrpc": "2.0",
        "method": "$/cancel",
        "params": { "id": subscription },
    }));
    let reply = client.next().unwrap().unwrap();

    assert_eq!(
        (reply["id"].clone(), reply["error"]["data"]["code"].clone()),
        (json!(subscription), json!("call_cancelled"))
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_stalled_script_answers_nothing_until_the_client_gives_up() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/stall.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();
    client.request("sample.echo", &json!({ "text": "hello" }));
    let mut client = client.within(Duration::from_millis(200));

    let waited = client.next().unwrap_err();

    assert!(
        matches!(waited.kind, StrictStdoutErrorKind::Deadline(_)),
        "{waited}"
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_message_no_step_expects_fails_the_engine() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/call-answered.json");
    let mut client = TinyClient::connect(&env, "collie", &map);

    client.hello("0.1", "9.9.9");
    let after = client.next().unwrap();

    let (status, stderr) = client.close();
    assert_eq!(
        (after, status.success(), failure(&stderr)),
        (None, false, "unexpected_message")
    );
}

#[test]
fn a_hello_without_the_jsonrpc_member_fails_the_engine() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/call-answered.json");
    let mut client = TinyClient::connect(&env, "collie", &map);

    client.send(&json!({
        "id": 1,
        "method": "hello",
        "params": {
            "protocol_version": "0.1",
            "surface": "sdk_typescript",
            "form": "typescript",
            "needs": "collie",
            "product_version": "0.1.0",
        },
    }));
    let after = client.next().unwrap();

    let (status, stderr) = client.close();
    assert_eq!(
        (after, status.success(), failure(&stderr)),
        (None, false, "unexpected_message")
    );
}

#[test]
fn an_answer_with_both_a_result_and_an_error_fails_the_engine() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/handler-call-and-event.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();
    client.request("sample.echo", &json!({ "text": "hello" }));
    let handler = client.next().unwrap().unwrap();

    client.send(&json!({
        "jsonrpc": "2.0",
        "id": handler["id"],
        "result": { "replaced": false },
        "error": { "code": -32000, "message": "both" },
    }));
    let after = client.next().unwrap();

    let (status, stderr) = client.close();
    assert_eq!(
        (after, status.success(), failure(&stderr)),
        (None, false, "unexpected_message")
    );
}

#[test]
fn a_step_left_unused_fails_the_engine() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/call-answered.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();

    let (status, stderr) = client.close();

    assert_eq!((status.success(), failure(&stderr)), (false, "unused_step"));
}

#[test]
fn the_refused_hello_script_gives_the_client_protocol_version_unsupported_and_closes() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "hello/protocol-version-unsupported.json");
    let mut client = TinyClient::connect(&env, "collie", &map);

    client.hello("0.2", "0.1.0");
    let reply = client.next().unwrap().unwrap();
    let after = client.next().unwrap();

    assert_eq!(
        (&reply["error"]["data"]["code"], after),
        (&json!("protocol_version_unsupported"), None)
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn the_crash_mid_call_script_leaves_the_client_a_dead_pipe() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "attached-engine/crash-mid-call.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();

    client.request("sample.echo", &json!({ "text": "hello" }));
    let after = client.next().unwrap();

    let (status, _) = client.close();
    assert_eq!((after, status.code()), (None, Some(101)));
}

#[test]
fn the_garbage_on_stdout_script_gives_the_client_a_line_that_is_not_json_rpc() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "attached-engine/garbage-on-stdout.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();

    let garbage = client.next().unwrap_err();

    assert!(
        matches!(&garbage.kind, StrictStdoutErrorKind::NotJson(line) if line.bytes == b"panicked at engine/src/lib.rs:1:1"),
        "{garbage}"
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
}

#[test]
fn the_engine_records_every_message_it_saw() {
    let env = TestEnv::new();
    let map = engine_map(&env, "collie", "self-test/call-answered.json");
    let mut client = TinyClient::connect(&env, "collie", &map);
    client.hello("0.1", "0.1.0");
    client.next().unwrap().unwrap();
    client.request("engine.info", &json!({}));
    client.next().unwrap().unwrap();
    client.close();

    let record = record_of(&env.home().join(ATTACHED_RECORD));

    assert_eq!(
        record
            .iter()
            .map(|entry| (
                entry["from"].as_str().unwrap(),
                entry["message"]["method"].as_str().unwrap_or("")
            ))
            .collect::<Vec<_>>(),
        [
            ("client", "hello"),
            ("engine", ""),
            ("client", "engine.info"),
            ("engine", "")
        ]
    );
}

#[test]
fn the_script_option_plays_a_script_named_on_the_command_line() {
    let env = TestEnv::new();
    let record: PathBuf = env.folder().join("record.jsonl");
    let mut command = env.process_command(SCRIPTED_ENGINE);
    command
        .arg("--script")
        .arg(engine_script("hello/protocol-version-unsupported.json"))
        .args(["--mode", "stdio", "--record"])
        .arg(&record)
        .stderr(Stdio::piped());
    let mut client = TinyClient::start(&env, "collie", &mut command);

    client.hello("0.2", "0.1.0");
    let reply = client.next().unwrap().unwrap();

    assert_eq!(
        reply["error"]["data"]["code"],
        json!("protocol_version_unsupported")
    );
    let (status, stderr) = client.close();
    assert!(status.success(), "{stderr}");
    assert_eq!(record_of(&record).len(), 2);
}

/// The record's entries, one JSON value per line.
fn record_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
