//! J-001, "Start the Daemon and see it stop on its own once idle", on the
//! `foxhound-cli` surface (ADRs 0063, 0075 and 0094): Collie's Journey,
//! given Foxhound's shipped executable, which keeps Collie's behaviour. A
//! person shortens the Daemon's idle time in their user configuration, sees
//! through `config explain` that `daemon.idle_timeout` now holds it, starts
//! the Daemon and leaves it with no Client, until it stops on its own.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's wait is measured in real time, from the Daemon's start to its stop, which the test waits on through the Daemon's own lock file"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_protocol::home::DaemonFile;
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{JourneyRun, RuntimeFile, TestEnv, cli};
use serde_json::Value;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// This commit's Product version, whose major folder holds the runtime
/// file.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The idle time the person sets, in milliseconds: short enough for a
/// test, long enough to tell a stop on its own from one at once.
const IDLE_TIMEOUT: u64 = 1_500;

/// The JSON `arguments` print through `executable`, which exits with 0.
pub(crate) fn json_of(executable: &Path, env: &TestEnv, arguments: &[&str]) -> Value {
    let output = cli(env, executable).args(arguments).run().unwrap();
    assert_eq!(output.exit, Some(0), "{}{}", output.stdout, output.stderr);
    serde_json::from_str(&output.stdout).unwrap()
}

/// Waits, without polling, until the Daemon of the test's home has
/// stopped: it holds its serving lock until then.
pub(crate) fn wait_until_stopped(env: &TestEnv) {
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(HomeLayout::of(env.home()).daemon_file(DaemonFile::ServingLock))
        .unwrap();
    lock.lock().unwrap();
}

/// Covers: story:3-18, setting:daemon.idle_timeout
#[test]
fn foxhound_cli() {
    let mut run = JourneyRun::start(
        "J-001",
        "foxhound-cli",
        Path::new(FOXHOUND),
        ResultDocument::journey,
    )
    .expect("J-001's foxhound-cli variant starts");
    let mut started_at = None;
    let mut waited = None;

    run.step(
        "Shorten the Daemon's idle time through its `daemon.idle_timeout` setting",
        |run| {
            run.env()
                .user_configuration(&format!("daemon.idle_timeout = {IDLE_TIMEOUT}\n"));
            let explained = json_of(
                run.engine(),
                run.env(),
                &["config", "explain", "daemon.idle_timeout", "--json"],
            );
            let set = explained["settings"]
                .as_array()
                .unwrap()
                .iter()
                .find(|setting| setting["key"] == "daemon.idle_timeout")
                .map(|setting| (setting["value"].clone(), setting["layer"].clone()));
            assert_eq!(
                set,
                Some((Value::from(IDLE_TIMEOUT), Value::from("user_configuration"))),
                "{explained}"
            );
        },
    );
    run.step("Start the Daemon", |run| {
        started_at = Some(Instant::now());
        let started = json_of(run.engine(), run.env(), &["daemon", "start", "--json"]);
        assert!(started["pid"].is_u64(), "{started}");
    });
    run.step("Wait past the idle time with no Client connected", |run| {
        wait_until_stopped(run.env());
        waited = started_at.map(|at| at.elapsed());
    });

    run.check(
        "The Daemon stops on its own once no Client has been connected for its idle time",
        |run| {
            let status = json_of(run.engine(), run.env(), &["daemon", "status", "--json"]);
            assert_eq!(
                (
                    status["pid"].clone(),
                    RuntimeFile::path(run.env().home(), VERSION).exists(),
                    waited.is_some_and(|waited| waited >= Duration::from_millis(IDLE_TIMEOUT)),
                ),
                (Value::Null, false, true),
                "{status} after {waited:?}"
            );
        },
    );
    run.finish()
        .expect("J-001's document is written")
        .assert_proved();
}
