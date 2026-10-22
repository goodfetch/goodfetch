//! The crash report a panic leaves, through `record_panics` (ADR 0043).
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and read the report once written (ADR 0059)"
)]

use std::time::{Duration, SystemTime};

use goodfetch_test_support::TestEnv;

use super::record_panics;
use crate::clock::Clock;

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
