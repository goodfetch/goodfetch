//! `goodfetch-conformance write <case list> <header> <case log> <document>`:
//! the command line through which a runner that is not written in Rust, such
//! as the Vitest reporter's tests, turns its case log into a `conformance`
//! result document with the writer (ADR 0069). It reads the Client
//! conformance case list as `conformance/clients/cases.toml` holds it, its
//! `surfaces`, `modes`, `script` and `task` fields included. It exits 0 when
//! it wrote the document and 1 otherwise.

use goodfetch_protocol::result_document::{LoadedDocument, Outcome, read};
use goodfetch_test_support::{TestEnv, command};
use serde_json::{Value, json};

const CONFORMANCE: &str = env!("CARGO_BIN_EXE_goodfetch-conformance");

/// Two capabilities of one case each, as the Client conformance case list
/// declares them.
const CASE_LIST: &str = r#"
[suite]
name = "clients"
version = "1"

[[capability]]
name = "hello"

[[capability.case]]
id = "hello-refused"
covers = ["story:11-1"]
surfaces = ["sdk-typescript", "cli"]
script = "hello/refused.json"

[[capability]]
name = "cancel"

[[capability.case]]
id = "cancel-mid-run"
covers = ["story:11-4"]
surfaces = ["sdk-typescript"]
modes = ["attached"]
task = "catalog/1"
"#;

fn header() -> Value {
    json!({
        "suite": { "name": "clients", "version": "1" },
        "target": { "name": "goodfetch-collie", "version": "0.0.0" },
        "platform": { "os": "linux", "arch": "x86_64" },
        "runtime": "node-24",
        "proving_ground": { "version": "0.0.0", "seed": 396 },
        "online": false,
    })
}

/// One entry as the Vitest reporter writes it, its one check holding as
/// `passed` says.
fn entry(case: &str, capability: &str, passed: bool) -> Value {
    json!({
        "case": case,
        "capability": capability,
        "mode": "attached",
        "platform": { "os": "linux", "arch": "x86_64" },
        "runtime": "node-24",
        "checks": [{ "name": "the Client raises the Error code", "passed": passed }],
        "evidence": { "messages": [{ "method": "hello" }] },
    })
}

/// Writes the case list, the header and a log of `entries` into the test's
/// folder, and runs `write` on them.
fn write(env: &TestEnv, entries: &[Value]) -> (std::process::Output, std::path::PathBuf) {
    let folder = env.folder();
    std::fs::write(folder.join("cases.toml"), CASE_LIST).unwrap();
    std::fs::write(
        folder.join("header.json"),
        serde_json::to_vec(&header()).unwrap(),
    )
    .unwrap();
    let log: String = entries
        .iter()
        .map(|entry| entry.to_string() + "\n")
        .collect();
    std::fs::write(folder.join("case-log.jsonl"), log).unwrap();
    let document = folder.join("conformance.json");
    let output = command(env, CONFORMANCE)
        .arg("write")
        .arg(folder.join("cases.toml"))
        .arg(folder.join("header.json"))
        .arg(folder.join("case-log.jsonl"))
        .arg(&document)
        .output()
        .unwrap();
    (output, document)
}

#[test]
fn write_turns_a_passing_and_a_failing_case_into_a_document_the_reader_accepts() {
    let env = TestEnv::new();

    let (output, document) = write(
        &env,
        &[
            entry("hello-refused", "hello", true),
            entry("cancel-mid-run", "cancel", false),
        ],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let LoadedDocument::Conformance(loaded) = read(&std::fs::read(document).unwrap()).unwrap()
    else {
        panic!("the document is not a conformance document");
    };
    let outcomes: Vec<(&str, Outcome)> = loaded
        .capabilities()
        .iter()
        .map(|capability| (capability.name(), capability.outcome()))
        .collect();
    assert_eq!(
        outcomes,
        [("hello", Outcome::Proved), ("cancel", Outcome::Failed)]
    );
}

#[test]
fn write_refuses_a_log_entry_that_sets_an_outcome() {
    let env = TestEnv::new();
    let mut smuggled = entry("hello-refused", "hello", true);
    smuggled["outcome"] = json!("passed");

    let (output, document) = write(&env, &[smuggled]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!document.exists());
}
