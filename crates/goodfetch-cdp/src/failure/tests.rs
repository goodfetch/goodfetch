use rstest::rstest;
use serde_json::json;

use super::{Failure, PartyText, Refusal};

fn refusal(message: &str) -> Refusal {
    match Failure::refused(
        "Page.navigate",
        &json!({ "code": -32000, "message": message }),
    ) {
        Failure::Refused { refusal, .. } => refusal,
        other => panic!("not a refusal: {other:?}"),
    }
}

#[rstest]
#[case::target_closed("Target closed", Refusal::TargetGone)]
#[case::session_gone("Session with given id not found", Refusal::TargetGone)]
#[case::no_target("No target with given id found", Refusal::TargetGone)]
#[case::detached("Not attached to an active page", Refusal::TargetGone)]
#[case::invalid_url("Cannot navigate to invalid URL", Refusal::InvalidUrl)]
#[case::context_unknown("Cannot find context with specified id", Refusal::ContextGone)]
#[case::context_destroyed("Execution context was destroyed.", Refusal::ContextGone)]
#[case::context_navigated("Inspected target navigated or closed", Refusal::ContextGone)]
#[case::unknown("Something the table has never seen", Refusal::Unrecognized)]
fn each_message_of_the_browser_becomes_its_refusal(
    #[case] message: &str,
    #[case] expected: Refusal,
) {
    assert_eq!(refusal(message), expected);
}

#[test]
fn an_error_with_no_message_is_unrecognized() {
    assert!(matches!(
        Failure::refused("Target.createTarget", &json!({})),
        Failure::Refused {
            refusal: Refusal::Unrecognized,
            code: 0,
            ..
        }
    ));
}

/// The browser's own text, which may echo a value it was given, such as a
/// cookie in an answer that did not read, reaches no message a call raises
/// from it: each failure formats it redacted (ADR 0127).
#[rstest]
#[case::refused(Failure::refused(
    "Network.getCookies",
    &json!({ "code": -32000, "message": "bad cookie session=hunter2" })
))]
#[case::unreadable(Failure::Unreadable {
    method: "Network.getCookies",
    message: PartyText::new(r#"invalid type: string "session=hunter2""#),
})]
fn a_failure_formats_the_browsers_own_text_redacted(#[case] failure: Failure) {
    let formatted = format!("{failure} {failure:?}");

    assert!(!formatted.contains("hunter2"), "{formatted}");
}
