//! The bounds of a Connection's opening on a listener that takes a token
//! (#2324), on the paused `Clock`: a Client that never says `hello` is
//! closed at `timeout.hello`, one whose first message passes
//! `network.max_hello_size` is closed as soon as it does, and a run of
//! wrong tokens records at most `audit.refusals_per_minute` entries a
//! minute, plus one that counts the rest, while every refusal is logged.

use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

use super::consent_walls::audit_entries;
use super::daemon::{cli_hello, connect, daemon, hello_with, the_pick};
use super::{engine, records_of};
use crate::endpoint::REFUSAL_TARGET;

/// `timeout.hello`'s default.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How many refusals of one listener a minute records by default.
const RECORDED_PER_MINUTE: usize = 10;

/// How many of `records` log a refusal on the Daemon's socket for
/// `reason`.
fn refusals(records: &[String], reason: &str) -> usize {
    let logged = format!("listener=\"daemon_socket\" reason=\"{reason}\"");
    records
        .iter()
        .filter(|record| record.contains(&logged))
        .count()
}

/// Covers: setting:timeout.hello
#[test]
fn a_client_that_never_says_hello_is_closed_at_the_deadline() {
    let mut closed = None;
    let records = records_of(REFUSAL_TARGET, || {
        let (_, after) = daemon(
            &engine(Product::Collie),
            &TestEnv::new(),
            the_pick(),
            |dialer| async move {
                let mut client = connect(&dialer).await;
                let connected = tokio::time::Instant::now();
                let ended = client.next().await.is_none();
                (ended, connected.elapsed())
            },
        );
        closed = Some(after);
    });

    assert_eq!(
        (closed, refusals(&records, "hello_timed_out")),
        (Some((true, HELLO_TIMEOUT)), 1),
        "{records:#?}"
    );
}

/// Covers: setting:network.max_hello_size
#[test]
fn a_first_line_past_the_opening_bound_is_closed_before_it_ends_with_one_refusal() {
    let mut closed = None;
    let records = records_of(REFUSAL_TARGET, || {
        let (_, after) = daemon(
            &engine(Product::Collie),
            &TestEnv::new(),
            the_pick(),
            |dialer| async move {
                let mut client = connect(&dialer).await;
                let connected = tokio::time::Instant::now();
                // 16 MiB with no newline, written while the engine reads;
                // a write the closed Connection refuses ends it.
                let chunk = vec![b'x'; 64 * 1024];
                for _ in 0..256 {
                    if client.writer.write_all(&chunk).await.is_err() {
                        break;
                    }
                }
                let ended = client.next().await.is_none();
                (ended, connected.elapsed() < HELLO_TIMEOUT)
            },
        );
        closed = Some(after);
    });

    assert_eq!(
        (
            closed,
            refusals(&records, "hello_too_large"),
            refusals(&records, "hello_timed_out")
        ),
        (Some((true, true)), 1, 0),
        "{records:#?}"
    );
}

/// Covers: setting:audit.refusals_per_minute
#[test]
fn a_thousand_wrong_tokens_record_the_minutes_limit_and_one_entry_that_counts_the_rest() {
    const TRIES: usize = 1000;
    let env = TestEnv::new();
    let mut answered = None;
    let records = records_of(REFUSAL_TARGET, || {
        let (_, all_closed) = daemon(
            &engine(Product::Collie),
            &env,
            the_pick(),
            |dialer| async move {
                let mut closed = 0;
                for _ in 0..TRIES {
                    let mut client = connect(&dialer).await;
                    client
                        .send(&[hello_with("collie", "0.1", "cli", Some("a-guessed-token"))])
                        .await;
                    if client.next().await.is_none() {
                        closed += 1;
                    }
                }
                // The minute ends, and its folded refusals are recorded.
                tokio::time::sleep(Duration::from_secs(61)).await;
                closed
            },
        );
        answered = Some(all_closed);
    });

    let entries = audit_entries(&env);
    let of_kind = |kind: &str| -> Vec<&Value> {
        entries
            .iter()
            .filter(|entry| entry["kind"] == kind)
            .collect()
    };
    assert_eq!(
        (
            answered,
            refusals(&records, "token_wrong"),
            of_kind("endpoint_authentication_failed").len(),
            of_kind("endpoint_refusals_folded")
                .iter()
                .map(|entry| entry["details"].clone())
                .collect::<Vec<_>>(),
        ),
        (
            Some(TRIES),
            TRIES,
            RECORDED_PER_MINUTE,
            vec![json!({
                "listener": "daemon_socket",
                "folded": TRIES - RECORDED_PER_MINUTE,
            })],
        ),
        "{records:#?}"
    );
}

/// A `hello` with the token opens as before, and a call written behind it
/// in the same write, longer than the opening's bound, is read.
#[test]
fn a_hello_with_its_token_opens_and_a_long_call_behind_it_is_read() {
    let (_, answers) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let mut client = connect(&dialer).await;
            let mut call = super::request(1, "engine.info");
            call["params"] = json!({ "padding": "x".repeat(128 * 1024) });
            client.send(&[cli_hello(), call]).await;
            let hello = client.next().await;
            let called = client.next().await;
            [hello, called].map(|answer| answer.map(|answer| answer["id"].clone()))
        },
    );

    assert_eq!(answers, [Some(json!(0)), Some(json!(1))]);
}
