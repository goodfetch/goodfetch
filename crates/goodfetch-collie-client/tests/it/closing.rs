//! Closing the values a program holds through the client crate (ADR 0078):
//! the Run, its Sessions and its pages, each closing its children with it.
//! The scripted cases play the corpus's scripts of the lifecycle client
//! conformance cases, which the TypeScript SDK plays too, with the
//! Scripted engine as the Daemon: it fails on a message it does not
//! expect and on a step left unused, so its clean exit says the crate sent
//! exactly what the script holds, and that a Connection's close only
//! disconnected from the Daemon. The others hand-play the engine over the
//! in-memory network on the paused runtime: a close waits no longer than
//! `timeout.close`, and dropping an unclosed value fires its close without
//! waiting and warns. The scripted cases are Rust's `daemon` mode plays;
//! `goodfetch-collie`'s `sdk::closing` tests play them `linked`.

use std::time::Duration;

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::connection::{
    CallCancelledDetails, CancelledBy, ClosedValue, DaemonEndpoint, HelloParams,
};
use goodfetch_protocol::log::LogLevel;
use goodfetch_protocol::run::{RunId, Tags};
use goodfetch_test_support::{TestEnv, case_logs, logged};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

use crate::connection::{hello, reply, scripted};
use crate::scripted_daemon::{daemon_hello, scripted_daemon};

/// `timeout.close`'s default, which every close waits at most.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

/// A Connection to the Scripted engine's Daemon at `endpoint`, on the
/// test's runtime.
async fn connect(endpoint: DaemonEndpoint) -> ClientConnection {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        daemon_hello(),
    )
    .await
    .unwrap();
    connection
}

/// Waits for the Scripted engine to exit and checks it played its script
/// whole and was sent nothing else.
fn assert_played_whole(process: &goodfetch_test_support::Process) {
    let status = process.child().wait().unwrap();
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}

/// The `children-close-with-their-parent` client conformance case, played
/// by the Scripted engine's Daemon: a Session's close closes its page,
/// whose own close then sends nothing and whose calls fail with
/// `closed`; the Connection's close aborts the Run once, and
/// every handle of it then refuses its calls with `closed`;
/// closing the Connection again does nothing.
/// Covers: story:11-16, story:11-17
#[test]
fn children_close_with_their_parent_and_refuse_every_later_call_with_closed() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/closed-with-its-children.json");
    let runtime = goodfetch_test_support::real_time_runtime();
    let (after_session, after_connection) = runtime.block_on(async {
        let connection = connect(endpoint).await;
        let run = connection.start().await.unwrap();
        let session = run.new_session().await.unwrap();
        let session_page = session.new_page().await.unwrap();

        session.close().await.unwrap();
        session_page.close().await.unwrap();
        let after_session = session_page.goto("https://shop.test/").await;
        let run_page = run.new_page().await.unwrap();
        connection.close().await.unwrap();
        connection.close().await.unwrap();
        let after_connection = (
            run_page.goto("https://shop.test/").await,
            session.new_page().await,
            run.finish().await,
        );
        (after_session, after_connection)
    });

    assert!(
        matches!(&after_session, Err(error) if error.code() == ErrorCode::CLOSED),
        "{after_session:?}"
    );
    assert!(
        matches!(
            &after_connection,
            (Err(a), Err(b), Err(c))
                if [a, b, c].iter().all(|error| error.code() == ErrorCode::CLOSED)
        ),
        "{after_connection:?}"
    );
    assert_played_whole(&process);
}

/// The `closed-after-close` client conformance case, played by the
/// Scripted engine's Daemon: closing a running Run aborts it once, and
/// closing it or the Connection again does nothing; the Run's finish and
/// a start then fail with `closed`.
/// Covers: story:11-17
#[test]
fn a_closed_run_and_connection_refuse_every_later_call_with_closed() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/closed-while-running.json");
    let nightly = RunId::parse("nightly").unwrap();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (finished, started) = runtime.block_on(async {
        let connection = connect(endpoint).await;
        let run = connection
            .start()
            .run_id(&nightly)
            .tags(Tags::parse(["shop"]).unwrap())
            .await
            .unwrap();

        run.close().await.unwrap();
        run.close().await.unwrap();
        connection.close().await.unwrap();
        connection.close().await.unwrap();
        (run.finish().await, connection.start().await)
    });

    assert!(
        matches!(&finished, Err(error) if error.code() == ErrorCode::CLOSED),
        "{finished:?}"
    );
    assert!(
        matches!(&started, Err(error) if error.code() == ErrorCode::CLOSED),
        "{started:?}"
    );
    assert_played_whole(&process);
}

/// The `runtime-errors-in-the-callers-form` client conformance case,
/// played by the Scripted engine's Daemon: a call on a closed Run, a call
/// on the closed Connection, and the call in flight when the Daemon drops
/// the Connection, each fail with a message headed by the call's operation
/// in Rust's spelling, then what failed, then the fix, while `operation`
/// holds the catalog's name.
/// Covers: story:11-31, story:11-192
#[test]
fn the_runtimes_own_errors_head_each_call_in_one_layout() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "runtime-errors-in-the-callers-form",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (closing, closing_endpoint) = scripted_daemon(&env, "run/closed-while-running.json");
    let nightly = RunId::parse("nightly").unwrap();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (finished, started) = runtime.block_on(async {
        let connection = connect(closing_endpoint).await;
        let run = connection
            .start()
            .run_id(&nightly)
            .tags(Tags::parse(["shop"]).unwrap())
            .await
            .unwrap();
        connection.close().await.unwrap();
        (
            run.finish().await.map(drop),
            connection.start().await.map(drop),
        )
    });
    assert_played_whole(&closing);
    let dropping_env = TestEnv::new();
    let (dropping, dropping_endpoint) =
        scripted_daemon(&dropping_env, "run/dropped-during-a-start.json");
    let in_flight = runtime.block_on(async {
        let connection = connect(dropping_endpoint).await;
        connection.start().await.map(drop)
    });
    assert_played_whole(&dropping);

    // What each failure reads as: its code, its operation, and whether its
    // message starts with the operation and ends with its fix.
    let read = |result: &Result<(), goodfetch_collie_client::Error>| {
        result.as_ref().err().map(|error| {
            let operation = error.operation().unwrap_or_default();
            let fix = error.fix().unwrap_or_default();
            (
                error.code(),
                operation.to_owned(),
                error.message().starts_with(&format!("{operation}: ")),
                !fix.is_empty() && error.message().ends_with(&format!(". {fix}")),
            )
        })
    };
    assert_eq!(
        [read(&finished), read(&started), read(&in_flight)],
        [
            Some((ErrorCode::CLOSED, "run.finish".to_owned(), true, true)),
            Some((ErrorCode::CLOSED, "run.start".to_owned(), true, true)),
            Some((ErrorCode::CLOSED, "run.start".to_owned(), true, true)),
        ]
    );
}

/// The `close-stops-a-running-run` client conformance case, played by the
/// Scripted engine's Daemon: closing a running Run aborts it once, a
/// second close and a later call send nothing, a start of its Run ID
/// resumes it, and closing the finished Run sends nothing; the
/// Connection's close then only disconnects from the Daemon.
/// Covers: story:11-18, story:11-19
#[test]
fn a_close_stops_a_running_run_once_and_frees_an_ended_runs_value_without_a_call() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/closed-then-resumed.json");
    let nightly = RunId::parse("nightly").unwrap();
    let runtime = goodfetch_test_support::real_time_runtime();
    let after_close = runtime.block_on(async {
        let connection = connect(endpoint).await;
        let first = connection.start().run_id(&nightly).await.unwrap();

        first.close().await.unwrap();
        first.close().await.unwrap();
        let after_close = first.finish().await;
        let resumed = connection.start().run_id(&nightly).await.unwrap();
        resumed.finish().await.unwrap();
        resumed.close().await.unwrap();
        connection.close().await.unwrap();
        after_close
    });

    assert!(
        matches!(&after_close, Err(error) if error.code() == ErrorCode::CLOSED),
        "{after_close:?}"
    );
    assert_played_whole(&process);
}

/// The `failed-close-raises-a-typed-error` client conformance case,
/// played by the Scripted engine's Daemon: the engine refuses the Run's
/// abort, and the Connection's close fails with that error, closing all
/// the same.
/// Covers: story:11-20
#[test]
fn a_failed_close_fails_with_the_engines_error() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "failed-close-raises-a-typed-error",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/abort-refused.json");
    let nightly = RunId::parse("nightly").unwrap();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (closed, again) = runtime.block_on(async {
        let connection = connect(endpoint).await;
        let _run = connection.start().run_id(&nightly).await.unwrap();
        let closed = connection.close().await;
        (closed, connection.close().await)
    });

    let Err(failure) = &closed else {
        panic!("{closed:?}");
    };
    assert_eq!(failure.code(), ErrorCode::RUN_STATE_UNAVAILABLE);
    assert!(again.is_ok(), "{again:?}");
    assert_played_whole(&process);
}

/// The paused test runtime's instant, which moves only as its timers fire.
fn paused_now() -> tokio::time::Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "the paused test runtime's clock, which only its timers move, in a test (ADR 0059)"
    )]
    tokio::time::Instant::now()
}

/// The engine `run.start` answers on the hand-played link: a running Run.
fn started() -> Value {
    json!({"run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc", "lifecycle": "running", "needs": "collie", "started_as": "new"})
}

/// What the hand-played engine answers `hello`.
fn said() -> Value {
    json!({
        "protocol_version": "0.1",
        "product": "collie",
        "product_version": "0.0.0",
        "engine_instance": "01J0000000000000000000000",
    })
}

/// The Run's report once its abort stopped it.
fn stopped() -> Value {
    json!({
        "run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc",
        "lifecycle": "stopped",
        "verdict": "verified",
        "completeness": "incomplete",
        "stop_reason": {"kind": "abort", "by": "client"},
        "started_at": "2026-01-01T00:00:00Z",
        "ended_at": "2026-01-01T00:00:00Z",
        "usage": {
            "model": {"calls": 0, "tokens": 0, "input_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0, "output_tokens": 0, "reasoning_tokens": 0, "price": null},
            "browser_time": 0,
            "proxy_bytes": 0,
            "requests": 0,
        },
    })
}

/// The call in flight of the `connection-close-stops-its-engine` client
/// conformance case, hand-played over the in-memory network: the
/// Connection's close aborts its running Run, then cancels the call in
/// flight, which fails with `call_cancelled` as the program's closing of
/// its Connection stopped it, and ends the engine's input.
/// Covers: story:11-19, story:11-48
#[test]
fn a_connection_close_aborts_its_run_fails_a_call_in_flight_and_ends_the_engines_input() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (finished, closed, heard) = runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            let start = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&start, &started())).await.unwrap();
            let mut heard = Vec::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                let request: Value = serde_json::from_str(&line).unwrap();
                if request["method"] == "run.abort" {
                    writer.write_all(&reply(&line, &stopped())).await.unwrap();
                }
                heard.push(request["method"].as_str().unwrap().to_owned());
            }
            heard.push("the end of the input".to_owned());
            writer.shutdown().await.unwrap();
            heard
        };
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let run = connection.start().await.unwrap();
            tokio::join!(run.finish(), connection.close())
        };
        let ((finished, closed), heard) = tokio::join!(client, peer);
        (finished, closed, heard)
    });

    let cancelled = finished.unwrap_err();
    assert_eq!(cancelled.code(), ErrorCode::CALL_CANCELLED, "{cancelled:?}");
    assert_eq!(
        cancelled.details::<CallCancelledDetails>(),
        Some(CallCancelledDetails {
            by: CancelledBy::Program,
            closed: Some(ClosedValue::Connection),
            person: None,
        })
    );
    assert!(closed.is_ok(), "{closed:?}");
    assert_eq!(heard, ["run.finish", "run.abort", "the end of the input"]);
}

/// A Run's close whose abort is still in flight when the program closes
/// the Connection succeeds: the Connection's close ends the Run's work
/// with it, and the Run is closed already, so the abort the close
/// cancelled fails nothing.
/// Covers: story:11-18, story:11-19
#[test]
fn a_runs_close_whose_abort_the_connections_close_cancels_succeeds() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (run_closed, connection_closed, heard) = runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            let start = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&start, &started())).await.unwrap();
            // The abort goes unanswered: the Connection's close comes first.
            let mut heard = Vec::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                let request: Value = serde_json::from_str(&line).unwrap();
                heard.push(request["method"].as_str().unwrap().to_owned());
            }
            heard.push("the end of the input".to_owned());
            writer.shutdown().await.unwrap();
            heard
        };
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let run = connection.start().await.unwrap();
            tokio::join!(run.close(), connection.close())
        };
        let ((run_closed, connection_closed), heard) = tokio::join!(client, peer);
        (run_closed, connection_closed, heard)
    });

    assert!(run_closed.is_ok(), "{run_closed:?}");
    assert!(connection_closed.is_ok(), "{connection_closed:?}");
    assert_eq!(heard, ["run.abort", "the end of the input"]);
}

/// A start whose `connection.set_log_level` is in flight when the program
/// closes the Connection fails as the close cancelled it, and warns of no
/// refused level: the engine refused nothing.
/// Covers: story:11-19, story:11-26
#[test]
fn a_start_whose_log_level_the_connections_close_cancels_warns_of_no_refusal() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (told, told_heard) = tokio::sync::oneshot::channel::<()>();
    let ((started, heard), log) = logged(|| {
        runtime.block_on(async {
            let peer = async {
                let hello = lines.next_line().await.unwrap().unwrap();
                writer.write_all(&reply(&hello, &said())).await.unwrap();
                // The level goes unanswered: the Connection's close comes first.
                let mut heard = Vec::new();
                let mut told = Some(told);
                while let Some(line) = lines.next_line().await.unwrap() {
                    let request: Value = serde_json::from_str(&line).unwrap();
                    heard.push(request["method"].as_str().unwrap().to_owned());
                    if let Some(told) = told.take() {
                        told.send(()).unwrap();
                    }
                }
                heard.push("the end of the input".to_owned());
                writer.shutdown().await.unwrap();
                heard
            };
            let client = async {
                let (connection, _) = ClientConnection::open(
                    ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                    &network,
                    &Endpoint::InMemory,
                    // A level other than the `trace` that `logged` records,
                    // so the start tells the engine the new one.
                    HelloParams {
                        log_level: Some(LogLevel::Warn),
                        ..hello()
                    },
                )
                .await
                .unwrap();
                let closing = async {
                    told_heard.await.unwrap();
                    connection.close().await
                };
                let (started, closed) = tokio::join!(connection.start(), closing);
                closed.unwrap();
                started.map(|_run| ())
            };
            tokio::join!(client, peer)
        })
    });

    let cancelled = started.unwrap_err();
    assert_eq!(cancelled.code(), ErrorCode::CALL_CANCELLED, "{cancelled:?}");
    assert_eq!(heard, ["connection.set_log_level", "the end of the input"]);
    let log = String::from_utf8(log).unwrap();
    assert!(!log.contains("did not take the log level"), "{log}");
}

/// A page's close and a Connection's close the engine never answers each
/// fail with `deadline_exceeded` once `timeout.close` has run out, and no
/// later; the Connection is closed all the same, and closing it again
/// does nothing.
/// Covers: story:11-20
#[test]
fn a_close_the_engine_never_answers_fails_once_timeout_close_runs_out() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let (page_closed, connection_closed, unanswered) = runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            let start = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&start, &started())).await.unwrap();
            let new_page = lines.next_line().await.unwrap().unwrap();
            writer
                .write_all(&reply(
                    &new_page,
                    &json!({"page_id": "page-1", "session_id": "session-1"}),
                ))
                .await
                .unwrap();
            let mut unanswered = Vec::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                let request: Value = serde_json::from_str(&line).unwrap();
                unanswered.push(request["method"].as_str().unwrap().to_owned());
            }
            // The engine keeps its output open past the client's close.
            released.await.unwrap();
            writer.shutdown().await.unwrap();
            unanswered
        };
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let run = connection.start().await.unwrap();
            let page = run.new_page().await.unwrap();
            let began = paused_now();
            let page_closed = (page.close().await, began.elapsed());
            let began = paused_now();
            let connection_closed = (connection.close().await, began.elapsed());
            let began = paused_now();
            let again = (connection.close().await, began.elapsed());
            release.send(()).unwrap();
            (page_closed, (connection_closed, again))
        };
        let ((page_closed, connection_closed), unanswered) = tokio::join!(client, peer);
        (page_closed, connection_closed, unanswered)
    });
    let (connection_closed, again) = connection_closed;

    for (closed, took) in [&page_closed, &connection_closed] {
        assert!(
            matches!(&closed, Err(error) if error.code() == ErrorCode::DEADLINE_EXCEEDED),
            "{closed:?}"
        );
        assert!(
            *took >= CLOSE_TIMEOUT && *took < CLOSE_TIMEOUT + Duration::from_secs(1),
            "{took:?}"
        );
    }
    assert!(again.0.is_ok(), "{again:?}");
    assert_eq!(again.1, Duration::ZERO);
    assert_eq!(unanswered, ["page.close", "run.abort"]);
}

/// Dropping the last handle of a Session or a page that is still open,
/// while its Run runs, sends its close without waiting for the reply and
/// warns; dropping one already closed sends nothing, and closing it twice
/// sends its close once.
/// Covers: story:11-21
#[test]
fn dropping_an_unclosed_value_sends_its_close_without_waiting_and_warns() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (sent, log) = logged(|| {
        runtime.block_on(async {
            let peer = async {
                let hello = lines.next_line().await.unwrap().unwrap();
                writer.write_all(&reply(&hello, &said())).await.unwrap();
                let mut sent = Vec::new();
                let mut opened = 0;
                while let Some(line) = lines.next_line().await.unwrap() {
                    let request: Value = serde_json::from_str(&line).unwrap();
                    let method = request["method"].as_str().unwrap().to_owned();
                    let result = match method.as_str() {
                        "run.start" => started(),
                        "run.new_session" => {
                            opened += 1;
                            json!({"session_id": format!("session-{opened}")})
                        }
                        "run.new_page" => {
                            opened += 1;
                            json!({"page_id": format!("page-{opened}"), "session_id": "session-0"})
                        }
                        _ => json!({}),
                    };
                    // A drop sends its close without waiting, and the
                    // Connection may close before its reply is written.
                    let _unread = writer.write_all(&reply(&line, &result)).await;
                    sent.push((method, request["params"].clone()));
                }
                writer.shutdown().await.unwrap();
                sent
            };
            let client = async {
                let (connection, _) = ClientConnection::open(
                    ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                    &network,
                    &Endpoint::InMemory,
                    // The `trace` that `logged` records, so the start
                    // tells the engine no other level.
                    HelloParams {
                        log_level: Some(LogLevel::Trace),
                        ..hello()
                    },
                )
                .await
                .unwrap();
                let run = connection.start().await.unwrap();
                let closed = run.new_session().await.unwrap();
                closed.close().await.unwrap();
                closed.close().await.unwrap();
                drop(closed);
                let session = run.new_session().await.unwrap();
                drop(session);
                let page = run.new_page().await.unwrap();
                let clone = page.clone();
                drop(page);
                drop(clone);
                drop(run);
                connection.close().await.unwrap();
            };
            let ((), sent) = tokio::join!(client, peer);
            sent
        })
    });

    let run_id = json!({"run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc"});
    assert_eq!(
        sent,
        [
            ("run.start".to_owned(), json!({})),
            ("run.new_session".to_owned(), run_id.clone()),
            (
                "session.close".to_owned(),
                json!({"session_id": "session-1"})
            ),
            ("run.new_session".to_owned(), run_id.clone()),
            (
                "session.close".to_owned(),
                json!({"session_id": "session-2"})
            ),
            ("run.new_page".to_owned(), run_id.clone()),
            ("page.close".to_owned(), json!({"page_id": "page-3"})),
            ("run.abort".to_owned(), run_id),
        ]
    );
    let log = String::from_utf8(log).unwrap();
    let warnings: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("WARN") && line.contains("without close()"))
        .collect();
    assert_eq!(warnings.len(), 3, "{log}");
    for (warning, named) in
        warnings
            .iter()
            .zip(["session-2", "page-3", "01ke6p4ym0j8x3v5wq2rtbn7hc"])
    {
        assert!(warning.contains(named), "{log}");
    }
}

/// A Connection's close that `timeout.close` cuts short, its first Run's
/// abort unanswered, still leaves every Run it started closed: dropping
/// the Run it had not reached yet sends nothing and does not warn.
/// Covers: story:11-19, story:11-21
#[test]
fn a_run_a_timed_out_connection_close_did_not_reach_is_closed_all_the_same() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let ((connection_closed, unanswered), log) = logged(|| {
        runtime.block_on(async {
            let peer = async {
                let hello = lines.next_line().await.unwrap().unwrap();
                writer.write_all(&reply(&hello, &said())).await.unwrap();
                for run_id in ["01ke6p4ym0j8x3v5wq2rtbn7hc", "01ke6p4ym0j8x3v5wq2rtbn7hd"] {
                    let start = lines.next_line().await.unwrap().unwrap();
                    let started = json!({"run_id": run_id, "lifecycle": "running", "needs": "collie", "started_as": "new"});
                    writer.write_all(&reply(&start, &started)).await.unwrap();
                }
                let mut unanswered = Vec::new();
                while let Some(line) = lines.next_line().await.unwrap() {
                    let request: Value = serde_json::from_str(&line).unwrap();
                    unanswered.push((
                        request["method"].as_str().unwrap().to_owned(),
                        request["params"]["run_id"].clone(),
                    ));
                }
                // The engine keeps its output open past the client's close.
                released.await.unwrap();
                writer.shutdown().await.unwrap();
                unanswered
            };
            let client = async {
                let (connection, _) = ClientConnection::open(
                    ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                    &network,
                    &Endpoint::InMemory,
                    // The `trace` that `logged` records, so the start
                    // tells the engine no other level.
                    HelloParams {
                        log_level: Some(LogLevel::Trace),
                        ..hello()
                    },
                )
                .await
                .unwrap();
                let first = connection.start().await.unwrap();
                let second = connection.start().await.unwrap();
                let connection_closed = connection.close().await;
                drop(second);
                drop(first);
                release.send(()).unwrap();
                connection_closed
            };
            tokio::join!(client, peer)
        })
    });

    assert!(
        matches!(&connection_closed, Err(error) if error.code() == ErrorCode::DEADLINE_EXCEEDED),
        "{connection_closed:?}"
    );
    assert_eq!(
        unanswered,
        [("run.abort".to_owned(), json!("01ke6p4ym0j8x3v5wq2rtbn7hc"))]
    );
    let log = String::from_utf8(log).unwrap();
    assert!(!log.contains("without close()"), "{log}");
}
