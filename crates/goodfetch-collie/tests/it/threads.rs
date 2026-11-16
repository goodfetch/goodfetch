//! The engine's Tokio threads through Collie's executable (spec #2165,
//! issue 2200): `governor.worker_threads` sets how many workers each of the
//! executable's runtimes has, the Attached engine's, the Daemon's in its
//! launched and its foreground form, and the one a command's Connection
//! runs on; `governor.blocking_threads` caps their blocking threads. The
//! runtimes name the two kinds, `gf-worker` and `gf-blocking`, so
//! `/proc/<pid>/task` tells them apart. Linux only, where `/proc` lists a
//! process's threads with their names.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads `/proc` and writes its script on its own thread, never an async worker"
)]

use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::{Attached, Hello, HomeLayout};
use goodfetch_test_support::{
    RuntimeFile, TestEnv, Tool, cli, engine_script, record_command, reported_address, tools,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::COLLIE;
use crate::daemon::StopsTheDaemon;
use crate::daemon_run::Foreground;

/// The name of a runtime's worker thread.
const WORKER: &str = "gf-worker";

/// The name of a thread of a runtime's blocking pool.
const BLOCKING: &str = "gf-blocking";

/// How long a runtime may take to start its workers on a loaded machine; a
/// test that passes never waits it out.
const STARTED: Duration = Duration::from_secs(10);

/// The threads of the process `pid` named `name`, as `/proc/<pid>/task`
/// lists them.
fn threads_named(pid: u32, name: &str) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/task"))
        .map(|tasks| {
            tasks
                .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
                .filter(|comm| comm.trim_end() == name)
                .count()
        })
        .unwrap_or_default()
}

/// The workers of the process `pid` once there are `expected`, or as many
/// as it had when [`STARTED`] ran out.
fn workers_once(pid: u32, expected: usize) -> usize {
    let deadline = Instant::now() + STARTED;
    loop {
        let workers = threads_named(pid, WORKER);
        if workers == expected || Instant::now() > deadline {
            return workers;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The one child of the process `pid`: the Daemon its reaper started.
fn only_child(pid: u32) -> u32 {
    let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).unwrap();
    children.trim().parse().unwrap()
}

/// One worker a core, as Tokio counts the cores the test may use.
fn per_core() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// An Attached engine has 2 workers by default, the count
/// `governor.worker_threads` gives, or one a core under `per_core`.
/// Covers: setting:governor.worker_threads, command:attached-engine
#[rstest]
#[case::by_default("", 2)]
#[case::at_a_count("governor.worker_threads = 3\n", 3)]
#[case::per_core("governor.worker_threads = \"per_core\"\n", per_core())]
fn an_attached_engine_has_the_workers_the_setting_says(
    #[case] settings: &str,
    #[case] expected: usize,
) {
    let env = TestEnv::new();
    env.user_configuration(settings);
    let mut engine = Attached::start(&env, COLLIE);
    engine.open(&Hello::needing(Product::Collie));

    let workers = threads_named(engine.pid(), WORKER);
    engine.finish();

    assert_eq!(workers, expected);
}

/// Many calls that each read the Usage ledger on a blocking thread, sent at
/// once, start no more blocking threads than `governor.blocking_threads`
/// caps, 4 by default. A blocking thread outlives its work by Tokio's
/// keep-alive of 10 seconds, longer than the calls take, so the threads
/// alive once they are answered are every one they started.
/// Covers: setting:governor.blocking_threads, command:usage.list
#[rstest]
#[case::by_default("", 4)]
#[case::at_a_count("governor.blocking_threads = 2\n", 2)]
fn blocking_work_never_takes_more_threads_than_the_cap(#[case] settings: &str, #[case] cap: usize) {
    let env = TestEnv::new();
    env.user_configuration(settings);
    let mut engine = Attached::start(&env, COLLIE);
    engine.open(&Hello::needing(Product::Collie));
    let calls: Vec<(i64, &str, Value)> = (1..=64).map(|id| (id, "usage.list", json!({}))).collect();

    let answered = engine
        .at_once(&calls)
        .iter()
        .filter(|reply| reply.get("result").is_some())
        .count();
    let blocking = threads_named(engine.pid(), BLOCKING);
    engine.finish();

    assert_eq!(
        (answered, blocking <= cap),
        (64, true),
        "{blocking} blocking threads"
    );
}

/// A Daemon that `daemon start` launched, and one `daemon run` runs in the
/// foreground under its reaper, have the workers `governor.worker_threads`
/// gives.
/// Covers: setting:governor.worker_threads, command:daemon.start, command:daemon.run
#[test]
fn a_daemon_launched_or_in_the_foreground_has_the_workers_the_setting_says() {
    let launched = TestEnv::new();
    launched.user_configuration("governor.worker_threads = 3\n");
    let _stops = StopsTheDaemon::of(&launched, COLLIE);
    let started = cli(&launched, COLLIE)
        .args(["daemon", "start", "--json"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);
    let found = RuntimeFile::read(&RuntimeFile::path(
        launched.home(),
        env!("CARGO_PKG_VERSION"),
    ))
    .unwrap();
    let foreground = TestEnv::new();
    foreground.user_configuration("governor.worker_threads = 3\n");
    let mut daemon = Foreground::start(&foreground, COLLIE, &[]);
    daemon.serving();

    let workers = (
        workers_once(found.pid, 3),
        workers_once(only_child(daemon.process.child().id()), 3),
    );
    daemon.terminate();

    assert_eq!(workers, (3, 3));
}

/// The corpus's script whose Daemon answers `config.explain`, which the
/// test's own copy leaves unanswered.
const CALL_FAILED: &str = "daemon/cli-call-failed.json";

/// Starts the Scripted engine as the Daemon of the test's home, taking the
/// CLI's `hello` and its `config.explain`, and answering the call never.
fn daemon_holding_its_call(env: &TestEnv) -> goodfetch_test_support::Process {
    let mut script: Value =
        serde_json::from_slice(&std::fs::read(engine_script(CALL_FAILED)).unwrap()).unwrap();
    let steps = script["steps"].as_array_mut().unwrap();
    steps.truncate(3);
    steps.push(json!({ "stall": {} }));
    let path = env.folder().join("call-held.json");
    std::fs::write(&path, serde_json::to_vec(&script).unwrap()).unwrap();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(&path)
        .args(["--mode", "socket"])
        .stderr(std::process::Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    reported_address(&mut process.child()).unwrap();
    process
}

/// A command's Connection to the Daemon runs on a runtime with the workers
/// `governor.worker_threads` gives.
/// Covers: setting:governor.worker_threads, command:config.explain
#[test]
fn a_commands_connection_runs_on_the_workers_the_setting_says() {
    let env = TestEnv::new();
    env.user_configuration("governor.worker_threads = 3\n");
    let scripted = daemon_holding_its_call(&env);
    let arguments = ["config", "explain", "daemon", "--daemon"];
    let mut command = env.process_command(COLLIE);
    command.args(arguments);
    let calling = env.spawn(&mut command).unwrap();

    let workers = workers_once(calling.child().id(), 3);
    let _ = calling.stop();
    let _ = scripted.stop();
    record_command(&env, &arguments, None).unwrap();

    assert_eq!(workers, 3);
}

/// 0 starts no thread: either setting at 0 is `setting_invalid`, which
/// `config explain` exits with, and an Attached engine started on it keeps
/// the default workers and refuses its Client's calls with that code.
/// Covers: setting:governor.worker_threads, setting:governor.blocking_threads, error:setting_invalid
#[rstest]
#[case::worker_threads("governor.worker_threads")]
#[case::blocking_threads("governor.blocking_threads")]
fn either_setting_at_0_is_setting_invalid(#[case] key: &str) {
    let env = TestEnv::new();
    env.user_configuration(&format!("{key} = 0\n"));

    let explained = cli(&env, COLLIE)
        .args(["config", "explain", key, "--json"])
        .run()
        .unwrap();
    let printed: Value = serde_json::from_str(&explained.stdout).unwrap_or_default();
    let mut engine = Attached::start(&env, COLLIE);
    let opened = engine.open(&Hello::needing(Product::Collie));
    let called = engine.ask(1, "config.explain", &json!({ "keys": [key] }));
    let workers = threads_named(engine.pid(), WORKER);
    engine.finish();

    assert_eq!(
        (
            explained.exit,
            printed["error"]["code"].clone(),
            opened.get("error").map_or_else(
                || called["error"]["data"]["code"].clone(),
                |error| error["data"]["code"].clone()
            ),
            workers,
        ),
        (
            Some(3),
            json!("setting_invalid"),
            json!("setting_invalid"),
            2
        ),
        "{}",
        explained.stderr
    );
}

/// `config explain` shows both settings at their defaults, and the
/// Operator policy can lock them.
/// Covers: setting:governor.worker_threads, setting:governor.blocking_threads, command:config.explain
#[test]
fn config_explain_shows_both_and_the_operator_policy_locks_them() {
    let shown = |env: &TestEnv| {
        let output = cli(env, COLLIE)
            .args([
                "config",
                "explain",
                "governor.worker_threads",
                "governor.blocking_threads",
                "--json",
            ])
            .run()
            .unwrap();
        assert_eq!(output.exit, Some(0), "{}", output.stderr);
        let output: Value = serde_json::from_str(&output.stdout).unwrap();
        output["settings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|setting| {
                json!([
                    setting["key"],
                    setting["value"],
                    setting["layer"],
                    setting["locked_by"]
                ])
            })
            .collect::<Vec<_>>()
    };
    let by_default = TestEnv::new();
    let locked = TestEnv::new();
    let layout = HomeLayout::of(locked.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.governor]\nworker_threads = \"per_core\"\nblocking_threads = 512\n",
    )
    .unwrap();

    assert_eq!(
        (shown(&by_default), shown(&locked)),
        (
            vec![
                json!(["governor.blocking_threads", 4, "default", null]),
                json!(["governor.worker_threads", 2, "default", null]),
            ],
            vec![
                json!([
                    "governor.blocking_threads",
                    512,
                    "operator_policy",
                    "operator_policy"
                ]),
                json!([
                    "governor.worker_threads",
                    "per_core",
                    "operator_policy",
                    "operator_policy"
                ]),
            ],
        )
    );
}
