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
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::DaemonClient;
use goodfetch_test_support::{
    DISPATCH_TARGET, RuntimeFile, SnapshotValues, TestEnv, cli, redact, snapshot_settings,
};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, json_of, runtime_file, scripted_daemon, short_idle};
use crate::daemon_hello;
#[cfg(unix)]
use crate::daemon_run::{ANY_PORT, Foreground, token};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Snapshots a command's `stderr`, this executable's path as `[COLLIE]`.
fn stderr_snapshot(env: &TestEnv, name: &str, stderr: &str) {
    let values = SnapshotValues::of(env).with_path(COLLIE, "[COLLIE]");
    snapshot_settings(&values).bind(|| insta::assert_snapshot!(name, stderr.trim()));
}

/// What the running Daemon answers `daemon.status`, `null` when none runs.
fn daemon(env: &TestEnv) -> Value {
    let status = json_of(COLLIE.as_ref(), env, &["daemon", "status", "--json"]);
    if status["pid"].is_null() {
        return Value::Null;
    }
    let found = RuntimeFile::read(&runtime_file(env)).unwrap();
    DaemonClient::opened(&found.endpoint, &daemon_hello(&found.token)).call(
        1,
        "daemon.status",
        &json!({}),
    )["result"]
        .clone()
}

/// The engine a hand-written command's flag chooses, for its `attached` and
/// `daemon` cases against one expected answer (ticket #1183, ADR 0069).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum On {
    /// `--attached`: an Attached engine in the command's process.
    Attached,
    /// `--daemon`: the Daemon of the major, which the command starts.
    Daemon,
}

impl On {
    /// Its flag.
    pub(crate) fn flag(self) -> &'static str {
        match self {
            Self::Attached => "--attached",
            Self::Daemon => "--daemon",
        }
    }
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
                .any(|line| line["target"] == DISPATCH_TARGET && line["fields"]["method"] == method)
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

/// Under `--daemon`, a Daemon that refuses the command's protocol version
/// and cannot hand over ends the command with exit `11` and a `fix` naming
/// this executable's `daemon restart` by its path (ADR 0098), read from the
/// Scripted engine playing the Daemon (ADR 0069).
///
/// Covers: story:11-213, command:run.start, error:protocol_version_unsupported
#[test]
fn under_daemon_a_daemon_refusing_the_version_fails_with_exit_11_and_its_daemon_restart() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-hello-refused.json");

    let output = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first", "--daemon"])
        .run()
        .unwrap();
    scripted.child().wait().unwrap();

    assert_eq!(
        (output.exit, output.stdout.as_str()),
        (Some(11), ""),
        "{output}"
    );
    stderr_snapshot(&env, "run_start_refused_version", &output.stderr);
}

/// With neither flag, a Daemon that refuses the protocol version of a
/// command with no value outliving it, and cannot hand over, leaves it to
/// an Attached engine, after a warning that names the refusal and its fix,
/// as `engine: "auto"` does (ADR 0098).
///
/// Covers: story:11-199, command:engine.info
#[test]
fn by_default_a_daemon_refusing_the_version_leaves_a_passing_command_to_an_attached_engine() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-hello-refused.json");

    let output = cli(&env, COLLIE).args(["engine", "info"]).run().unwrap();
    scripted.child().wait().unwrap();

    let warning = output
        .stderr
        .lines()
        .find(|line| line.starts_with("warning:"))
        .map(|line| {
            redact(
                line,
                &SnapshotValues::of(&env).with_path(COLLIE, "[COLLIE]"),
            )
        })
        .unwrap_or_default();
    assert_eq!(
        (output.exit, output.stdout.is_empty(), warning.as_str()),
        (
            Some(0),
            false,
            "warning: the Daemon refused this command, which runs on an Attached engine instead: This Client speaks protocol 0.1, and this engine speaks 1.0: it accepts 1.0 to 1.0. Upgrade the Client to one that speaks protocol 1.x. Run `[COLLIE] daemon restart` to restart the Daemon on its major's pick."
        ),
        "{output}"
    );
}

/// With neither flag, a Daemon that refuses the protocol version of a
/// command whose value outlives it ends the command with its refusal and
/// `fix`, exit `11`, as under `--daemon`: an Attached engine would stop its
/// Run with the command, after printing its Run ID (608-O6).
///
/// Covers: story:11-199, command:run.start, error:protocol_version_unsupported
#[test]
fn by_default_a_daemon_refusing_the_version_ends_a_lasting_command_with_exit_11() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-hello-refused.json");

    let output = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first"])
        .run()
        .unwrap();
    scripted.child().wait().unwrap();

    assert_eq!(
        (
            output.exit,
            output.stdout.as_str(),
            output.stderr.contains("warning:"),
        ),
        (Some(11), "", false),
        "{output}"
    );
    stderr_snapshot(&env, "run_start_refused_version", &output.stderr);
}

/// A runtime file whose Daemon died is replaced: a command whose value
/// outlives it starts a new Daemon, where its Run goes on.
///
/// Covers: story:11-213, command:run.start
#[test]
fn a_dead_pid_in_the_runtime_file_is_replaced_by_the_daemon_a_command_starts() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let scripted = scripted_daemon(&env, "daemon/dead-pid.json");
    scripted.child().wait().unwrap();
    let dead = RuntimeFile::read(&runtime_file(&env)).unwrap();

    let output = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first"])
        .run()
        .unwrap();
    let running = daemon(&env);

    assert_eq!(
        (
            output.exit,
            output.stdout.as_str(),
            running["pid"] == json!(dead.pid),
            running["live_runs"].clone(),
        ),
        (Some(0), "first\n", false, json!(1)),
        "{output}"
    );
}

/// A Connection to the Daemon that drops before the call is answered fails
/// the command with `connection_lost`, an `UnavailableError`: exit `8`.
///
/// Covers: story:11-213, command:run.start, error:connection_lost
#[test]
fn a_connection_dropped_mid_command_is_reported_and_exits_8() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-run-start-dropped.json");

    let output = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first", "--daemon"])
        .run()
        .unwrap();
    scripted.child().wait().unwrap();

    assert_eq!(
        (output.exit, output.stdout.as_str()),
        (Some(8), ""),
        "{output}"
    );
    stderr_snapshot(&env, "run_start_connection_dropped", &output.stderr);
}

/// No command takes the Daemon's token as a flag, so a token never sits in
/// a process list: `--token` is a usage error, before any engine call.
///
/// Covers: story:11-199, command:run.start
#[test]
fn a_token_flag_is_a_usage_error() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first", "--token", "a-token"])
        .run()
        .unwrap();

    assert_eq!(
        (
            output.exit,
            output.stderr.contains("unexpected argument '--token'")
        ),
        (Some(2), true),
        "{output}"
    );
}

/// `config explain --daemon` with `GOODFETCH_URL` set explains the remote
/// Daemon's settings, as every command under `--daemon` reaches it, and
/// starts no Daemon of the major (608-6).
///
/// Covers: story:11-199, command:config.explain
#[cfg(unix)]
#[test]
fn config_explain_under_daemon_reaches_the_remote_daemon_goodfetch_url_names() {
    let env = TestEnv::new();
    // A Daemon of the major started by mistake stops soon, and with the test.
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\ndaemon.idle_timeout = 60000\n"
    ));
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let mut remote = Foreground::start(&env, COLLIE, &[]);
    let address = remote.serving();

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "daemon", "--daemon", "--json"])
        .env("GOODFETCH_URL", format!("ws://{address}"))
        .env("GOODFETCH_TOKEN", token(&env))
        .run()
        .unwrap();
    let started_one = runtime_file(&env).exists();
    let drained = remote.terminate();
    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);

    assert_eq!(
        (
            output.exit,
            printed["settings"].is_array(),
            started_one,
            drained
        ),
        (Some(0), true, false, true),
        "{output}"
    );
}

/// The Exposed Daemon `GOODFETCH_URL` names, with `GOODFETCH_TOKEN`'s token:
/// a command whose value outlives it runs there, and starts no Daemon of
/// the major.
///
/// Covers: story:11-199, command:run.start
#[cfg(unix)]
#[test]
fn goodfetch_url_and_goodfetch_token_reach_a_remote_daemon() {
    let env = TestEnv::new();
    // A Daemon of the major started by mistake stops soon, and with the test.
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\ndaemon.idle_timeout = 60000\n"
    ));
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let mut remote = Foreground::start(&env, COLLIE, &[]);
    let address = remote.serving();

    let output = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first"])
        .env("GOODFETCH_URL", format!("ws://{address}"))
        .env("GOODFETCH_TOKEN", token(&env))
        .run()
        .unwrap();
    let started_one = runtime_file(&env).exists();
    let drained = remote.terminate();

    assert_eq!(
        (output.exit, output.stdout.as_str(), started_one, drained),
        (Some(0), "first\n", false, true),
        "{output}"
    );
}

/// A token the remote Daemon refuses fails the command with
/// `daemon_unreachable`, exit `8`, its `fix` naming `GOODFETCH_TOKEN`, and
/// the token's value is in no output, log or error (ADR 0083).
///
/// Covers: story:11-199, command:engine.info, error:daemon_unreachable
#[cfg(unix)]
#[test]
fn goodfetch_tokens_value_appears_in_no_output_log_or_error() {
    const WRONG: &str = "c0ffee-a-token-the-daemon-never-issued";
    let env = TestEnv::new();
    // A Daemon of the major started by mistake stops soon, and with the test.
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\ndaemon.idle_timeout = 60000\n"
    ));
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let mut remote = Foreground::start(&env, COLLIE, &[]);
    let address = remote.serving();

    let output = cli(&env, COLLIE)
        .args(["engine", "info"])
        .env("GOODFETCH_URL", format!("ws://{address}"))
        .env("GOODFETCH_TOKEN", WRONG)
        .run()
        .unwrap();
    let drained = remote.terminate();
    let logs: Vec<String> = std::fs::read_dir(env.home().join("logs"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|log| std::fs::read_to_string(log.path()).unwrap())
        .chain(remote.logged.iter().cloned())
        .collect();

    assert_eq!(
        (
            output.exit,
            output.stdout.contains(WRONG),
            output.stderr.contains(WRONG),
            logs.iter().any(|log| log.contains(WRONG)),
            output.stderr.contains("GOODFETCH_TOKEN"),
            drained,
        ),
        (Some(8), false, false, false, true, true),
        "{output}"
    );
}
