//! The engine's cap on calls in flight, kept by the Client (ADR 0038's
//! note of 2026-11-21; issue 3189): `hello`'s result says how many calls
//! one Connection keeps in flight, the Connection sends at most that many
//! at once and queues the rest in the order they were made, a queued call
//! keeps its own timeout and cancellation without being sent, and the
//! queue is bounded by `connection.max_pending_calls` and
//! `connection.max_queued_write_bytes`, past which a call fails at once
//! with `connection_overloaded`. Over a peer the test plays on the
//! in-memory network, on the paused clock, which moves only once the
//! Client has nothing left to do.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::VecDeque;
use std::future::{Future, IntoFuture as _};
use std::pin::pin;
use std::task::Poll;
use std::time::Duration;

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode, Reply};
use goodfetch_protocol::connection::{
    ConnectionOverloadedDetails, DeadlineExceededDetails, HelloParams, OverloadedBound,
    TOO_MANY_CALLS, WaitedFor,
};
use goodfetch_protocol::engine::ENGINE_INFO;
use goodfetch_protocol::page::PAGE_GOTO;
use goodfetch_protocol::settings::{RunOrigin, SettingInvalidDetails, SettingsLayer};
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt as _, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf};

use crate::connection::{hello, reply, said, scripted};

/// The engine's own cap: as many calls in flight as it serves on one
/// Connection.
const CAP: usize = 256;

/// What the peer reads of the Connection.
type PeerLines = Lines<BufReader<ReadHalf<DuplexStream>>>;

/// What the peer writes to the Connection.
type PeerWriter = WriteHalf<DuplexStream>;

/// How long the peer waits for another line before it takes the Client
/// to have sent all it can: on the paused clock, the time passes only
/// once the Client waits on nothing but the peer.
const IDLE: Duration = Duration::from_millis(1);

/// What the engine answers `hello` with when it keeps `cap` calls in
/// flight.
fn capped(cap: usize) -> Value {
    let mut said = said();
    said["max_calls_in_flight"] = json!(cap);
    said
}

/// A Rust program's `hello` with the Connection's own `settings`.
fn hello_with(settings: Value) -> HelloParams {
    let mut hello = hello();
    hello.settings = serde_json::from_value(settings).unwrap();
    hello
}

/// The `too_many_calls` the engine refuses the request `line` with.
fn too_many(line: &str) -> Vec<u8> {
    let request: Value = serde_json::from_str(line).unwrap();
    let mut answer = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": request["id"],
        "error": {
            "code": TOO_MANY_CALLS.group.json_rpc_class(),
            "message": "the Connection already holds as many calls in flight as the engine serves",
            "data": { "code": TOO_MANY_CALLS.name, "retryable": TOO_MANY_CALLS.retryable, "max": CAP },
        },
    }))
    .unwrap();
    answer.push(b'\n');
    answer
}

/// What a peer read once its Client closed: each request's method and
/// its params' `index`, in the order read, and how many it refused.
#[derive(Debug, Default, PartialEq, Eq)]
struct Read {
    requests: Vec<(String, Option<u64>)>,
    refused: usize,
}

/// Plays an engine that keeps `cap` calls in flight after answering
/// `hello` with `said`: a request read while `cap` wait is refused with
/// `too_many_calls`, and the oldest is answered once the Client has sent
/// all it can, until the Client closes.
async fn serve(mut lines: PeerLines, mut writer: PeerWriter, said: Value, cap: usize) -> Read {
    let hello = lines.next_line().await.unwrap().unwrap();
    writer.write_all(&reply(&hello, &said)).await.unwrap();
    let mut held: VecDeque<String> = VecDeque::new();
    let mut read = Read::default();
    loop {
        let line = if held.is_empty() {
            lines.next_line().await.unwrap()
        } else if let Ok(line) = tokio::time::timeout(IDLE, lines.next_line()).await {
            line.unwrap()
        } else {
            let oldest = held.pop_front().unwrap();
            writer.write_all(&reply(&oldest, &json!({}))).await.unwrap();
            continue;
        };
        let Some(line) = line else { break };
        let request: Value = serde_json::from_str(&line).unwrap();
        read.requests.push((
            request["method"].as_str().unwrap().to_owned(),
            request["params"]["index"].as_u64(),
        ));
        if held.len() >= cap {
            read.refused += 1;
            writer.write_all(&too_many(&line)).await.unwrap();
        } else {
            held.push_back(line);
        }
    }
    writer.shutdown().await.unwrap();
    read
}

/// Opens a Connection over `network` with `hello`, runs `calls` on it,
/// then closes it.
async fn on_connection<T>(
    network: &Network,
    hello: HelloParams,
    calls: impl AsyncFnOnce(&ClientConnection) -> T,
) -> T {
    let (connection, _) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        network,
        &Endpoint::InMemory,
        hello,
    )
    .await
    .unwrap();
    let done = calls(&connection).await;
    connection.close().await.unwrap();
    done
}

/// Awaits every reply at once, each first polled, so sent, in the order
/// given, which is the order the calls were made.
async fn all<T>(mut replies: Vec<Reply<T>>) -> Vec<Result<T, Error>> {
    let mut answered: Vec<Option<Result<T, Error>>> = replies.iter().map(|_| None).collect();
    std::future::poll_fn(|context| {
        for (reply, answered) in replies.iter_mut().zip(&mut answered) {
            if answered.is_none()
                && let Poll::Ready(outcome) = reply.as_mut().poll(context)
            {
                *answered = Some(outcome);
            }
        }
        if answered.iter().all(Option::is_some) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    answered.into_iter().map(Option::unwrap).collect()
}

/// The `engine.info` call numbered `index` in its params.
fn numbered(connection: &ClientConnection, index: u64) -> Reply<Value> {
    connection
        .call::<Value>(ENGINE_INFO.name, &json!({ "index": index }))
        .into_future()
}

/// 300 calls started at once against an engine that keeps [`CAP`] in
/// flight: what they ended with, and what the engine read.
fn three_hundred_at_once() -> (Vec<Result<Value, Error>>, Read) {
    let (network, lines, writer) = scripted();
    goodfetch_test_support::runtime().block_on(async {
        let client = on_connection(&network, hello(), async |connection| {
            all((0..300).map(|index| numbered(connection, index)).collect()).await
        });
        tokio::join!(client, serve(lines, writer, capped(CAP), CAP))
    })
}

/// 300 calls started at once on one Connection all get their answers,
/// and none is refused with `too_many_calls`: the Client sends at most
/// the engine's cap and queues the rest (AC 1).
///
/// Covers: story:11-60, error:too_many_calls
#[test]
fn three_hundred_calls_at_once_all_get_answers_and_none_too_many_calls() {
    let (outcomes, read) = three_hundred_at_once();

    let failed: Vec<&Error> = outcomes.iter().filter_map(|outcome| outcome.as_ref().err()).collect();
    assert_eq!((failed.len(), read.refused), (0, 0), "{failed:?}");
}

/// The calls the Client queued leave the queue in the order they were
/// made (AC 4).
///
/// Covers: story:11-60
#[test]
fn queued_calls_leave_the_queue_in_the_order_they_were_made() {
    let (_, read) = three_hundred_at_once();

    let indexes: Vec<Option<u64>> = read.requests.iter().map(|(_, index)| *index).collect();
    assert_eq!(indexes, (0..300).map(Some).collect::<Vec<_>>());
}

/// A queued call whose own timeout passes before a place frees fails with
/// `deadline_exceeded`, waiting for `calls_in_flight`, its limit the call's
/// `timeout`, and is never sent (AC 2).
///
/// Covers: story:11-60, error:deadline_exceeded
#[test]
fn a_queued_call_past_its_timeout_fails_with_it_and_is_never_sent() {
    let (network, lines, writer) = scripted();
    let (outcomes, read) = goodfetch_test_support::runtime().block_on(async {
        let client = on_connection(&network, hello(), async |connection| {
            let first = numbered(connection, 0);
            let waiting = connection
                .call::<Value>(ENGINE_INFO.name, &json!({ "index": 1 }))
                .timeout(Duration::from_millis(5))
                .into_future();
            all(vec![first, waiting]).await
        });
        let peer = async {
            let (mut lines, mut writer) = (lines, writer);
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &capped(1))).await.unwrap();
            // Holds the one place past the queued call's timeout.
            let first = lines.next_line().await.unwrap().unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
            writer.write_all(&reply(&first, &json!({}))).await.unwrap();
            let request: Value = serde_json::from_str(&first).unwrap();
            let mut read = vec![request["params"]["index"].as_u64()];
            read.extend(serve_answered(lines, writer).await);
            read
        };
        tokio::join!(client, peer)
    });

    let Err(failure) = &outcomes[1] else {
        panic!("{outcomes:?}");
    };
    let details: DeadlineExceededDetails = failure.details().unwrap();
    assert_eq!(
        (
            failure.code(),
            details.waited_for,
            details.limit.key.as_str(),
            details.limit.value.clone(),
            details.limit.layer,
            details.limit.origin,
            outcomes[0].is_ok(),
            read,
        ),
        (
            ErrorCode::DEADLINE_EXCEEDED,
            WaitedFor::CALLS_IN_FLIGHT,
            "timeout",
            json!(5),
            SettingsLayer::Run,
            Some(RunOrigin::Argument),
            true,
            vec![Some(0)],
        )
    );
}

/// A queued call its caller stops waiting for, by dropping its future,
/// leaves the queue unsent: neither it nor its `$/cancel` reaches the
/// engine (AC 2).
///
/// Covers: story:11-60
#[test]
fn a_cancelled_queued_call_is_never_sent() {
    let (network, lines, writer) = scripted();
    let (answered, read) = goodfetch_test_support::runtime().block_on(async {
        let client = on_connection(&network, hello(), async |connection| {
            let first = numbered(connection, 0);
            // Boxed, so dropping it drops the call.
            let mut dropped = connection
                .call::<Value>(PAGE_GOTO.name, &json!({ "index": 1 }))
                .into_future();
            let mut first = pin!(first);
            std::future::poll_fn(|context| {
                assert!(first.as_mut().poll(context).is_pending());
                assert!(dropped.as_mut().poll(context).is_pending());
                Poll::Ready(())
            })
            .await;
            drop(dropped);
            first.await
        });
        tokio::join!(client, serve(lines, writer, capped(1), 1))
    });

    assert_eq!(
        (answered.is_ok(), read.requests),
        (true, vec![(ENGINE_INFO.name.to_owned(), Some(0))])
    );
}

/// Past `connection.max_pending_calls`, set in the Connection's own
/// settings, the next call fails at once with `connection_overloaded`,
/// the bound `pending_calls` and its value, unsent, while the calls made
/// before it go on (AC 3).
///
/// Covers: story:11-60, setting:connection.max_pending_calls, error:connection_overloaded
#[test]
fn a_call_past_the_pending_calls_bound_fails_at_once_overloaded() {
    let (network, lines, writer) = scripted();
    let settings = json!({ "connection": { "max_pending_calls": 2 } });
    let (outcomes, read) = goodfetch_test_support::runtime().block_on(async {
        let client = on_connection(&network, hello_with(settings), async |connection| {
            all((0..3).map(|index| numbered(connection, index)).collect()).await
        });
        tokio::join!(client, serve(lines, writer, capped(CAP), CAP))
    });

    let Err(refused) = &outcomes[2] else {
        panic!("{outcomes:?}");
    };
    assert_eq!(
        (
            refused.code(),
            refused.details::<ConnectionOverloadedDetails>(),
            outcomes[0].is_ok() && outcomes[1].is_ok(),
            read.requests.len(),
        ),
        (
            ErrorCode::CONNECTION_OVERLOADED,
            Some(ConnectionOverloadedDetails {
                bound: OverloadedBound::PendingCalls,
                value: 2,
            }),
            true,
            2,
        )
    );
}

/// While bytes wait to be written, a call whose message would pass
/// `connection.max_queued_write_bytes` fails at once with
/// `connection_overloaded` and the bound `queued_write_bytes`, while one
/// message larger than the bound is still taken when nothing waits.
///
/// Covers: setting:connection.max_queued_write_bytes, error:connection_overloaded
#[test]
fn a_call_past_the_queued_write_bytes_bound_fails_at_once_overloaded() {
    // The in-memory link holds 64 KiB the engine has not read, so the
    // larger call waits to be written while the engine reads nothing.
    let (network, lines, writer) = scripted();
    let settings = json!({ "connection": { "max_queued_write_bytes": 64 * 1024 } });
    let large = "x".repeat(100 * 1024);
    let (outcomes, read) = goodfetch_test_support::runtime().block_on(async {
        let client = on_connection(&network, hello_with(settings), async |connection| {
            let large = connection
                .call::<Value>(ENGINE_INFO.name, &json!({ "index": 0, "large": large }))
                .into_future();
            all(vec![large, numbered(connection, 1)]).await
        });
        let peer = async {
            let (mut lines, mut writer) = (lines, writer);
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            // Reads nothing until the Client has done all it can.
            tokio::time::sleep(IDLE).await;
            serve_answered(lines, writer).await
        };
        tokio::join!(client, peer)
    });

    let Err(refused) = &outcomes[1] else {
        panic!("{outcomes:?}");
    };
    assert_eq!(
        (
            refused.code(),
            refused.details::<ConnectionOverloadedDetails>(),
            outcomes[0].is_ok(),
            read,
        ),
        (
            ErrorCode::CONNECTION_OVERLOADED,
            Some(ConnectionOverloadedDetails {
                bound: OverloadedBound::QueuedWriteBytes,
                value: 64 * 1024,
            }),
            true,
            vec![Some(0)],
        )
    );
}

/// Answers each request at once until the Client closes: the `index` of
/// each, in the order read.
async fn serve_answered(mut lines: PeerLines, mut writer: PeerWriter) -> Vec<Option<u64>> {
    let mut read = Vec::new();
    while let Some(line) = lines.next_line().await.unwrap() {
        let request: Value = serde_json::from_str(&line).unwrap();
        read.push(request["params"]["index"].as_u64());
        writer.write_all(&reply(&line, &json!({}))).await.unwrap();
    }
    writer.shutdown().await.unwrap();
    read
}

/// A bound of the Connection's own that is no whole number of 1 or more
/// refuses the opening with `setting_invalid`, its key and the run layer,
/// before the engine is reached.
///
/// Covers: setting:connection.max_pending_calls, setting:connection.max_queued_write_bytes, error:setting_invalid
#[rstest::rstest]
#[case::no_call_at_all("max_pending_calls", json!(0))]
#[case::a_fraction_of_a_byte("max_queued_write_bytes", json!(1.5))]
#[case::no_number("max_pending_calls", json!("ten"))]
fn a_bound_that_is_no_whole_number_from_one_refuses_the_opening(
    #[case] bound: &str,
    #[case] value: Value,
) {
    let (network, _lines, _writer) = scripted();
    let opened = goodfetch_test_support::runtime().block_on(async {
        ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &network,
            &Endpoint::InMemory,
            hello_with(json!({ "connection": { bound: value } })),
        )
        .await
    });

    let Err(refused) = opened else {
        panic!("the opening was not refused");
    };
    let details: SettingInvalidDetails = refused.details().unwrap();
    assert_eq!(
        (refused.code(), details.key, details.layer),
        (
            ErrorCode::SETTING_INVALID,
            format!("connection.{bound}"),
            Some(SettingsLayer::Run),
        )
    );
}
