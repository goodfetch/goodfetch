//! A Foxhound Attached engine over stdio accepts a Client that needs
//! Foxhound and answers what only Foxhound carries (ADR 0095), read through
//! `StrictStdout` with the engine's logs at `trace`.

use std::io::Write as _;
use std::process::Stdio;

use goodfetch_test_support::{StrictStdout, TestEnv};
use serde_json::{Value, json};

fn line(message: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).unwrap();
    line.push(b'\n');
    line
}

/// Covers: story:3-135
#[test]
fn foxhounds_engine_accepts_a_client_that_needs_foxhound_and_answers_its_operations() {
    let env = TestEnv::new();
    env.log_level("trace");
    let mut command = env.process_command(env!("CARGO_BIN_EXE_goodfetch-foxhound"));
    command.arg("attached-engine").stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();
    let mut stdin = process.child().stdin.take().unwrap();
    let mut stdout = StrictStdout::of(&process).unwrap();

    stdin
        .write_all(&line(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "hello",
            "params": {
                "protocol_version": "0.1",
                "surface": "sdk_typescript",
                "needs": "foxhound",
                "product_version": "0.0.0",
            },
        })))
        .unwrap();
    stdin
        .write_all(&line(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sample.crawl",
            "params": { "echo": { "text": "a" }, "pages": 3 },
        })))
        .unwrap();
    drop(stdin);
    let replies: Vec<Value> = std::iter::from_fn(|| stdout.try_next().unwrap()).collect();
    let status = process.child().wait().unwrap();

    assert_eq!(
        (
            &replies[0]["result"]["product"],
            &replies[1]["result"],
            status.success()
        ),
        (&json!("foxhound"), &json!({ "visited": 3 }), true)
    );
}
