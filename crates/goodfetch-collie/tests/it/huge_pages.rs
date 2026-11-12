//! Transparent huge pages through Collie's executable (spec #2165, issue
//! 2199): the engine starts with them off for itself unless
//! `governor.huge_pages` turns them on, and the browser it starts keeps
//! the host's state either way. Linux only, where the kernel has the
//! per-process flag `/proc/<pid>/status` shows as `THP_enabled`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads `/proc` on its own thread, never an async worker"
)]

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use rstest::rstest;
use serde_json::{Map, json};

use crate::browser_sandbox::{browser_processes, status_of};
use crate::page::{opened, started_with};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 2199;

/// The anonymous memory of the process `pid` that huge pages back, in kB,
/// as `/proc/<pid>/smaps_rollup` sums it.
fn huge_page_backed_kb(pid: u32) -> Option<u64> {
    let rollup = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    let line = rollup
        .lines()
        .find_map(|line| line.strip_prefix("AnonHugePages:"))?;
    line.trim().strip_suffix("kB")?.trim().parse().ok()
}

/// An engine started with `governor.huge_pages` unset or off has turned
/// transparent huge pages off for itself, and none backs its memory; set
/// on, it has the flag clear. The browser it starts has the host's flag,
/// the test runner's, whatever the engine's.
/// Covers: setting:governor.huge_pages, operation:run.new_page
#[rstest]
#[case::by_default("", "THP_enabled: 0")]
#[case::off("governor.huge_pages = false\n", "THP_enabled: 0")]
#[case::on("governor.huge_pages = true\n", "THP_enabled: 1")]
fn the_engine_has_huge_pages_as_the_setting_says_and_its_browser_has_the_hosts(
    #[case] settings: &str,
    #[case] engines: &str,
) {
    let env = TestEnv::new();
    env.user_configuration(settings);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "huge-pages",
        &Map::new(),
        |_| {},
    );
    opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "huge-pages" })));

    // The test's own process links the executable's allocator, whose
    // constructor turned huge pages off here too: the host's state is the
    // one of the runner that started the test.
    let hosts = status_of(std::os::unix::process::parent_id(), &["THP_enabled"]);
    let engine_status = status_of(engine.pid(), &["THP_enabled"]);
    let backed = huge_page_backed_kb(engine.pid());
    let browsers: Vec<Vec<String>> = browser_processes(&env, |line| !line.contains("--type="))
        .into_iter()
        .map(|pid| status_of(pid, &["THP_enabled"]))
        .collect();
    engine.finish();

    assert_eq!(engine_status, [engines]);
    if engines.ends_with('0') {
        assert_eq!(backed, Some(0));
    }
    assert_eq!(browsers, [hosts]);
}
