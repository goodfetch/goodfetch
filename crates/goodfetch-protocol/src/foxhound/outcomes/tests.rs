use rstest::rstest;
use serde_json::{Value, json};

use crate::catalog::OperationClass;

use super::{
    CrawlRequestOutcome, Ending, OutcomeKind, RUN_RETRY_FAILED, RunOutcomesParams, SkipReason,
};

/// An outcome as an engine sends it, ending as `ending` says.
fn outcome_ending(ending: &Value) -> Value {
    let mut outcome = json!({
        "position": 7,
        "request": { "id": "r1", "url": "https://example.com/", "label": "detail", "depth": 1 },
        "attempts": 1,
        "errors": [],
        "ended_at": "2026-11-20T10:00:00Z",
    });
    let fields = outcome.as_object_mut().expect("an outcome is an object");
    for (name, value) in ending.as_object().expect("an ending is an object") {
        fields.insert(name.clone(), value.clone());
    }
    outcome
}

/// `sent` read as an outcome, and what that writes back.
fn read_outcome(sent: &Value) -> (CrawlRequestOutcome, Value) {
    let read: CrawlRequestOutcome = serde_json::from_value(sent.clone()).expect("it reads");
    let written = serde_json::to_value(&read).expect("it writes");
    (read, written)
}

/// A re-admission is an Effect: a Client never sends it again after its
/// Connection dropped, since a resend finds nothing failed and answers 0
/// for a retry that happened (ADR 0099). The program reads
/// `run.failed_requests` instead.
///
/// Covers: story:6-306
#[test]
fn a_re_admission_is_classed_an_effect_that_no_client_sends_again() {
    assert_eq!(RUN_RETRY_FAILED.class, OperationClass::Effect);
}

#[test]
fn an_outcome_kind_this_reader_does_not_know_is_kept() {
    let kind: OutcomeKind = serde_json::from_value(json!("a_newer_kind")).expect("it reads");

    assert_eq!(
        (kind.as_str(), serde_json::to_value(&kind).unwrap()),
        ("a_newer_kind", json!("a_newer_kind"))
    );
}

/// A newer engine's skip reason keeps its outcome a known skip.
#[test]
fn a_skip_this_reader_does_not_know_the_reason_of_is_kept() {
    let sent = outcome_ending(&json!({ "kind": "skipped", "reason": "a_newer_reason" }));

    let (outcome, written) = read_outcome(&sent);

    let Ending::Skipped { reason, .. } = &outcome.ending else {
        panic!("expected a skip, got {:?}", outcome.ending);
    };
    assert_eq!(
        (matches!(reason, SkipReason::Unknown(_)), written),
        (true, sent)
    );
}

/// A newer engine's ending keeps its tag and every field it carries, and
/// its kind is that tag.
#[test]
fn an_ending_this_reader_does_not_know_keeps_its_tag_and_fields() {
    let sent = outcome_ending(&json!({ "kind": "a_newer_ending", "why": { "code": 3 } }));

    let (outcome, written) = read_outcome(&sent);

    let Ending::Unknown(member) = &outcome.ending else {
        panic!("expected the unknown ending, got {:?}", outcome.ending);
    };
    assert_eq!(
        (member.tag(), outcome.ending.kind().as_str(), written),
        ("a_newer_ending", "a_newer_ending", sent)
    );
}

#[test]
fn a_known_ending_never_reads_as_unknown() {
    let sent = outcome_ending(&json!({ "kind": "blocked", "cause": "captcha" }));

    let (outcome, _) = read_outcome(&sent);

    assert_eq!(outcome.ending.kind(), OutcomeKind::Blocked);
}

#[rstest]
#[case::failed_without_its_error(json!({ "kind": "failed" }))]
#[case::skipped_without_a_reason(json!({ "kind": "skipped" }))]
#[case::succeeded_without_records(json!({ "kind": "succeeded" }))]
fn a_known_ending_whose_fields_do_not_read_is_refused(#[case] ending: Value) {
    assert!(serde_json::from_value::<CrawlRequestOutcome>(outcome_ending(&ending)).is_err());
}

/// The endings a reader knows are the members the schema lists, so a known
/// ending whose fields do not read is never kept as unknown.
#[test]
fn the_endings_a_reader_knows_are_the_schemas() {
    let schema = schemars::schema_for!(Ending);
    let mut listed: Vec<&str> = schema.as_value()["oneOf"]
        .as_array()
        .expect("a tagged union lists its members")
        .iter()
        .filter_map(|member| member["properties"]["kind"]["const"].as_str())
        .collect();
    let mut known = Ending::KNOWN_TAGS.to_vec();
    listed.sort_unstable();
    known.sort_unstable();

    assert_eq!(known, listed);
}

/// A filter is a request, which ADR 0079's keeping of a newer engine's
/// values does not cover: an engine refuses a kind it does not know
/// instead of answering an empty page.
#[test]
fn an_outcome_kind_filter_this_reader_does_not_know_is_refused() {
    let filter = |kind| {
        serde_json::from_value::<RunOutcomesParams>(json!({ "run_id": "run-1", "kinds": [kind] }))
    };

    assert!(filter("a_newer_kind").is_err());
    assert_eq!(
        filter("failed").expect("a known kind filters").kinds,
        Some(vec![OutcomeKind::Failed])
    );
}
