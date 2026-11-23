//! B3's message scenario, which `examples/counts.rs` and `xtask counts` read:
//! the work it counts happens, on stdio and over an Exposed Daemon's
//! WebSocket alike, a plant adds one unit per call and the Connection's log
//! level switched off takes its encodes away (ADR 0140).

use std::sync::Arc;

use goodfetch_counted_work::Row;
use goodfetch_engine::Params;
use goodfetch_engine::metrics::Metrics;
use rstest::rstest;
use serde_json::Value;

use crate::message_scenario::{
    self as scenario, BODY_CHUNKS, Carriage, Logs, Plant, Sent, Traffic,
};

const TRAFFIC: Traffic = Traffic {
    small: 3,
    large: 2,
    plugin: 2,
    bodies: 1,
};

/// The calls that went through a handler: every request but `hello`.
fn calls() -> u64 {
    u64::try_from(TRAFFIC.requests() - 1).unwrap()
}

/// What one run counted: its deep copies, its encodes, its decodes and what
/// left the engine.
fn run(carriage: Carriage, logs: Logs, planted: Option<Row>) -> (u64, u64, u64, Sent) {
    let home = tempfile::tempdir().unwrap();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let plant = planted.map(|row| {
        let work = work.clone();
        match row {
            // One more read of the call's params, which counts itself.
            Row::MessageDecodes => {
                Arc::new(|params: Params<'_>| drop(params.read::<Value>())) as Plant
            }
            _ => Arc::new(move |_: Params<'_>| work.count(row)) as Plant,
        }
    });
    let sent = scenario::run(TRAFFIC, logs, carriage, metrics, plant, home.path());
    (
        work.read(Row::MessageDeepCopies),
        work.read(Row::MessageEncodes),
        work.read(Row::MessageDecodes),
        sent,
    )
}

/// Every message is read without a copy, a notification's params and a reply
/// to a handler request included, and every answer and event that leaves is
/// encoded once, a Plugin's reply, its events, its log lines and a body's
/// `$/chunk`s included, the `$/events` batches copying their events; a plant
/// adds one for each call that went through a handler. A WebSocket frame
/// carries its message as a line does: through the same dispatch, decoded
/// once and encoded once (ADR 0049).
#[rstest]
fn every_message_is_read_without_a_copy_and_encoded_once(
    #[values(Carriage::Stdio, Carriage::WebSocket)] carriage: Carriage,
) {
    let (copies, encodes, _, sent) = run(carriage, Logs::On, None);
    let (copies_planted, _, _, _) = run(carriage, Logs::On, Some(Row::MessageDeepCopies));
    let (_, encodes_planted, _, _) = run(carriage, Logs::On, Some(Row::MessageEncodes));
    let (copies_off, encodes_off, _, sent_off) = run(carriage, Logs::Off, None);

    assert!(sent.event_batches > 0, "{sent:?}");
    assert_eq!(sent.handler_requests, TRAFFIC.plugin, "{sent:?}");
    assert_eq!(sent.chunks, TRAFFIC.bodies * BODY_CHUNKS, "{sent:?}");
    assert!(sent.log_lines > 0, "{sent:?}");
    assert_eq!(sent_off.log_lines, 0, "{sent_off:?}");

    assert_eq!(copies, 0);
    assert_eq!(
        encodes,
        u64::try_from(sent.answers_and_events()).unwrap(),
        "{sent:?}"
    );
    assert_eq!(copies_planted - copies, calls());
    assert_eq!(encodes_planted - encodes, calls());
    assert_eq!(copies_off, 0);
    assert_eq!(
        encodes_off,
        u64::try_from(sent_off.answers_and_events()).unwrap(),
        "{sent_off:?}"
    );
    assert!(encodes_off < encodes, "{encodes_off} off, {encodes} on");
}

/// Each request's params are decoded once, where they are, straight into
/// the type the request's handler takes, `hello`'s and `page.body`'s among
/// them (ADR 0140's E1), on either carriage: dispatch takes their `_meta`
/// and the Run and Session IDs its span names from that pass. A `$/credit`
/// and a reply to a handler request are no request and count none; a plant
/// that reads each handled call's params once more adds one decode for
/// each.
#[rstest]
fn each_decode_of_a_requests_params_is_counted(
    #[values(Carriage::Stdio, Carriage::WebSocket)] carriage: Carriage,
) {
    let requests = u64::try_from(TRAFFIC.requests()).unwrap();

    let (_, _, decodes, sent) = run(carriage, Logs::On, None);
    let (_, _, decodes_planted, _) = run(carriage, Logs::On, Some(Row::MessageDecodes));

    assert_eq!(sent.handler_requests, TRAFFIC.plugin, "{sent:?}");
    assert_eq!(requests, 12);
    assert_eq!(decodes, requests);
    assert_eq!(decodes_planted - decodes, calls());
}
