//! An interrupted command whose call is cancellable sends `$/cancel`, and
//! the engine stops the work (ADRs 0069 and 0080), through Foxhound's
//! executable as through Collie's: `events subscribe --follow` stays in
//! flight until `SIGINT`, which the engine answers by cancelling the
//! subscription, and the command exits `130`.

use std::io::{BufRead as _, BufReader, Read as _};
use std::process::Stdio;

use goodfetch_test_support::{DISPATCH_TARGET, TestEnv, record_command};
use serde_json::{Value, json};

use crate::client_conformance::FOXHOUND;

const ARGUMENTS: [&str; 6] = [
    "events",
    "subscribe",
    "--after",
    r#"{"engine_instance":"another-engine","position":100}"#,
    "--follow",
    "--json",
];

/// Covers: story:11-198, command:events.subscribe, exit:interrupted
#[test]
fn attached() {
    let env = TestEnv::new();
    env.log_level("trace");
    let mut command = env.process_command(FOXHOUND);
    command
        .args(ARGUMENTS)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();
    let mut stdout = BufReader::new(process.child().stdout.take().unwrap());
    // A position another engine instance gave starts with `events.resync`:
    // the subscription is in flight once it is printed.
    let mut resync = String::new();
    stdout.read_line(&mut resync).unwrap();

    process.interrupt().unwrap();

    let status = process.child().wait().unwrap();
    record_command(&env, &ARGUMENTS, status.code()).unwrap();
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    let mut stderr = String::new();
    let mut pipe = process.child().stderr.take().unwrap();
    pipe.read_to_string(&mut stderr).unwrap();
    let resync: Value = serde_json::from_str(&resync).unwrap();
    assert_eq!(
        (resync["event"].clone(), rest.as_str(), status.code()),
        (json!("events.resync"), "", Some(130))
    );
    let records: Vec<&str> = stderr
        .lines()
        .filter_map(|line| line.split_once(&format!("{DISPATCH_TARGET}: ")))
        .map(|(_, record)| record)
        .collect();
    assert_eq!(
        records,
        [
            r#"request answered method="hello""#,
            r#"event sent event="events.resync""#,
            r#"cancel received notification="$/cancel""#,
            r#"request failed method="events.subscribe" error_code="call_cancelled""#,
        ]
    );
}
