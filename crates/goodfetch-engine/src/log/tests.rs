//! The log module's interface (spec #213's Seam 4): the origin a span
//! records of a URL, property-tested with URLs hard to redact (ADR 0132), a
//! slow Connection's log events dropped and counted, and the timer that
//! reads the `Clock` (ADR 0059).
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::time::{Duration, SystemTime};

use goodfetch_protocol::jsonrpc::Notification;
use goodfetch_protocol::log::{LogEvent, LogLevel};
use proptest::prelude::*;
use rstest::rstest;
use tokio::sync::mpsc;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{ClockTime, LogEvents, origin};
use crate::clock::Clock;

/// Covers: story:3-105
#[rstest]
#[case::user_and_password("https://alice:hunter2@example.test/", "https://example.test")]
#[case::path_query_and_fragment(
    "https://example.test:8443/account/42?token=hunter2#frag",
    "https://example.test:8443"
)]
#[case::default_port("http://example.test:80/a", "http://example.test")]
#[case::encoded_credentials("https://%61lice:%68unter2@example.test/x", "https://example.test")]
#[case::international_host("https://bücher.example/kaufen?q=1", "https://xn--bcher-kva.example")]
#[case::ipv6_host("http://[::1]:9000/p?q", "http://[::1]:9000")]
#[case::no_host("data:text/plain,hunter2", "null")]
#[case::file("file:///home/alice/hunter2.txt", "null")]
#[case::not_a_url("hunter2", "null")]
fn a_url_is_recorded_as_its_origin(#[case] url: &str, #[case] recorded: &str) {
    assert_eq!(origin(url), recorded);
}

proptest! {
    /// Whatever a URL's user, password, path, query and fragment hold, its
    /// origin holds none of them.
    ///
    /// Covers: story:3-105
    #[test]
    fn a_urls_origin_holds_nothing_of_its_user_password_path_query_or_fragment(
        user in "[a-zA-Z0-9%!$&'()*+,;=._~-]{1,12}",
        password in "[a-zA-Z0-9%!$&'()*+,;=._~:-]{1,12}",
        path in "(/[a-zA-Z0-9%._~@:-]{0,8}){0,3}",
        query in "[a-zA-Z0-9%=&._~/?:@-]{0,16}",
        fragment in "[a-zA-Z0-9%=&._~/?:@#-]{0,16}",
    ) {
        let secret = format!("GFCANARY{user}{password}");
        let url = format!(
            "https://{user}:{password}{secret}@example.test{path}/{secret}?{query}{secret}#{fragment}{secret}"
        );

        let recorded = origin(&url);

        prop_assert_eq!(recorded, "https://example.test");
    }
}

/// The `$/log` events a Connection's outgoing lines carry.
fn log_events(lines: &mut mpsc::Receiver<crate::wire::Line>) -> Vec<LogEvent> {
    let mut events = Vec::new();
    while let Ok(line) = lines.try_recv() {
        let notification: Notification = serde_json::from_slice(&line.bytes).unwrap();
        assert_eq!(notification.method, "$/log");
        events.push(serde_json::from_value(notification.params.unwrap()).unwrap());
    }
    events
}

/// Covers: story:3-105
#[test]
fn a_connection_too_slow_to_read_its_log_events_receives_the_count_it_lost() {
    let events = LogEvents::new();
    let subscriber = tracing_subscriber::registry().with(events.layer());
    let (outgoing, mut lines) = mpsc::channel(4096);

    let received = tracing::subscriber::with_default(subscriber, || {
        let log = events.connection();
        log.set(LogLevel::Info);
        let forwarding = log.forward(outgoing);
        for count in 0..3000_u32 {
            tracing::info!(count, "a state change");
        }
        drop(log);
        goodfetch_test_support::runtime().block_on(forwarding);
        log_events(&mut lines)
    });

    let counted: u64 = received.iter().map(|event| event.dropped + 1).sum();
    assert_eq!(
        (
            counted,
            received.first().map(|event| event.dropped),
            received.last().map(|event| event.fields["count"].clone()),
        ),
        (3000, Some(1976), Some(serde_json::json!(2999)))
    );
}

/// Covers: story:3-105
#[test]
fn a_connection_receives_only_the_log_events_at_its_level_and_above() {
    let events = LogEvents::new();
    let subscriber = tracing_subscriber::registry().with(events.layer());
    let (outgoing, mut lines) = mpsc::channel(16);

    let received = tracing::subscriber::with_default(subscriber, || {
        let log = events.connection();
        log.set(LogLevel::Warn);
        let forwarding = log.forward(outgoing);
        tracing::debug!("a line per request");
        tracing::info!("a state change");
        tracing::warn!(attempt = 2_u64, "a warning");
        tracing::error!("an error");
        drop(log);
        goodfetch_test_support::runtime().block_on(forwarding);
        log_events(&mut lines)
    });

    assert_eq!(
        received
            .iter()
            .map(|event| (event.level, event.message.as_str(), event.fields.clone()))
            .collect::<Vec<_>>(),
        [
            (
                LogLevel::Warn,
                "a warning",
                serde_json::json!({ "attempt": 2 })
                    .as_object()
                    .unwrap()
                    .clone()
            ),
            (LogLevel::Error, "an error", serde_json::Map::new()),
        ]
    );
}

/// The `$/log` lines a Connection receives, as sent.
fn sent_lines(lines: &mut mpsc::Receiver<crate::wire::Line>) -> Vec<String> {
    let mut sent = Vec::new();
    while let Ok(line) = lines.try_recv() {
        sent.push(String::from_utf8(line.bytes).unwrap());
    }
    sent
}

/// Each log event reaches a Connection with its target, its message
/// formatted and escaped, and each of its other fields as the JSON value of
/// its kind; a message given as another value is that value's JSON text, and
/// an event with no other field carries no `fields`.
///
/// Covers: story:3-105
#[test]
fn a_log_events_message_and_fields_reach_a_connection_as_json_values() {
    let events = LogEvents::new();
    let subscriber = tracing_subscriber::registry().with(events.layer());
    let (outgoing, mut lines) = mpsc::channel(16);

    let sent = tracing::subscriber::with_default(subscriber, || {
        let log = events.connection();
        log.set(LogLevel::Trace);
        let forwarding = log.forward(outgoing);
        tracing::info!(
            flag = true,
            delta = -3_i64,
            count = 7_u64,
            ratio = 0.5_f64,
            missing = f64::NAN,
            name = "a \"quoted\"\nname",
            shown = ?Some("x"),
            "a message with \"quotes\", a {} and a \u{7}",
            "tab\t",
        );
        tracing::warn!(message = 42_u64);
        tracing::error!("no other field");
        drop(log);
        goodfetch_test_support::runtime().block_on(forwarding);
        sent_lines(&mut lines)
    });

    let received: Vec<LogEvent> = sent
        .iter()
        .map(|line| {
            let notification: Notification = serde_json::from_str(line).unwrap();
            serde_json::from_value(notification.params.unwrap()).unwrap()
        })
        .collect();
    assert_eq!(
        (
            received,
            sent.iter()
                .map(|line| line.contains("\"fields\""))
                .collect::<Vec<_>>()
        ),
        (
            vec![
                LogEvent {
                    level: LogLevel::Info,
                    target: "goodfetch_engine::log::tests".to_owned(),
                    message: "a message with \"quotes\", a tab\t and a \u{7}".to_owned(),
                    fields: serde_json::json!({
                        "flag": true,
                        "delta": -3,
                        "count": 7,
                        "ratio": 0.5,
                        "missing": null,
                        "name": "a \"quoted\"\nname",
                        "shown": "Some(\"x\")",
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                    dropped: 0,
                },
                LogEvent {
                    level: LogLevel::Warn,
                    target: "goodfetch_engine::log::tests".to_owned(),
                    message: "42".to_owned(),
                    fields: serde_json::Map::new(),
                    dropped: 0,
                },
                LogEvent {
                    level: LogLevel::Error,
                    target: "goodfetch_engine::log::tests".to_owned(),
                    message: "no other field".to_owned(),
                    fields: serde_json::Map::new(),
                    dropped: 0,
                },
            ],
            vec![true, false, false]
        )
    );
}

/// No Connection that asks for none receives a log event, so the layer
/// listens to nothing.
#[test]
fn a_connection_whose_level_is_off_receives_no_log_event() {
    let events = LogEvents::new();
    let subscriber = tracing_subscriber::registry().with(events.layer());
    let (outgoing, mut lines) = mpsc::channel(16);

    let received = tracing::subscriber::with_default(subscriber, || {
        let log = events.connection();
        let forwarding = log.forward(outgoing);
        tracing::error!("an error");
        drop(log);
        goodfetch_test_support::runtime().block_on(forwarding);
        log_events(&mut lines)
    });

    assert_eq!(received, []);
}

#[test]
fn the_timer_writes_the_clocks_wall_time() {
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);

    let written = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(start);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let mut written = String::new();
        ClockTime(clock)
            .format_time(&mut Writer::new(&mut written))
            .unwrap();
        written
    });

    assert_eq!(written, "2026-09-21T14:13:21.5Z");
}
