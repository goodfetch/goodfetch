//! B3's message scenario, which `examples/counts.rs` and `xtask counts` read:
//! the work it counts happens, a plant adds one unit per call and the
//! Connection's log level switched off takes its encodes away (ADR 0140).

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;

use crate::message_scenario::{self as scenario, Logs, Plant, Sent, Traffic};

const TRAFFIC: Traffic = Traffic {
    small: 3,
    large: 2,
    plugin: 2,
};

/// What one run counted and what left the engine.
fn counted(logs: Logs, planted: Option<Row>) -> (CountedWork, Sent) {
    let home = tempfile::tempdir().unwrap();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let plant = planted.map(|row| {
        let work = work.clone();
        Arc::new(move |_: &serde_json::Value| work.count(row)) as Plant
    });
    let sent = scenario::run(TRAFFIC, logs, metrics, plant, home.path());
    (work, sent)
}

/// What one run counted: its deep copies, its encodes and what left the
/// engine.
fn run(logs: Logs, planted: Option<Row>) -> (u64, u64, Sent) {
    let (work, sent) = counted(logs, planted);
    (
        work.read(Row::MessageDeepCopies),
        work.read(Row::MessageEncodes),
        sent,
    )
}

/// Each call counts its copy on the way in and its two encodes on the way
/// out, a Plugin's reply and event included; a plant adds one for each call
/// that went through a handler.
#[test]
fn the_messages_count_their_copies_and_encodes_a_plant_adds_one_a_call_and_log_off_removes_its_own()
{
    let calls = u64::try_from(1 + TRAFFIC.small + TRAFFIC.large + TRAFFIC.plugin).unwrap();

    let (copies, encodes, sent) = run(Logs::On, None);
    let (copies_planted, _, _) = run(Logs::On, Some(Row::MessageDeepCopies));
    let (_, encodes_planted, _) = run(Logs::On, Some(Row::MessageEncodes));
    let (copies_off, encodes_off, sent_off) = run(Logs::Off, None);

    assert!(sent.event_batches > 0, "{sent:?}");
    assert_eq!(sent.handler_requests, TRAFFIC.plugin, "{sent:?}");
    assert!(sent.log_lines > 0, "{sent:?}");
    assert_eq!(sent_off.log_lines, 0, "{sent_off:?}");

    assert!(copies >= u64::try_from(TRAFFIC.incoming()).unwrap());
    assert!(encodes >= 2 * u64::try_from(sent.outgoing - sent.log_lines).unwrap());
    assert_eq!(copies_planted - copies, calls);
    assert_eq!(encodes_planted - encodes, calls);
    assert_eq!(copies_off, copies);
    assert!(encodes_off < encodes, "{encodes_off} off, {encodes} on");
}

/// Each request counts its decodes, `hello`'s and `events.subscribe`'s
/// included, and no other message counts one: a Plugin's replies and
/// `$/credit` are no requests. A plant adds one for each call that went
/// through a handler.
#[test]
fn each_request_counts_its_decodes_and_a_plant_adds_one_a_call() {
    let requests = u64::try_from(TRAFFIC.requests()).unwrap();
    let calls = u64::try_from(1 + TRAFFIC.small + TRAFFIC.large + TRAFFIC.plugin).unwrap();
    let decodes = |planted| counted(Logs::On, planted).0.read(Row::MessageDecodes);

    let (unplanted, planted) = (decodes(None), decodes(Some(Row::MessageDecodes)));

    assert!(unplanted >= requests, "{unplanted} for {requests} requests");
    assert_eq!(planted - unplanted, calls);
}
