//! The Attached engine's JSON log in the Goodfetch home (ADR 0122): written
//! only when `GOODFETCH_LOG_OUTPUT` is `file`, as test support turns it on
//! for ADR 0072's exercise audit, which reads `rpc`'s dispatch records in it.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the engine's log files once it has exited"
)]

use std::path::Path;

use serde_json::{Value, json};

use goodfetch_test_support::TestEnv;

use crate::attached::{Attached, hello};

/// Every JSON line of the log files in the home's log folder.
fn log_lines(home: &Path) -> Vec<Value> {
    let mut files: Vec<_> = std::fs::read_dir(home.join("logs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
        .iter()
        .flat_map(|file| {
            std::fs::read_to_string(file)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<Value>>()
        })
        .collect()
}

#[test]
fn with_log_output_file_the_engine_writes_its_dispatch_records_as_json_lines_into_the_home() {
    let env = TestEnv::new();
    let mut engine = Attached::start_with(&env, |command| {
        command.env("GOODFETCH_LOG_OUTPUT", "file");
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    engine.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "sample.crawl", "params": {} }));
    let _ = engine.next();

    let _ = engine.finish();

    let dispatched: Vec<(Value, Value)> = log_lines(env.home())
        .into_iter()
        .filter(|line| line["target"] == "goodfetch::rpc::dispatch")
        .map(|line| {
            (
                line["fields"]["method"].clone(),
                line["fields"]["error_code"].clone(),
            )
        })
        .collect();
    assert_eq!(
        dispatched,
        vec![
            (json!("hello"), Value::Null),
            (json!("sample.crawl"), json!("not_in_product")),
        ]
    );
}

#[test]
fn without_log_output_the_engine_writes_no_log_file() {
    let env = TestEnv::new();
    let mut engine = Attached::start_with(&env, |command| {
        command.env_remove("GOODFETCH_LOG_OUTPUT");
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();

    let _ = engine.finish();

    assert!(!env.home().join("logs").exists());
}

#[test]
fn an_unknown_log_output_fails_the_start() {
    let env = TestEnv::new();
    let engine = Attached::start_with(&env, |command| {
        command.env("GOODFETCH_LOG_OUTPUT", "stdout");
    });

    let (sent, succeeded) = engine.finish();

    assert_eq!((sent, succeeded), (vec![], false));
}
