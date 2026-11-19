//! Which engine `connect()` reaches (ADRs 0024, 0078, 0083 and 0098), the
//! `connect-finds-the-engine` client conformance case in `linked` and
//! `daemon` modes: at `auto`, the remote Daemon `GOODFETCH_URL` names,
//! else the Daemon running in the Goodfetch home, else the linked engine;
//! `engine()` reports the engine reached, and the choice is logged at
//! info. The Daemon is the Scripted engine, playing a script of the corpus
//! with this SDK's `hello` in place of the TypeScript SDK's.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use goodfetch_collie::{
    Collie, ConnectOptions, EngineChoice, EngineInfo, EngineKind, ErrorCode, RemoteDaemon, blocking,
};
use goodfetch_collie_client::logging::engine_log_level;
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{DaemonEndpoint, Form, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::daemon::DaemonUnreachableDetails;
use goodfetch_protocol::settings::SettingInvalidDetails;
use goodfetch_test_support::{
    CapturedLog, Process, RuntimeFile, SCRIPTED_DAEMON_TOKEN, TestEnv, Tool, capturing,
    engine_script, real_time_runtime, reported_address, tools,
};
use rstest::rstest;
use serde_json::Value;
use tracing::Level;
use tracing::subscriber::with_default;

/// The planted token of the remote Daemon's script (ADR 0067), which
/// nothing the SDK shows may hold.
const CANARY: &str = "endpoint-token-canary-5f0e9b27c4d1";

/// The script the Scripted engine plays as the Daemon of the home.
const REACHED_DAEMON: &str = "connect/reached-daemon.json";

/// The script it plays as a remote Daemon, its token the canary.
const REACHED_REMOTE: &str = "connect/reached-remote-with-a-canary-token.json";

/// The environment of `env` with `variables` added.
fn environment(env: &TestEnv, variables: &[(&str, &str)]) -> BTreeMap<OsString, OsString> {
    let mut environment = env.env_os();
    for (name, value) in variables {
        environment.insert(name.into(), value.into());
    }
    environment
}

/// This SDK's `hello` to a Daemon whose token is `token`, at the level the
/// calling thread's subscriber records for the engine.
pub(super) fn hello(token: &str) -> HelloParams {
    HelloParams {
        needs: Some(Product::Collie),
        log_level: Some(engine_log_level()),
        token: Some(token.to_owned()),
        form: Some(Form::Rust),
        ..HelloParams::new(Surface::Rust, version())
    }
}

/// The Product version of the SDK and its engines.
fn version() -> ProductVersion {
    ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap()
}

/// Starts the Scripted engine playing `script` with `hello` in mode `mode`
/// (`socket`, as the Daemon of `env`'s home, or `tcp`), and returns it with
/// the line it printed.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script before any async worker runs"
)]
pub(super) fn scripted(
    env: &TestEnv,
    script: &str,
    hello: &HelloParams,
    mode: &str,
) -> (Process, Value) {
    let mut played: Value =
        serde_json::from_slice(&std::fs::read(engine_script(script)).unwrap()).unwrap();
    played["steps"][0]["expect"]["params"] = serde_json::to_value(hello).unwrap();
    let path = env.folder().join("script.json");
    std::fs::write(&path, serde_json::to_vec(&played).unwrap()).unwrap();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(&path)
        .args(["--mode", mode])
        .stderr(std::process::Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    let reported = reported_address(&mut process.child()).unwrap();
    (process, reported.line)
}

/// Where the Scripted engine, as the Daemon of the home, listens, as its
/// runtime file names it.
fn listening(line: &Value) -> String {
    let runtime_file = line["runtime_file"].as_str().unwrap();
    let found = RuntimeFile::read(runtime_file.as_ref()).unwrap();
    let endpoint: DaemonEndpoint =
        serde_json::from_value(serde_json::to_value(found.endpoint).unwrap()).unwrap();
    endpoint.to_string()
}

/// Asserts that the Scripted engine played its script whole.
pub(super) fn played_whole(process: &Process) {
    assert!(
        process.child().wait().unwrap().success(),
        "the Scripted engine played its script whole"
    );
}

/// A subscriber recording every Goodfetch event at every level, the SDK's
/// and its engine's, and what it writes.
fn goodfetch_log() -> (impl tracing::Subscriber + Send + Sync, CapturedLog) {
    capturing(&[("goodfetch", Level::TRACE)])
}

/// The text `log` holds.
fn text(log: &CapturedLog) -> String {
    String::from_utf8(log.written()).unwrap()
}

/// What `engine()` reports of the engine reached as `kind` in `home`,
/// listening at `url`.
fn reached(kind: EngineKind, home: Option<PathBuf>, url: Option<String>) -> EngineInfo {
    EngineInfo {
        kind,
        product: Product::Collie,
        version: version(),
        protocol_version: PROTOCOL_VERSION,
        project_file: None,
        home,
        engine_path: None,
        url,
        fallback: None,
    }
}

/// The `connect-finds-the-engine` case in `linked` mode: at `auto`, with no
/// Daemon running in the home, the connection links its own engine, and
/// logs that choice at info.
/// Covers: story:11-2, story:11-4, story:11-8
#[test]
fn auto_links_its_own_engine_when_no_daemon_runs_in_the_home() {
    let env = TestEnv::new();
    let (subscriber, log) = goodfetch_log();

    let engine = with_default(subscriber, || {
        real_time_runtime().block_on(async {
            let collie = Collie::connect(ConnectOptions::default().environment(env.env_os()))
                .await
                .unwrap();
            let engine = collie.engine().clone();
            collie.close().await.unwrap();
            engine
        })
    });

    assert_eq!(
        engine,
        reached(EngineKind::Linked, Some(env.home().to_owned()), None)
    );
    let written = text(&log);
    let chosen: Vec<&str> = written
        .lines()
        .filter(|line| line.contains("INFO") && line.contains("reached its engine"))
        .collect();
    assert_eq!(chosen.len(), 1, "{written}");
    assert!(chosen[0].contains("kind=linked"), "{}", chosen[0]);
}

/// The blocking connection reports the engine it reached too.
/// Covers: story:11-4
#[test]
fn a_blocking_connection_reports_the_engine_it_reached() {
    let env = TestEnv::new();

    let collie =
        blocking::Collie::connect(ConnectOptions::default().environment(env.env_os())).unwrap();
    let engine = collie.engine().clone();
    collie.close().unwrap();

    assert_eq!(
        engine,
        reached(EngineKind::Linked, Some(env.home().to_owned()), None)
    );
}

/// The `connect-finds-the-engine` case in `daemon` mode: at `auto` and at
/// `daemon`, the connection reaches the Daemon running in the home with
/// the token of its runtime file, and reports it.
/// Covers: story:11-2, story:11-4, story:11-5
#[rstest]
#[case::auto(EngineChoice::Auto)]
#[case::daemon(EngineChoice::Daemon)]
fn the_daemon_running_in_the_home_is_reached(#[case] choice: EngineChoice) {
    let env = TestEnv::new();
    let (process, line) = scripted(
        &env,
        REACHED_DAEMON,
        &hello(SCRIPTED_DAEMON_TOKEN),
        "socket",
    );
    // The runtime file goes when the play ends.
    let url = listening(&line);
    let options = ConnectOptions::default()
        .environment(env.env_os())
        .engine(choice);

    let engine = real_time_runtime().block_on(async {
        let collie = Collie::connect(options).await.unwrap();
        let engine = collie.engine().clone();
        collie.close().await.unwrap();
        engine
    });

    played_whole(&process);
    assert_eq!(
        engine,
        reached(EngineKind::Daemon, Some(env.home().to_owned()), Some(url))
    );
}

/// At `auto`, the remote Daemon `GOODFETCH_URL` names is reached with
/// `GOODFETCH_TOKEN`'s token, which no log, `Debug` or `engine()` holds
/// (ADR 0067).
/// Covers: story:11-2, story:11-4
#[test]
fn auto_reaches_the_remote_daemon_goodfetch_url_names_and_shows_no_token() {
    let env = TestEnv::new();
    let (subscriber, log) = goodfetch_log();

    let (process, url, engine, shown) = with_default(subscriber, || {
        let (process, line) = scripted(&env, REACHED_REMOTE, &hello(CANARY), "tcp");
        let url = format!("ws://{}", line["address"].as_str().unwrap());
        let options = ConnectOptions::default().environment(environment(
            &env,
            &[("GOODFETCH_URL", &url), ("GOODFETCH_TOKEN", CANARY)],
        ));
        let options_shown = format!("{options:?}");
        let (engine, collie_shown) = real_time_runtime().block_on(async {
            let collie = Collie::connect(options).await.unwrap();
            let shown = format!("{collie:?} {:?}", collie.engine());
            let engine = collie.engine().clone();
            collie.close().await.unwrap();
            (engine, shown)
        });
        (
            process,
            url,
            engine,
            format!("{options_shown} {collie_shown}"),
        )
    });

    played_whole(&process);
    assert_eq!(engine, reached(EngineKind::Remote, None, Some(url)));
    assert!(!shown.contains(CANARY), "{shown}");
    assert!(!text(&log).contains(CANARY), "{}", text(&log));
}

/// The engine `linked` names is reached whatever `GOODFETCH_URL` says.
/// Covers: story:11-2
#[test]
fn linked_wins_over_goodfetch_url() {
    let env = TestEnv::new();
    let refused_by_auto = environment(
        &env,
        &[
            ("GOODFETCH_URL", "ws://192.0.2.1:7878"),
            ("GOODFETCH_TOKEN", "t"),
        ],
    );

    let engine = real_time_runtime().block_on(async {
        let collie = Collie::connect(
            ConnectOptions::default()
                .environment(refused_by_auto)
                .engine(EngineChoice::Linked),
        )
        .await
        .unwrap();
        let engine = collie.engine().clone();
        collie.close().await.unwrap();
        engine
    });

    assert_eq!(engine.kind, EngineKind::Linked);
}

/// A remote Daemon given in code, `{url, token}`, is reached with its own
/// token whatever `GOODFETCH_URL` and `GOODFETCH_TOKEN` say.
/// Covers: story:11-2
#[test]
fn a_remote_daemon_given_in_code_wins_over_goodfetch_url() {
    let env = TestEnv::new();
    let (process, line) = scripted(&env, REACHED_REMOTE, &hello(CANARY), "tcp");
    let url = format!("ws://{}", line["address"].as_str().unwrap());
    let options = ConnectOptions::default()
        .environment(environment(
            &env,
            &[
                ("GOODFETCH_URL", "ws://127.0.0.1:1"),
                ("GOODFETCH_TOKEN", "not-the-token"),
            ],
        ))
        .engine(EngineChoice::Remote(RemoteDaemon::new(url.clone(), CANARY)));

    let engine = real_time_runtime().block_on(async {
        let collie = Collie::connect(options).await.unwrap();
        let engine = collie.engine().clone();
        collie.close().await.unwrap();
        engine
    });

    played_whole(&process);
    assert_eq!(engine, reached(EngineKind::Remote, None, Some(url)));
}

/// A linked program never starts the Daemon (ADR 0098): `daemon` with none
/// running fails with `daemon_unreachable`, whose fix starts it.
/// Covers: story:11-5
#[test]
fn daemon_fails_when_none_runs_and_its_fix_starts_one() {
    let env = TestEnv::new();

    let refused = real_time_runtime().block_on(async {
        Collie::connect(
            ConnectOptions::default()
                .environment(env.env_os())
                .engine(EngineChoice::Daemon),
        )
        .await
    });

    let error = refused.unwrap_err();
    assert_eq!(error.code(), ErrorCode::DAEMON_UNREACHABLE, "{error:?}");
    let details: DaemonUnreachableDetails = error.details().unwrap();
    assert!(
        details.fix.contains("goodfetch-collie daemon start"),
        "{}",
        details.fix
    );
}

/// Nothing listening where a remote Daemon's URL says fails the connect
/// with `daemon_unreachable`.
/// Covers: story:11-2
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "the test takes a free loopback port outside the engine, which nothing then listens on (ADR 0060)"
)]
fn a_remote_daemon_nothing_answers_for_is_unreachable() {
    let env = TestEnv::new();
    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();

    let refused = real_time_runtime().block_on(async {
        Collie::connect(ConnectOptions::default().environment(env.env_os()).engine(
            EngineChoice::Remote(RemoteDaemon::new(format!("ws://{free}"), CANARY)),
        ))
        .await
    });

    let error = refused.unwrap_err();
    assert_eq!(error.code(), ErrorCode::DAEMON_UNREACHABLE, "{error:?}");
    assert!(error.fix().is_some(), "{error:?}");
    assert!(!format!("{error:?}").contains(CANARY), "{error:?}");
}

/// A remote Daemon's URL is `ws://` on loopback with a port and nothing
/// else; any other fails with `setting_invalid` naming where it came from,
/// before anything is reached, and never shows a credential it holds.
/// Covers: story:11-2
#[rstest]
#[case::credentials("ws://user:canary-secret@127.0.0.1:7878")]
#[case::query("ws://127.0.0.1:7878/?token=canary-secret")]
#[case::fragment("ws://127.0.0.1:7878/#canary-secret")]
#[case::no_url("canary-secret")]
#[case::wss("wss://127.0.0.1:7878")]
#[case::beyond_loopback("ws://192.0.2.1:7878")]
#[case::no_port("ws://127.0.0.1")]
#[case::path("ws://127.0.0.1:7878/canary-secret")]
#[case::other_scheme_with_a_path("https://example.test/canary-secret")]
fn a_url_that_names_no_loopback_daemon_is_refused(#[case] url: &str) {
    let env = TestEnv::new();
    let options = |choice| {
        ConnectOptions::default()
            .environment(environment(
                &env,
                &[("GOODFETCH_URL", url), ("GOODFETCH_TOKEN", CANARY)],
            ))
            .engine(choice)
    };

    for (choice, source) in [
        (EngineChoice::Auto, "GOODFETCH_URL"),
        (
            EngineChoice::Remote(RemoteDaemon::new(url, CANARY)),
            "engine",
        ),
    ] {
        let refused =
            real_time_runtime().block_on(async { Collie::connect(options(choice)).await });

        let error = refused.unwrap_err();
        assert_eq!(error.code(), ErrorCode::SETTING_INVALID, "{error:?}");
        let details: SettingInvalidDetails = error.details().unwrap();
        assert_eq!(details.key, source);
        assert!(error.fix().is_some(), "{error:?}");
        let shown = format!("{error:?} {error}");
        assert!(!shown.contains("canary-secret"), "{shown}");
        assert!(!shown.contains(CANARY), "{shown}");
    }
}

/// `GOODFETCH_URL` without `GOODFETCH_TOKEN` fails with `setting_invalid`
/// naming the token's variable; an empty variable counts as unset.
/// Covers: story:11-2
#[rstest]
#[case::unset(&[("GOODFETCH_URL", "ws://127.0.0.1:7878")])]
#[case::empty(&[("GOODFETCH_URL", "ws://127.0.0.1:7878"), ("GOODFETCH_TOKEN", "")])]
fn goodfetch_url_without_a_token_is_refused(#[case] variables: &[(&str, &str)]) {
    let env = TestEnv::new();

    let refused = real_time_runtime().block_on(async {
        Collie::connect(ConnectOptions::default().environment(environment(&env, variables))).await
    });

    let error = refused.unwrap_err();
    assert_eq!(error.code(), ErrorCode::SETTING_INVALID, "{error:?}");
    let details: SettingInvalidDetails = error.details().unwrap();
    assert_eq!(details.key, "GOODFETCH_TOKEN");
}

/// An empty `GOODFETCH_URL` counts as unset: `auto` links its own engine.
/// Covers: story:11-2
#[test]
fn an_empty_goodfetch_url_counts_as_unset() {
    let env = TestEnv::new();

    let engine = real_time_runtime().block_on(async {
        let collie = Collie::connect(
            ConnectOptions::default().environment(environment(&env, &[("GOODFETCH_URL", "")])),
        )
        .await
        .unwrap();
        let engine = collie.engine().clone();
        collie.close().await.unwrap();
        engine
    });

    assert_eq!(engine.kind, EngineKind::Linked);
}
