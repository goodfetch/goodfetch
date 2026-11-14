//! B3's message scenario, which `examples/counts.rs` and `xtask counts` read:
//! the work it counts happens, a plant adds one unit per call and the
//! Connection's log level switched off takes its encodes away (ADR 0140).

use std::sync::Arc;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;

use crate::message_scenario::{self as scenario, Logs, Plant, Sent, Traffic};

const TRAFFIC: Traffic = Traffic {
    small: 3,
    large: 2,
    plugin: 2,
};

/// What one run counted: its deep copies, its encodes and what left the
/// engine.
fn run(logs: Logs, planted: Option<Row>) -> (u64, u64, Sent) {
    let home = tempfile::tempdir().unwrap();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let plant = planted.map(|row| {
        let work = work.clone();
        Arc::new(move |_: goodfetch_engine::Params<'_>| work.count(row)) as Plant
    });
    let sent = scenario::run(TRAFFIC, logs, metrics, plant, home.path());
    (
        work.read(Row::MessageDeepCopies),
        work.read(Row::MessageEncodes),
        sent,
    )
}

/// Every message is read without a copy, a notification's params and a reply
/// to a handler request included, and every answer and event that leaves is
/// encoded once, a Plugin's reply, its events and its log lines included, the
/// `$/events` batches copying their events; a plant adds one for each call
/// that went through a handler.
#[test]
fn every_message_is_read_without_a_copy_and_encoded_once() {
    let calls = u64::try_from(1 + TRAFFIC.small + TRAFFIC.large + TRAFFIC.plugin).unwrap();

    let (copies, encodes, sent) = run(Logs::On, None);
    let (copies_planted, _, _) = run(Logs::On, Some(Row::MessageDeepCopies));
    let (_, encodes_planted, _) = run(Logs::On, Some(Row::MessageEncodes));
    let (copies_off, encodes_off, sent_off) = run(Logs::Off, None);

    assert!(sent.event_batches > 0, "{sent:?}");
    assert_eq!(sent.handler_requests, TRAFFIC.plugin, "{sent:?}");
    assert!(sent.log_lines > 0, "{sent:?}");
    assert_eq!(sent_off.log_lines, 0, "{sent_off:?}");

    assert_eq!(copies, 0);
    assert_eq!(
        encodes,
        u64::try_from(sent.answers_and_events()).unwrap(),
        "{sent:?}"
    );
    assert_eq!(copies_planted - copies, calls);
    assert_eq!(encodes_planted - encodes, calls);
    assert_eq!(copies_off, 0);
    assert_eq!(
        encodes_off,
        u64::try_from(sent_off.answers_and_events()).unwrap(),
        "{sent_off:?}"
    );
    assert!(encodes_off < encodes, "{encodes_off} off, {encodes} on");
}
