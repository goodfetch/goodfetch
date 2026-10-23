//! Which engine a command runs on, through Collie's executable (ADRs 0024,
//! 0088, 0098 and 0099): a value-bound command or an opener needs a
//! long-lived engine, so it reaches the Daemon of the major, which it
//! starts when none runs, unless `--attached` is given; a command with no
//! value outliving it runs on an Attached engine unless a Daemon runs or
//! `--daemon` is given.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Daemon's log files in their own Goodfetch home"
)]

use goodfetch_test_support::{TestEnv, cli};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, json_of, short_idle};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The running Daemon's status, `null` when none runs.
fn daemon(env: &TestEnv) -> Value {
    json_of(COLLIE.as_ref(), env, &["daemon", "status", "--json"])["daemon"].clone()
}

/// Whether a Daemon's log file in the home records the engine dispatching
/// `method`: only a Daemon writes one named `daemon-<major>`.
pub(crate) fn daemon_dispatched(env: &TestEnv, method: &str) -> bool {
    let Ok(logs) = std::fs::read_dir(env.home().join("logs")) else {
        return false;
    };
    logs.flatten()
        .filter(|log| log.file_name().to_string_lossy().starts_with("daemon-"))
        .any(|log| {
            std::fs::read_to_string(log.path())
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|line| {
                    line["target"] == "goodfetch::rpc::dispatch"
                        && line["fields"]["method"] == method
                })
        })
}

/// An opener with no Daemon running starts the Daemon, where its Run goes
/// on after the command ended; the same command under `--attached` runs in
/// the executable's own process and starts none.
///
/// Covers: story:11-199, command:run.start
#[test]
fn an_opener_starts_the_daemon_its_run_outlives_the_command_on_and_attached_starts_none() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let attached = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "here", "--attached"])
        .run()
        .unwrap();
    let after_attached = daemon(&env);
    let lasting = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "there"])
        .run()
        .unwrap();
    let after_lasting = daemon(&env);

    assert_eq!(
        (
            attached.exit,
            after_attached,
            lasting.exit,
            lasting.stdout.as_str(),
            after_lasting["live_runs"].clone(),
        ),
        (Some(0), Value::Null, Some(0), "there\n", json!(1)),
        "{attached}\n{lasting}"
    );
}

/// A command with no value outliving it runs on an Attached engine when no
/// Daemon runs, and on the Daemon once one runs.
///
/// Covers: story:11-199, command:engine.info
#[test]
fn a_command_with_no_lasting_value_runs_attached_unless_a_daemon_runs() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let alone = cli(&env, COLLIE).args(["engine", "info"]).run().unwrap();
    let started_none = daemon(&env).is_null();
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let beside = cli(&env, COLLIE).args(["engine", "info"]).run().unwrap();

    assert_eq!(
        (
            alone.exit,
            started_none,
            beside.exit,
            beside.stdout == alone.stdout,
            daemon_dispatched(&env, "engine.info"),
        ),
        (Some(0), true, Some(0), true, true),
        "{alone}\n{beside}"
    );
}
