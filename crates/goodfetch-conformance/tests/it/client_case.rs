//! The Rust runner's case-log writer (ADR 0069): a Rust test that runs a
//! Client conformance case appends one entry to the case log in the folder
//! it is given, with the case's capability from the case list, the mode,
//! this host's platform, the `rust` runtime, the checks it recorded and
//! whether the test passed, so a test that fails outside its checks fails its
//! case; the writer reads it back.

use std::path::PathBuf;

use goodfetch_conformance::{ClientCaseError, ClientCaseRun, RustMode, read_case_log};
use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::result_document::{CaseCheck, Evidence, Platform};
use goodfetch_test_support::TestEnv;
use serde_json::json;

/// The folder of the case logs in `env`'s folder, which the writer makes.
fn case_logs(env: &TestEnv) -> PathBuf {
    env.folder().join("case-logs")
}

/// The entries of the case log in `env`'s folder.
fn logged(env: &TestEnv) -> Vec<CaseLogEntry> {
    let text = std::fs::read_to_string(case_logs(env).join(ClientCaseRun::FILE)).unwrap();
    read_case_log(&text).unwrap()
}

fn check(name: &str, passed: bool) -> CaseCheck {
    CaseCheck {
        name: name.to_owned(),
        passed,
    }
}

fn this_host() -> Platform {
    Platform {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

#[test]
fn each_run_appends_its_case_with_the_lists_capability_and_whether_the_test_passed() {
    let env = TestEnv::new();
    let folder = case_logs(&env);

    let mut linked = ClientCaseRun::start(
        Some(&folder),
        "log-level-follows-the-logger",
        RustMode::Linked,
    )
    .unwrap();
    let level = linked.check("hello carries debug", || "debug");
    linked.message(json!({ "method": "hello", "params": { "log_level": level } }));
    drop(linked);
    let daemon = ClientCaseRun::start(
        Some(&folder),
        "engine-logs-reach-the-logger",
        RustMode::Daemon,
    )
    .unwrap();
    drop(daemon);

    assert_eq!(
        logged(&env),
        [
            CaseLogEntry {
                case: "log-level-follows-the-logger".to_owned(),
                capability: "logging".to_owned(),
                mode: Some("linked".to_owned()),
                platform: this_host(),
                runtime: "rust".to_owned(),
                checks: vec![
                    check("hello carries debug", true),
                    check("the test passed", true),
                ],
                evidence: Evidence {
                    messages: vec![
                        json!({ "method": "hello", "params": { "log_level": "debug" } })
                    ],
                    ..Evidence::default()
                },
            },
            CaseLogEntry {
                case: "engine-logs-reach-the-logger".to_owned(),
                capability: "logging".to_owned(),
                mode: Some("daemon".to_owned()),
                platform: this_host(),
                runtime: "rust".to_owned(),
                checks: vec![check("the test passed", true)],
                evidence: Evidence::default(),
            },
        ]
    );
}

#[test]
fn a_check_that_fails_is_logged_failed_and_fails_the_test() {
    let env = TestEnv::new();
    let folder = case_logs(&env);

    let test = std::thread::spawn(move || {
        let mut run = ClientCaseRun::start(
            Some(&folder),
            "log-level-follows-the-logger",
            RustMode::Daemon,
        )
        .unwrap();
        run.check("the start tells the engine warn", || {
            assert_eq!("debug", "warn");
        });
    })
    .join();

    assert!(test.is_err(), "the failed check fails the test");
    let [entry] = logged(&env).try_into().unwrap();
    assert_eq!(
        entry.checks,
        [
            check("the start tells the engine warn", false),
            check("the test passed", false),
        ]
    );
}

#[test]
fn a_test_that_fails_outside_its_checks_fails_its_case() {
    let env = TestEnv::new();
    let folder = case_logs(&env);

    let test = std::thread::spawn(move || {
        let mut run = ClientCaseRun::start(
            Some(&folder),
            "engine-logs-reach-the-logger",
            RustMode::Linked,
        )
        .unwrap();
        run.check("connect succeeds", || ());
        panic!("the engine never answered");
    })
    .join();

    assert!(test.is_err());
    let [entry] = logged(&env).try_into().unwrap();
    assert_eq!(
        entry.checks,
        [
            check("connect succeeds", true),
            check("the test passed", false),
        ]
    );
}

#[test]
fn without_a_folder_a_run_writes_nothing() {
    let env = TestEnv::new();

    let mut run =
        ClientCaseRun::start(None, "log-level-follows-the-logger", RustMode::Linked).unwrap();
    run.check("hello carries debug", || ());
    drop(run);

    assert!(!case_logs(&env).exists());
}

#[test]
fn a_case_the_list_does_not_declare_or_not_on_rust_is_refused() {
    let env = TestEnv::new();
    let folder = case_logs(&env);

    let undeclared = ClientCaseRun::start(Some(&folder), "no-such-case", RustMode::Linked);
    let not_on_rust = ClientCaseRun::start(
        Some(&folder),
        "nothing-at-import-or-connect",
        RustMode::Linked,
    );

    assert!(
        matches!(&undeclared, Err(ClientCaseError::Undeclared { case }) if case == "no-such-case"),
        "{undeclared:?}"
    );
    assert!(
        matches!(&not_on_rust, Err(ClientCaseError::NotOnRust { case }) if case == "nothing-at-import-or-connect"),
        "{not_on_rust:?}"
    );
    assert!(!folder.exists());
}
