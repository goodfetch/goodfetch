//! Covers: story:11-91

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::page::BlockedDetails;

/// 2026-11-19T12:30:00Z.
const HALF_PAST_NOON: Duration = Duration::from_mins(29_918_190);

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Holder {
    #[serde(with = "super")]
    at: SystemTime,
    #[serde(default, with = "super::nullable")]
    until: Option<SystemTime>,
}

fn half_past_noon_and(extra: Duration) -> SystemTime {
    SystemTime::UNIX_EPOCH + HALF_PAST_NOON + extra
}

#[test]
fn a_point_in_time_crosses_the_wire_as_rfc3339_in_utc() {
    let held = Holder {
        at: half_past_noon_and(Duration::from_millis(123)),
        until: Some(half_past_noon_and(Duration::ZERO)),
    };

    assert_eq!(
        serde_json::to_value(&held).unwrap(),
        json!({"at": "2026-11-19T12:30:00.123Z", "until": "2026-11-19T12:30:00Z"})
    );
}

#[test]
fn any_offset_reads_as_the_same_instant_to_the_nanosecond() {
    let read: Holder = serde_json::from_value(json!({
        "at": "2026-11-19T14:30:00.123456789+02:00",
        "until": "2026-11-19T07:30:00-05:00",
    }))
    .unwrap();

    assert_eq!(
        read,
        Holder {
            at: half_past_noon_and(Duration::from_nanos(123_456_789)),
            until: Some(half_past_noon_and(Duration::ZERO)),
        }
    );
}

#[test]
fn a_nullable_point_in_time_reads_null_or_its_absence_as_none() {
    let at = "2026-11-19T12:30:00Z";

    let null: Holder = serde_json::from_value(json!({"at": at, "until": null})).unwrap();
    let absent: Holder = serde_json::from_value(json!({"at": at})).unwrap();

    assert_eq!((null.until, absent.until), (None, None));
}

#[test]
fn a_protocol_type_reads_an_absent_nullable_point_in_time_as_none() {
    let blocked: BlockedDetails = serde_json::from_value(json!({
        "cause": "rate_limit",
        "url": "https://a.test/",
        "status": 429,
    }))
    .unwrap();

    assert_eq!(blocked.retry_after, None);
}

#[test]
fn text_with_no_offset_is_refused() {
    for text in ["2026-11-19T12:30:00", "2026-11-19", "noon"] {
        assert!(
            serde_json::from_value::<Holder>(json!({"at": text})).is_err(),
            "{text}"
        );
    }
}
