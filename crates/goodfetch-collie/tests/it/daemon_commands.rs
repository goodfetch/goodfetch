//! The `daemon` commands' `--json` output through Collie's executable (spec
//! #608's hand-written commands, ADRs 0024, 0049 and 0069): `daemon start`,
//! `restart`, `status` and `stop` each print the major's `DaemonStatus`,
//! `daemon run --json` prints it once it listens, `daemon
//! http-listener-token` prints the HTTP listener's token on stdout and
//! nowhere else, and `daemon rotate-token` replaces the tokens, so a
//! Daemon started after it refuses the old one.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Daemon's token files and logs themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fmt::Write as _;

use goodfetch_protocol::home::DaemonFile;
use goodfetch_test_client::DaemonClient;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{CliOutput, TestEnv, cli};
use serde_json::{Value, json};

use crate::daemon::StopsTheDaemon;
use crate::daemon_hello;
use crate::daemon_run::{ANY_PORT, Foreground, opened, token};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Runs `arguments` and checks that its stdout is one document of
/// `definition`.
fn checked(env: &TestEnv, arguments: &[&str], definition: &str) -> (CliOutput, Value) {
    let checked = cli(env, COLLIE).args(arguments).json(definition).unwrap();
    let document = checked.documents[0].clone();
    (checked.output, document)
}

/// Snapshots `outputs` as `name`, each under its command line, with the
/// shared filters and the Daemon's PID and socket, whose folder depends on
/// the OS's longest socket path, replaced.
fn assert_transcript(name: &str, outputs: &[(&[&str], &CliOutput)]) {
    let mut settings = outputs[0].1.settings();
    settings.add_filter(r#""pid": [0-9]+"#, r#""pid": "[pid]""#);
    settings.add_filter(r#""address": "[^"]+""#, r#""address": "[address]""#);
    let text = outputs
        .iter()
        .fold(String::new(), |mut text, (arguments, output)| {
            let _ = writeln!(text, "$ {}\n{output}", arguments.join(" "));
            text
        });
    settings.bind(|| goodfetch_test_support::__insta::assert_snapshot!(name, text));
}

/// `daemon start`, `status`, `restart` and `stop` each print the major's
/// Daemon as `DaemonStatus`: its PID, major, Product, version, address and
/// Console, every field but the major `null` once it stopped.
///
/// Covers: story:11-214, command:daemon.start, command:daemon.status, command:daemon.restart, command:daemon.stop
#[test]
fn each_daemon_command_prints_the_majors_daemon_status() {
    let env = TestEnv::new();
    env.user_configuration("daemon.idle_timeout = 60000\nconsole.address = \"127.0.0.1:0\"\n");
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let start: &[&str] = &["daemon", "start", "--json"];
    let status: &[&str] = &["daemon", "status", "--json"];
    let restart: &[&str] = &["daemon", "restart", "--json"];
    let stop: &[&str] = &["daemon", "stop", "--json"];
    let (started, first) = checked(&env, start, "DaemonStatus");
    let (statused, shown) = checked(&env, status, "DaemonStatus");
    let (restarted, second) = checked(&env, restart, "DaemonStatus");
    let (stopped, after) = checked(&env, stop, "DaemonStatus");

    assert_eq!(
        (
            shown["pid"] == first["pid"],
            second["pid"] == first["pid"],
            after["pid"].clone()
        ),
        (true, false, Value::Null)
    );
    assert_transcript(
        "daemon_start_status_restart_stop",
        &[
            (start, &started),
            (status, &statused),
            (restart, &restarted),
            (stop, &stopped),
        ],
    );
}

/// `daemon restart` fails on settings it could not start on before it stops
/// anything: a typo in the file leaves the running Daemon serving, which never
/// needed that file again.
///
/// Covers: story:11-214, command:daemon.restart, error:setting_unknown
#[test]
fn a_restart_on_unreadable_settings_leaves_the_running_daemon_serving() {
    let env = TestEnv::new();
    let readable = "daemon.idle_timeout = 60000\nconsole.address = \"127.0.0.1:0\"\n";
    env.user_configuration(readable);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let (_, first) = checked(&env, &["daemon", "start", "--json"], "DaemonStatus");

    env.user_configuration("crawl.max_deepth = 3\n");
    let refused = cli(&env, COLLIE)
        .args(["daemon", "restart", "--json"])
        .run()
        .unwrap();
    let printed: Value = serde_json::from_str(&refused.stdout).unwrap_or(Value::Null);
    env.user_configuration(readable);
    let (_, after) = checked(&env, &["daemon", "status", "--json"], "DaemonStatus");

    assert_eq!(
        (
            refused.exit,
            printed["error"]["code"].clone(),
            after["pid"] == first["pid"],
        ),
        (Some(3), json!("setting_unknown"), true),
        "{refused}"
    );
}

/// `daemon run --json` prints the foreground Daemon's `DaemonStatus` as
/// its first line once it listens, its logs following as JSON lines.
///
/// Covers: story:11-214, command:daemon.run
#[test]
fn daemon_run_json_prints_its_daemon_status_once_it_listens() {
    let env = TestEnv::new();
    env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n"));
    let mut command = env.process_command(COLLIE);
    command.args(["daemon", "run", "--json"]);

    let mut daemon = Foreground::spawned(&env, &mut command);
    let status = daemon.first_line();
    let endpoint = daemon.serving();
    let exited = daemon.terminate();

    crate::daemon::validate(&status, "DaemonStatus");
    assert_eq!(
        (
            status["address"].clone(),
            status["pid"].is_u64(),
            status["console_url"].clone(),
            exited,
        ),
        (json!(endpoint.to_string()), true, Value::Null, true)
    );
    // Its first line beyond the schema, the PID and the port it took
    // replaced (608-5).
    let mut shown = status;
    shown["pid"] = json!("[pid]");
    shown["address"] = json!("[address]");
    goodfetch_test_support::__insta::assert_snapshot!(
        "daemon_run_json_first_line",
        serde_json::to_string_pretty(&shown).unwrap()
    );
}

/// `daemon http-listener-token` prints the HTTP listener's token on stdout
/// and nowhere else: not on stderr, not in a log, and as the same token
/// each time until a rotation.
///
/// Covers: story:11-214, command:daemon.http-listener-token
#[test]
fn daemon_http_listener_token_prints_the_credential_on_stdout_and_nowhere_else() {
    let env = TestEnv::new();
    let arguments: &[&str] = &["daemon", "http-listener-token", "--json"];

    let (output, printed) = checked(&env, arguments, "HttpListenerToken");
    let human = cli(&env, COLLIE)
        .args(["daemon", "http-listener-token"])
        .run()
        .unwrap();

    let token = printed["token"].as_str().unwrap().to_owned();
    let file = std::fs::read_to_string(
        HomeLayout::of(env.home()).daemon_file(DaemonFile::HttpListenerToken),
    )
    .unwrap();
    let logged = logs_holding(&env, &token);
    assert_eq!(
        (
            file == token,
            human.stdout == format!("{token}\n"),
            output.stderr.contains(&token) || human.stderr.contains(&token),
            logged,
        ),
        (true, true, false, Vec::<String>::new())
    );
    let mut settings = output.settings();
    settings.add_filter(&regex_escape(&token), "[token]");
    settings.bind(|| goodfetch_test_support::__insta::assert_snapshot!(output.to_string()));
}

/// `daemon rotate-token` replaces the HTTP listener's token and the
/// persistent token a foreground Daemon takes, and stops the foreground
/// Daemon that runs with the old one, so the old token stops working at
/// once (608-O3): it exits by itself, and one started after it refuses a
/// Client with the old token and opens with the new one.
///
/// Covers: story:11-214, command:daemon.rotate-token, command:daemon.run
#[test]
fn daemon_rotate_token_makes_the_old_token_refuse() {
    let env = TestEnv::new();
    env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n"));
    let mut first = Foreground::start(&env, COLLIE, &[]);
    let endpoint = first.serving();
    drop(opened(&env, &endpoint));
    let old = token(&env);
    let (_, old_listener) = checked(
        &env,
        &["daemon", "http-listener-token", "--json"],
        "HttpListenerToken",
    );
    let arguments: &[&str] = &["daemon", "rotate-token", "--json"];

    // The foreground Daemon took a free port, which `rotate-token` reaches
    // it at as an operator's `GOODFETCH_DAEMON_ADDRESS` names it.
    let goodfetch_test_support::DaemonEndpoint::Tcp(address) = &endpoint else {
        unreachable!("a foreground Daemon listens on TCP");
    };
    let rotated = cli(&env, COLLIE)
        .args(arguments)
        .env("GOODFETCH_DAEMON_ADDRESS", address.to_string())
        .json("TokenRotated")
        .unwrap()
        .output;
    // The rotation stopped it: it exits with no signal.
    assert!(first.exited(), "{:#?}", first.logged);
    let mut second = Foreground::start(&env, COLLIE, &[]);
    let endpoint = second.serving();
    let refused = DaemonClient::refuses(&endpoint, &daemon_hello(&old));
    drop(opened(&env, &endpoint));
    let (_, new_listener) = checked(
        &env,
        &["daemon", "http-listener-token", "--json"],
        "HttpListenerToken",
    );
    let stopped = second.terminate();

    assert_eq!(
        (
            refused,
            token(&env) == old,
            new_listener["token"] == old_listener["token"],
            stopped,
        ),
        (true, false, false, true)
    );
    goodfetch_test_support::assert_cli_snapshot!(rotated);
}

/// The files of the test home's log folder whose text holds `secret`.
fn logs_holding(env: &TestEnv, secret: &str) -> Vec<String> {
    let folder = env.home().join("logs");
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| std::fs::read_to_string(path).is_ok_and(|text| text.contains(secret)))
        .map(|path| path.display().to_string())
        .collect()
}

fn regex_escape(text: &str) -> String {
    text.chars()
        .flat_map(|character| {
            let escaped = !character.is_ascii_alphanumeric();
            escaped.then_some('\\').into_iter().chain([character])
        })
        .collect()
}
