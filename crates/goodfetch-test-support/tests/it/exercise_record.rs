//! The exercise record ADR 0072's audit reads: `TestEnv`'s map turns on the
//! JSON log of the engine a test starts, and the test `tracing` subscriber
//! writes an in-process test's dispatch records into the record, as the
//! global one test support installs does for a test with none.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::Stdio;

use goodfetch_test_support::{
    DISPATCH_TARGET, OUTCOME_TARGET, RECORDS_VARIABLE, TestEnv, Tool, tools,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::probe;

/// Every JSON line of every file in `folder`, in the files' order.
fn json_lines(folder: &Path) -> Vec<Value> {
    let mut files: Vec<_> = std::fs::read_dir(folder)
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

/// The method of each dispatch record among `lines`.
fn dispatched(lines: &[Value]) -> Vec<Value> {
    lines
        .iter()
        .filter(|line| line["target"] == DISPATCH_TARGET)
        .map(|line| line["fields"]["method"].clone())
        .collect()
}

/// The Error code of each outcome record among `lines`.
fn recorded(lines: &[Value]) -> Vec<Value> {
    lines
        .iter()
        .filter(|line| line["target"] == OUTCOME_TARGET)
        .map(|line| line["fields"]["error_code"].clone())
        .collect()
}

#[test]
fn the_test_env_map_turns_on_the_json_log_of_the_engine_a_test_starts() {
    let env = TestEnv::new();
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_owned();
    let mut command = env.process_command(collie);
    command.arg("attached-engine");
    let process = env.spawn(&mut command).unwrap();
    let mut stdin = process.child().stdin.take().unwrap();
    let hello = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": "0.1",
            "surface": "sdk_python",
            "needs": "collie",
            "product_version": "0.0.0",
        },
    });
    writeln!(stdin, "{hello}").unwrap();
    let stdout = process.child().stdout.take().unwrap();
    let _reply = BufReader::new(stdout).lines().next();

    drop(stdin);
    process.child().wait().unwrap();

    assert_eq!(
        dispatched(&json_lines(&env.home().join("logs"))),
        [json!("hello")]
    );
}

#[test]
fn the_test_subscriber_writes_the_dispatch_and_outcome_records_into_the_exercise_record() {
    let env = TestEnv::new();
    let records = env.folder().join("records");
    let mut command = probe(&env, "probe::dispatches_under_the_test_subscriber");
    command
        .env(RECORDS_VARIABLE, &records)
        .env("NEXTEST_BINARY_ID", "sample::it")
        .env("NEXTEST_TEST_NAME", "engine_info");

    let status = command.status().unwrap();

    assert!(status.success());
    let lines = json_lines(&records);
    assert_eq!(
        (&lines[0], dispatched(&lines[1..]), recorded(&lines[1..])),
        (
            &json!({ "test": "sample::it engine_info" }),
            vec![json!("engine.info")],
            vec![json!("handler_lost")]
        )
    );
}

/// A test that installs no subscriber, such as one that links the engine,
/// whose threads log to the process's global subscriber, still writes the
/// dispatch and outcome records of another thread into its record, from
/// its `TestEnv` or its test runtime, whichever comes first.
#[rstest]
#[case::from_its_test_env("probe::dispatches_from_another_thread_after_a_test_env")]
#[case::from_its_runtime("probe::dispatches_from_another_thread_on_a_test_runtime")]
fn a_test_with_no_subscriber_of_its_own_records_what_another_thread_dispatches(
    #[case] child: &str,
) {
    let env = TestEnv::new();
    let records = env.folder().join("records");
    let mut command = probe(&env, child);
    command
        .env(RECORDS_VARIABLE, &records)
        .env("NEXTEST_BINARY_ID", "sample::it")
        .env("NEXTEST_TEST_NAME", "engine_info");

    let status = command.status().unwrap();

    assert!(status.success());
    let lines = json_lines(&records);
    assert_eq!(
        (&lines[0], dispatched(&lines[1..]), recorded(&lines[1..])),
        (
            &json!({ "test": "sample::it engine_info" }),
            vec![json!("engine.info")],
            vec![json!("handler_lost")]
        )
    );
}

/// `subscriber_unless_own` installs the test subscriber over the one test
/// support installs for a test with none, and not over a test's own.
#[test]
fn the_test_subscriber_goes_on_unless_the_test_installed_its_own() {
    let env = TestEnv::new();
    let records = env.folder().join("records");
    let mut command = probe(
        &env,
        "probe::installs_the_test_subscriber_unless_it_has_its_own",
    );
    command
        .env(RECORDS_VARIABLE, &records)
        .env("NEXTEST_BINARY_ID", "sample::it")
        .env("NEXTEST_TEST_NAME", "engine_info")
        .stderr(Stdio::piped());

    let output = command.output().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
