//! What a failed accept does to a Daemon: one failed attempt ends alone and
//! the Daemon keeps serving, a listener closed for good drains its Runs and
//! ends it. The in-memory dialer injects each failure as the OS reports it
//! (ADR 0060).

use std::io;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::json;
use tokio::time::Instant;

use super::super::run::{call, run_folder, run_row};
use super::super::{engine, engine_with};
use super::{cli_hello, daemon, opened, resumed, stuck, the_pick};
use crate::daemon::DaemonOptions;

/// `daemon.drain_grace`'s default.
const DRAIN_GRACE: Duration = Duration::from_secs(110);

/// The OS's error for a process with no descriptor left: `EMFILE`, or
/// Winsock's `WSAEMFILE`.
fn too_many_open_files() -> io::Error {
    #[cfg(unix)]
    let code = libc::EMFILE;
    #[cfg(not(unix))]
    let code = 10024;
    io::Error::from_raw_os_error(code)
}

/// The OS's error for a descriptor that is no socket any more: `EBADF`, or
/// Winsock's `WSAENOTSOCK`.
fn closed_listener() -> io::Error {
    #[cfg(unix)]
    let code = libc::EBADF;
    #[cfg(not(unix))]
    let code = 10038;
    io::Error::from_raw_os_error(code)
}

/// A kept-alive pick, so only what the test does ends it.
fn kept_alive() -> DaemonOptions {
    DaemonOptions {
        keep_alive: true,
        ..the_pick()
    }
}

/// One accept that fails with `EMFILE` ends that attempt alone: the Daemon
/// accepts again at once, and the next Client opens its Connection and
/// calls.
#[test]
fn a_daemon_whose_accept_failed_once_serves_the_next_client_at_once() {
    let (end, (waited, status)) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        kept_alive(),
        |dialer| async move {
            let began = Instant::now();
            dialer.fail_accept(too_many_open_files()).await.unwrap();
            let mut client = opened(&dialer, cli_hello()).await;
            let waited = began.elapsed();
            let status = call(&mut client, 1, "daemon.status", json!({})).await;
            call(&mut client, 2, "daemon.stop", json!({})).await;
            client.close().await;
            (waited, status)
        },
    );

    assert_eq!(
        (
            waited,
            status["result"]["clients"].clone(),
            format!("{end:?}")
        ),
        (Duration::ZERO, json!(1), "Stopped".to_owned())
    );
}

/// While accepts keep failing, as they do until a descriptor is free, the
/// Daemon waits a little longer before each new attempt, never more than a
/// second, and serves the first Client it can accept.
#[test]
fn a_daemon_whose_accepts_keep_failing_backs_off_and_then_serves() {
    let (end, (waited, status)) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        kept_alive(),
        |dialer| async move {
            let began = Instant::now();
            for _ in 0..12 {
                dialer.fail_accept(too_many_open_files()).await.unwrap();
            }
            let mut client = opened(&dialer, cli_hello()).await;
            let waited = began.elapsed();
            let status = call(&mut client, 1, "daemon.status", json!({})).await;
            call(&mut client, 2, "daemon.stop", json!({})).await;
            client.close().await;
            (waited, status)
        },
    );

    // Eleven waits after the first failure: 10 ms doubling to 640 ms, then
    // one second each.
    let backed_off = Duration::from_millis(10 + 20 + 40 + 80 + 160 + 320 + 640 + 4 * 1000);
    assert_eq!(
        (
            waited,
            status["result"]["clients"].clone(),
            format!("{end:?}")
        ),
        (backed_off, json!(1), "Stopped".to_owned())
    );
}

/// A listener closed for good ends the Daemon as a listener failure, once
/// its Runs drained as on a requested drain: each live Run stops at its
/// next safe point with a Checkpoint, `stopped` for the drain and resumable
/// by its Run ID.
#[test]
fn a_daemon_whose_listener_closed_drains_its_runs_and_ends_as_a_listener_failure() {
    let env = TestEnv::new();
    let engine = engine(Product::Collie);

    let (end, closed) = daemon(&engine, &env, kept_alive(), |dialer| async move {
        let mut client = opened(&dialer, cli_hello()).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "drained" })).await;
        dialer.fail_accept(closed_listener()).await.unwrap();
        client.next().await.is_none()
    });

    let folder = run_folder(&env, "drained");
    let stopped = run_row(&folder);
    assert_eq!(
        (
            format!("{end:?}"),
            closed,
            stopped["lifecycle"].clone(),
            stopped["stop_reason"].clone(),
            folder.join("checkpoint.sqlite").is_file(),
            resumed(&engine, &env, "drained"),
        ),
        (
            "ListenerFailed".to_owned(),
            true,
            json!("stopped"),
            json!({ "kind": "drain" }),
            true,
            json!("resumed"),
        )
    );
}

/// The drain a closed listener starts ends at the drain grace, the default
/// of `daemon.drain_grace` when the Daemon's starter gave none: a Run that
/// reached no safe point by then stays `running`, resumable from its run
/// file, never stopped as aborted by its Client.
#[test]
fn a_daemon_whose_listener_closed_ends_at_its_drain_grace_leaving_a_stuck_run_resumable() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(stuck()));

    let (end, waited) = daemon(&engine, &env, kept_alive(), |dialer| async move {
        let mut client = opened(&dialer, cli_hello()).await;
        call(&mut client, 1, "test.stuck", json!({ "run_id": "stuck" })).await;
        let began = Instant::now();
        dialer.fail_accept(closed_listener()).await.unwrap();
        while client.next().await.is_some() {}
        began.elapsed()
    });

    assert_eq!(
        (
            format!("{end:?}"),
            waited,
            run_row(&run_folder(&env, "stuck"))["lifecycle"].clone(),
            resumed(&engine, &env, "stuck"),
        ),
        (
            "ListenerFailed".to_owned(),
            DRAIN_GRACE,
            json!("running"),
            json!("resumed"),
        )
    );
}
