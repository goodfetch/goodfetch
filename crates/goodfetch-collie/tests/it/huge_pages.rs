//! Transparent huge pages through Collie's executable (spec #2165, issue
//! 2199): the engine starts with them off for itself unless
//! `governor.huge_pages` turns them on, and the browser it starts keeps
//! the state the engine was started with either way. Linux only, where
//! the kernel has the per-process flag `/proc/<pid>/status` shows as
//! `THP_enabled`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads `/proc` on its own thread, never an async worker"
)]
#![expect(
    unsafe_code,
    reason = "the engine is started with a chosen huge-page flag, which only `prctl` sets, between fork and exec (STYLE.md, `unsafe`)"
)]

use std::os::unix::process::CommandExt;

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

/// `THP_enabled` as `/proc/<pid>/status` shows a process with huge pages
/// `on` for it.
fn shown(on: bool) -> Vec<String> {
    vec![format!("THP_enabled: {}", u8::from(on))]
}

/// An engine started with `governor.huge_pages` unset or off has turned
/// transparent huge pages off for itself, and none backs its memory; set
/// on, it has them on. Either way the browser it starts has the state the
/// engine was started with, on a host that left them on and on one that
/// turned them off.
/// Covers: setting:governor.huge_pages
#[rstest]
#[case::by_default_where_the_host_has_them("", true, false)]
#[case::off_where_the_host_has_them("governor.huge_pages = false\n", true, false)]
#[case::on_where_the_host_has_them("governor.huge_pages = true\n", true, true)]
#[case::by_default_where_the_host_has_none("", false, false)]
#[case::on_where_the_host_has_none("governor.huge_pages = true\n", false, true)]
fn the_engine_has_huge_pages_as_the_setting_says_and_its_browser_as_the_host(
    #[case] settings: &str,
    #[case] host: bool,
    #[case] engines: bool,
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
        |command| {
            // The test's own process links the executable's allocator,
            // which turned huge pages off here too, so the engine is
            // given the host's state the case names.
            let off = libc::c_ulong::from(!host);
            // SAFETY: the closure runs in the child between fork and
            // exec, and makes one async-signal-safe system call, which
            // takes no pointer.
            unsafe {
                command.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_THP_DISABLE, off, 0, 0, 0) == 0 {
                        Ok(())
                    } else {
                        Err(std::io::Error::last_os_error())
                    }
                });
            }
        },
    );
    opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "huge-pages" })));

    let engine_status = status_of(engine.pid(), &["THP_enabled"]);
    let backed = huge_page_backed_kb(engine.pid());
    let browsers: Vec<Vec<String>> = browser_processes(&env, |line| !line.contains("--type="))
        .into_iter()
        .map(|pid| status_of(pid, &["THP_enabled"]))
        .collect();
    engine.finish();

    assert_eq!(engine_status, shown(engines));
    if !engines {
        assert_eq!(backed, Some(0));
    }
    assert_eq!(browsers, [shown(host)]);
}
