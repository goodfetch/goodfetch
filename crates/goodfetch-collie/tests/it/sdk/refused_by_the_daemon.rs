//! A Daemon that refuses `connect()`'s `hello` by Product or protocol
//! version (ADR 0098), the `connect-falls-back-when-the-daemon-refuses`
//! client conformance case in `linked` and `daemon` modes: at `auto` the
//! connection links its own engine, reports why in `engine().fallback` and
//! warns once on `goodfetch::client`; at `daemon` it fails with the
//! refusal, whose `fix` names the Product's install command first; a
//! remote Daemon's refusal fails it unchanged; and a Daemon that hands
//! over is waited past until its major's pick serves. The Daemon is the
//! Scripted engine, playing a script of the corpus with this SDK's
//! `hello` in place of the TypeScript SDK's.

use std::time::Duration;

use goodfetch_collie::{
    Collie, ConnectOptions, EngineChoice, EngineInfo, EngineKind, ErrorCode, RemoteDaemon, blocking,
};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_engine::daemon::files::DaemonFiles;
use goodfetch_engine::home::Home;
use goodfetch_engine::installed::InstalledList;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{DeadlineExceededDetails, WaitedFor};
use goodfetch_protocol::executables::ExecutableKind;
use goodfetch_protocol::settings::{RunOrigin, SettingsLayer};
use goodfetch_test_support::{
    CapturedLog, SCRIPTED_DAEMON_TOKEN, TestEnv, capturing, case_logs, real_time_runtime,
};
use rstest::rstest;
use tracing::Level;
use tracing::subscriber::with_default;

use super::engine_choice::{
    environment, hello, listening, played_whole, reached, remote, scripted, subprotocol, text,
    version,
};

/// The Daemon refusing by protocol version, unable to hand over.
const REFUSED_BY_VERSION: &str = "connect/daemon-refused-protocol-version-unsupported.json";

/// The Daemon refusing by Product, unable to hand over.
const REFUSED_BY_PRODUCT: &str = "connect/daemon-refused-not-in-product.json";

/// The idle Daemon that refuses and hands over to its major's pick.
const HANDS_OVER: &str = "connect/daemon-hands-over.json";

/// The Daemon of the home that answers `hello`.
const REACHED_DAEMON: &str = "connect/reached-daemon.json";

/// The `fix` [`REFUSED_BY_VERSION`]'s refusal gives.
const VERSION_FIX: &str = "Upgrade the engine to a version that speaks protocol 0.1, or use a Client of the engine's own version.";

/// The `fix` [`REFUSED_BY_PRODUCT`]'s refusal gives.
const PRODUCT_FIX: &str = "Connect to a Foxhound engine, or run with Foxhound, which contains everything Collie does (`npm install @goodfetch/foxhound`).";

/// What a refusal's `fix` adds for a Rust program linking Collie (ADR
/// 0098): the install of Collie's executable first, then its `daemon
/// restart`, and Foxhound at its version or newer, since a Foxhound of the
/// major would still be the pick.
const LINKED_FIX: &str = "Install Collie's executable at 0.0.0 (`cargo install goodfetch-collie --locked --features executable`), then run its `daemon restart` to restart the Daemon on its major's pick. Collie's executable would still not run the Daemon while a Foxhound of the major is installed: install Foxhound 0.0.0 or newer (`cargo install goodfetch-foxhound --locked --features executable`) and run its `daemon restart`.";

/// A subscriber recording the warnings on `goodfetch::client` alone, with
/// what it writes.
fn client_warnings() -> (impl tracing::Subscriber + Send + Sync, CapturedLog) {
    capturing(&[("goodfetch::client", Level::WARN)])
}

/// The `connect-falls-back-when-the-daemon-refuses` case in `linked` mode:
/// at `auto`, a Daemon of the home that refuses by Product or by protocol
/// version and cannot hand over leaves the connection to link its own
/// engine, which `engine().fallback` says why, with one warning on
/// `goodfetch::client`.
/// Covers: story:11-9, story:11-290
#[rstest]
#[case::by_product(REFUSED_BY_PRODUCT, "not_in_product", PRODUCT_FIX)]
#[case::by_protocol_version(REFUSED_BY_VERSION, "protocol_version_unsupported", VERSION_FIX)]
fn auto_links_its_own_engine_when_the_daemon_refuses(
    #[case] script: &str,
    #[case] code: &str,
    #[case] given_fix: &str,
) {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "connect-falls-back-when-the-daemon-refuses",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let (subscriber, log) = client_warnings();

    let (process, url, engine) = with_default(subscriber, || {
        let (process, line) = scripted(&env, script, &hello(SCRIPTED_DAEMON_TOKEN), "socket");
        let url = listening(&line);
        let engine = real_time_runtime().block_on(async {
            let collie = Collie::connect(ConnectOptions::default().environment(env.env_os()))
                .await
                .unwrap();
            let engine = collie.engine().clone();
            collie.close().await.unwrap();
            engine
        });
        (process, url, engine)
    });

    played_whole(&process);
    let fallback = engine.fallback.clone().expect("the fallback is reported");
    assert_eq!(fallback.fix, format!("{given_fix} {LINKED_FIX}"));
    assert!(
        fallback.cause.contains(code) && fallback.cause.contains(&url),
        "{}",
        fallback.cause
    );
    assert_eq!(
        engine,
        EngineInfo {
            fallback: Some(fallback),
            ..reached(EngineKind::Linked, Some(env.home().to_owned()), None)
        }
    );
    let written = text(&log);
    let warned: Vec<&str> = written.lines().collect();
    assert_eq!(warned.len(), 1, "{written}");
    assert!(warned[0].contains(&url), "{}", warned[0]);
}

/// The blocking connection falls back too.
/// Covers: story:11-290
#[test]
fn a_blocking_connection_falls_back_too() {
    let env = TestEnv::new();
    let (process, _) = scripted(
        &env,
        REFUSED_BY_VERSION,
        &hello(SCRIPTED_DAEMON_TOKEN),
        "socket",
    );

    let collie =
        blocking::Collie::connect(ConnectOptions::default().environment(env.env_os())).unwrap();
    let engine = collie.engine().clone();
    collie.close().unwrap();

    played_whole(&process);
    assert_eq!(engine.kind, EngineKind::Linked);
    assert!(engine.fallback.is_some(), "{engine:?}");
}

/// The `connect-falls-back-when-the-daemon-refuses` case in `daemon` mode:
/// the refusal fails the connect, its `fix` naming Collie's install
/// command first, then its `daemon restart`; nothing is warned and no
/// linked engine starts.
/// Covers: story:11-9, story:11-293
#[test]
fn daemon_fails_with_the_refusal_whose_fix_names_the_install_command_first() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "connect-falls-back-when-the-daemon-refuses",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (subscriber, log) = client_warnings();

    let (process, refused) = with_default(subscriber, || {
        let (process, _) = scripted(
            &env,
            REFUSED_BY_VERSION,
            &hello(SCRIPTED_DAEMON_TOKEN),
            "socket",
        );
        let refused = real_time_runtime().block_on(async {
            Collie::connect(
                ConnectOptions::default()
                    .environment(env.env_os())
                    .engine(EngineChoice::Daemon),
            )
            .await
        });
        (process, refused)
    });

    played_whole(&process);
    let error = refused.unwrap_err();
    assert_eq!(
        error.code(),
        ErrorCode::PROTOCOL_VERSION_UNSUPPORTED,
        "{error:?}"
    );
    assert_eq!(
        error.fix().unwrap_or_default(),
        format!("{VERSION_FIX} {LINKED_FIX}")
    );
    assert_eq!(text(&log), "");
    let home = Home::new(env.home().to_owned());
    assert_eq!(InstalledList::of(&home).read().unwrap(), Vec::new());
}

/// A remote Daemon's refusal, given in code or by `GOODFETCH_URL`, fails
/// the connect unchanged: it never falls back.
/// Covers: story:11-295
#[rstest]
#[case::in_code(false)]
#[case::goodfetch_url(true)]
fn a_remote_daemons_refusal_fails_the_connect_unchanged(#[case] in_environment: bool) {
    let env = TestEnv::new();
    let (subscriber, log) = client_warnings();

    let (process, refused) =
        with_default(subscriber, || {
            let (process, line) = remote(
                &env,
                REFUSED_BY_VERSION,
                &hello(SCRIPTED_DAEMON_TOKEN),
                &subprotocol(),
            );
            let url = format!("ws://{}", line["address"].as_str().unwrap());
            let options =
                if in_environment {
                    ConnectOptions::default().environment(environment(
                        &env,
                        &[
                            ("GOODFETCH_URL", &url),
                            ("GOODFETCH_TOKEN", SCRIPTED_DAEMON_TOKEN),
                        ],
                    ))
                } else {
                    ConnectOptions::default().environment(env.env_os()).engine(
                        EngineChoice::Remote(RemoteDaemon::new(url, SCRIPTED_DAEMON_TOKEN)),
                    )
                };
            let refused = real_time_runtime().block_on(async { Collie::connect(options).await });
            (process, refused)
        });

    played_whole(&process);
    let error = refused.unwrap_err();
    assert_eq!(
        error.code(),
        ErrorCode::PROTOCOL_VERSION_UNSUPPORTED,
        "{error:?}"
    );
    assert_eq!(error.fix(), Some(VERSION_FIX));
    assert_eq!(text(&log), "");
}

/// An idle Daemon that refuses and hands over is waited past until its
/// major's pick serves, which the connection reaches.
/// Covers: story:11-292
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the blocking connect waits on its own thread while the test starts the pick, never on an async worker"
)]
fn a_daemon_that_hands_over_is_waited_past_until_the_pick_serves() {
    let env = TestEnv::new();
    let (handing, _) = scripted(&env, HANDS_OVER, &hello(SCRIPTED_DAEMON_TOKEN), "socket");
    let options = ConnectOptions::default().environment(env.env_os());

    let (pick, url, engine) = std::thread::scope(|scope| {
        let connecting = scope.spawn(|| {
            let collie = blocking::Collie::connect(options).unwrap();
            let engine = collie.engine().clone();
            collie.close().unwrap();
            engine
        });
        played_whole(&handing);
        // The pick starts once the Daemon that handed over has stopped.
        let (pick, line) = scripted(
            &env,
            REACHED_DAEMON,
            &hello(SCRIPTED_DAEMON_TOKEN),
            "socket",
        );
        // The waiting connection may reach the pick, whose play then ends
        // and takes its runtime file, before this could read it: the
        // address comes from the line the pick printed instead.
        let url = line["address"].as_str().unwrap().to_owned();
        (pick, url, connecting.join().unwrap())
    });

    played_whole(&pick);
    assert_eq!(
        engine,
        reached(EngineKind::Daemon, Some(env.home().to_owned()), Some(url))
    );
}

/// No pick serving within `timeout.connect` of a handover fails the
/// connect with `deadline_exceeded` naming it, whose `fix` points at the
/// Daemon's log.
/// Covers: story:11-292
#[test]
fn no_pick_serving_in_time_after_a_handover_fails_with_deadline_exceeded() {
    let env = TestEnv::new();
    let (handing, _) = scripted(&env, HANDS_OVER, &hello(SCRIPTED_DAEMON_TOKEN), "socket");

    let refused = real_time_runtime().block_on(async {
        Collie::connect(
            ConnectOptions::default()
                .environment(env.env_os())
                .timeout(Duration::from_millis(300)),
        )
        .await
    });

    played_whole(&handing);
    let error = refused.unwrap_err();
    assert_eq!(error.code(), ErrorCode::DEADLINE_EXCEEDED, "{error:?}");
    let details: DeadlineExceededDetails = error.details().unwrap();
    assert_eq!(details.waited_for, WaitedFor::HELLO);
    assert_eq!(
        (
            details.limit.key.as_str(),
            details.limit.value.as_u64(),
            details.limit.layer,
            details.limit.origin,
        ),
        (
            "timeout.connect",
            Some(300),
            SettingsLayer::Run,
            Some(RunOrigin::Argument)
        )
    );
    let logs = Home::new(env.home().to_owned()).logs();
    let fix = error.fix().unwrap_or_default();
    assert!(fix.contains(&logs.display().to_string()), "{fix}");
}

/// A program linking the engine records itself in the home's list of
/// installed executables as a linked program, and runs no Daemon (ADR
/// 0098).
/// Covers: story:11-290
#[test]
fn a_linked_program_records_itself_and_runs_no_daemon() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_owned());

    real_time_runtime().block_on(async {
        let collie = Collie::connect(ConnectOptions::default().environment(env.env_os()))
            .await
            .unwrap();
        collie.close().await.unwrap();
    });

    let recorded = InstalledList::of(&home).read().unwrap();
    let program = std::env::current_exe().unwrap();
    assert_eq!(
        recorded
            .iter()
            .map(|entry| (
                entry.kind,
                entry.product,
                entry.product_version.clone(),
                entry.path.clone()
            ))
            .collect::<Vec<_>>(),
        vec![(
            ExecutableKind::LinkedProgram,
            Product::Collie,
            version(),
            program.display().to_string()
        )]
    );
    assert!(
        DaemonFiles::of(&home, &version())
            .running()
            .unwrap()
            .is_none()
    );
}
