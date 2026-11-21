use rstest::rstest;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::block::BlockCause;
use crate::run::{StopReason, Verdict};

/// `sent` read as `T`, and what that writes back.
fn round_trip<T: Serialize + DeserializeOwned>(sent: &Value) -> (T, Value) {
    let read: T = serde_json::from_value(sent.clone()).expect("an unknown value reads");
    let written = serde_json::to_value(&read).expect("it writes");
    (read, written)
}

#[test]
fn an_unknown_verdict_is_kept_and_is_never_verified() {
    let (verdict, written) = round_trip::<Verdict>(&json!("a_newer_verdict"));

    assert_eq!(
        (verdict.as_str(), verdict == Verdict::Verified, written),
        ("a_newer_verdict", false, json!("a_newer_verdict"))
    );
}

#[test]
fn an_unknown_block_cause_is_kept() {
    let (cause, written) = round_trip::<BlockCause>(&json!("a_newer_cause"));

    assert_eq!(
        (cause.as_str(), written),
        ("a_newer_cause", json!("a_newer_cause"))
    );
}

#[test]
fn a_known_value_never_reads_as_unknown() {
    let verdict: Verdict = serde_json::from_value(json!("verified")).expect("it reads");
    let cause: BlockCause = serde_json::from_value(json!("captcha")).expect("it reads");

    assert_eq!((verdict, cause), (Verdict::Verified, BlockCause::Captcha));
}

#[test]
fn an_unknown_stop_reason_keeps_its_tag_and_fields() {
    let sent = json!({ "kind": "a_newer_reason", "by_whom": "x" });

    let (reason, written) = round_trip::<StopReason>(&sent);

    let StopReason::Unknown(member) = &reason else {
        panic!("expected the unknown member, got {reason:?}");
    };
    assert_eq!((member.tag(), written), ("a_newer_reason", sent));
}

#[test]
fn a_known_stop_reason_never_reads_as_unknown() {
    let reason: StopReason = serde_json::from_value(json!({ "kind": "pause" })).expect("it reads");

    assert_eq!(reason, StopReason::Pause);
}

#[test]
fn a_member_without_a_tag_is_refused() {
    assert!(serde_json::from_value::<StopReason>(json!({ "by_whom": "x" })).is_err());
}

#[rstest]
#[case::abort_without_who(json!({ "kind": "abort" }))]
#[case::resources_with_a_newer_resource(json!({ "kind": "resources", "resource": "gpu" }))]
#[case::limit_without_its_limit(json!({ "kind": "limit" }))]
fn a_known_stop_reason_whose_fields_do_not_read_is_refused(#[case] sent: Value) {
    assert!(serde_json::from_value::<StopReason>(sent).is_err());
}
