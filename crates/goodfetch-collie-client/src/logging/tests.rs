//! The level the engine is told and the targets its events are re-emitted
//! under (ADR 0078).

use goodfetch_protocol::log::LogLevel;
use goodfetch_test_support::capturing;
use rstest::rstest;
use tracing::Level;

use super::{ENGINE, engine_log_level, engine_target, relay};

#[rstest]
#[case::the_engines_own("goodfetch::rpc::dispatch", "goodfetch::engine::rpc::dispatch")]
#[case::one_segment("goodfetch::page", "goodfetch::engine::page")]
#[case::outside_goodfetch("hyper::client", "goodfetch::engine::hyper::client")]
#[case::goodfetch_alone("goodfetch", "goodfetch::engine")]
#[case::empty("", "goodfetch::engine")]
#[case::not_an_identifier("goodfetch::page cdp", "goodfetch::engine")]
#[case::an_empty_segment("goodfetch::::page", "goodfetch::engine")]
#[case::a_leading_digit("goodfetch::2fa", "goodfetch::engine")]
fn an_engine_target_is_re_emitted_under_goodfetch_engine(
    #[case] target: &str,
    #[case] expected: &str,
) {
    assert_eq!(engine_target(target), expected);
}

#[test]
fn with_no_subscriber_the_engine_is_told_off() {
    assert_eq!(engine_log_level(), LogLevel::Off);
}

#[rstest]
#[case::trace(&[(ENGINE, Level::TRACE)], LogLevel::Trace)]
#[case::debug(&[("goodfetch", Level::DEBUG)], LogLevel::Debug)]
#[case::info(&[(ENGINE, Level::INFO)], LogLevel::Info)]
#[case::warn(&[(ENGINE, Level::WARN)], LogLevel::Warn)]
#[case::error(&[(ENGINE, Level::ERROR)], LogLevel::Error)]
#[case::an_area_of_the_engine_only(&[("goodfetch::engine::page", Level::TRACE)], LogLevel::Off)]
#[case::the_client_only(&[("goodfetch::client", Level::TRACE)], LogLevel::Off)]
fn the_engine_is_told_the_highest_level_recorded_for_goodfetch_engine(
    #[case] levels: &[(&str, Level)],
    #[case] expected: LogLevel,
) {
    let (subscriber, _log) = capturing(levels);

    let told = tracing::subscriber::with_default(subscriber, engine_log_level);

    assert_eq!(told, expected);
}

/// A subscriber that records less than another live one is read for
/// itself: a callsite's kept interest, which the first settled, decides
/// nothing.
#[test]
fn the_level_is_read_from_the_current_subscriber_while_another_lives() {
    let (at_debug, _) = capturing(&[(ENGINE, Level::DEBUG)]);
    let (at_warn, _) = capturing(&[(ENGINE, Level::WARN)]);

    let told = tracing::subscriber::with_default(at_debug, || {
        let first = engine_log_level();
        let then = tracing::subscriber::with_default(at_warn, engine_log_level);
        (first, then, engine_log_level())
    });

    assert_eq!(told, (LogLevel::Debug, LogLevel::Warn, LogLevel::Debug));
}

/// An engine event the program's subscriber does not record at its level
/// and target is not re-emitted, whatever another live subscriber records.
#[test]
fn an_engine_event_the_subscriber_does_not_record_is_dropped() {
    let (at_trace, _) = capturing(&[(ENGINE, Level::TRACE)]);
    let (at_warn, log) = capturing(&[(ENGINE, Level::WARN)]);
    let event = |level: &str, message: &str| {
        Some(serde_json::json!({
            "level": level, "target": "goodfetch::page", "message": message,
            "fields": {}, "dropped": 0
        }))
    };

    tracing::subscriber::with_default(at_trace, || {
        relay(event("debug", "the outer subscriber's"));
        tracing::subscriber::with_default(at_warn, || {
            relay(event("debug", "below the level"));
            relay(event("warn", "at the level"));
        });
    });

    let written = String::from_utf8(log.written()).unwrap();
    assert!(written.contains("at the level"), "{written}");
    assert!(!written.contains("below the level"), "{written}");
}
