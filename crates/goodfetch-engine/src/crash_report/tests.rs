//! The crash report a panic leaves, through `record_panics` (ADR 0043).
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and read the report once written (ADR 0059)"
)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_test_support::TestEnv;

use secrecy::SecretString;

use super::record_panics;
use crate::clock::Clock;
use crate::mask::{Masks, Reach, Redaction};

/// Covers: story:3-105
#[test]
fn a_panic_leaves_a_crash_report_in_its_folder_on_the_machine() {
    let env = TestEnv::new();
    let folder = env.home().join("crashes");
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);

    let caught = goodfetch_test_support::runtime().block_on(async {
        record_panics(
            folder.clone(),
            "goodfetch-collie 0.1.0".to_owned(),
            Clock::paused(start),
            Masks::new(),
        );
        std::panic::catch_unwind(|| panic!("the invariant broke"))
    });
    let _ = std::panic::take_hook();

    let reports: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let text = std::fs::read_to_string(&reports[0]).unwrap();
    let lines: Vec<&str> = text.lines().take(5).collect();
    assert_eq!(
        (
            caught.is_err(),
            reports.len(),
            reports[0].file_name().unwrap().to_str().unwrap().to_owned(),
            lines[0],
            lines[1],
            lines[3].starts_with("location: crates/goodfetch-engine/src/crash_report/tests.rs:"),
            lines[4],
        ),
        (
            true,
            1,
            format!("crash-20260921T141320Z-{}.txt", std::process::id()),
            "Crash report of goodfetch-collie 0.1.0",
            "time: 2026-09-21T14:13:20Z",
            true,
            "message: the invariant broke",
        )
    );
}

/// A panic's text stays in its crash report: the one log line the panic
/// writes, all a log file or a configured collector receives of it, names
/// the report's path and holds none of the panic's text (spec #564).
/// Covers: story:4-84
#[test]
fn a_panics_text_is_in_its_crash_report_and_in_no_log_line() {
    let env = TestEnv::new();
    let folder = env.home().join("crashes");
    let log = env.home().join("panic.log");
    std::fs::create_dir_all(env.home()).unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(Arc::new(std::fs::File::create(&log).unwrap()))
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        goodfetch_test_support::runtime().block_on(async {
            record_panics(
                folder.clone(),
                "goodfetch-collie 0.1.0".to_owned(),
                Clock::paused(SystemTime::UNIX_EPOCH),
                Masks::new(),
            );
            std::panic::catch_unwind(|| panic!("the page at https://shop.example.test/cart broke"))
        })
    })
    .unwrap_err();
    let _ = std::panic::take_hook();

    let report = std::fs::read_dir(&folder)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = std::fs::read_to_string(&report).unwrap();
    let logged = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = logged.lines().collect();
    assert_eq!(
        (
            text.contains("message: the page at https://shop.example.test/cart broke"),
            lines.len(),
            lines.iter().any(|line| line.contains("shop.example.test")),
            lines
                .first()
                .is_some_and(|line| line.contains(&report.display().to_string())),
        ),
        (true, 1, false, true)
    );
}

/// A panic whose message holds a value a Run resolved leaves a report that
/// holds its `%name%` in its place (spec #564), whatever the Run's
/// `security.redaction`: a crash report is what the engine keeps.
/// Covers: story:4-56
#[test]
fn a_crash_report_holds_no_value_a_run_resolved() {
    let env = TestEnv::new();
    let folder = env.home().join("crashes");
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::Off);
    mask.join(
        "proxy.password",
        &SecretString::from("pa ss\"w/rd>?9~"),
        Reach::Value,
    );

    goodfetch_test_support::runtime()
        .block_on(async {
            record_panics(
                folder.clone(),
                "goodfetch-collie 0.1.0".to_owned(),
                Clock::paused(SystemTime::UNIX_EPOCH),
                masks,
            );
            std::panic::catch_unwind(|| panic!("the proxy refused pa ss\"w/rd>?9~"))
        })
        .unwrap_err();
    let _ = std::panic::take_hook();

    let report = std::fs::read_dir(&folder)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = std::fs::read_to_string(report).unwrap();
    assert_eq!(
        text.lines().nth(4),
        Some("message: the proxy refused %proxy.password%")
    );
}
