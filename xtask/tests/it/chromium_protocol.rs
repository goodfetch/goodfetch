//! `xtask chromium protocol check` (ADR 0074; TESTING.md, The Goodfetch
//! Chromium): every CDP method, event and parameter the driver uses is in
//! the committed protocol JSON, `chromium/protocol.json`. The driver's use
//! is read from its source: the generated bindings it names and the method
//! names it sends as text. One fixture per refused case, each one edit away
//! from a clean fixture that passes. The local tooling declares no Feature.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

const PROTOCOL: &str = "chromium/protocol.json";

fn check(workspace: &Workspace) -> Outcome {
    workspace.xtask(&["chromium", "protocol", "check"])
}

/// The protocol JSON with `from` written as `to`: an entry removed, or
/// renamed where removing it would leave a trailing comma.
fn edited(from: &'static str, to: &'static str) -> impl Fn(&Workspace) {
    move |workspace| workspace.edit(PROTOCOL, from, to)
}

#[test]
fn a_clean_protocol_passes_and_test_code_counts_for_nothing() {
    let workspace = Workspace::from_fixture("chromium-protocol");

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), []);
}

#[rstest]
#[case::a_command_sent_through_the_bindings(
    (r#"{ "name": "enable" },"#, ""),
    "method-missing",
    "Page.enable"
)]
#[case::a_command_sent_by_its_name(
    (r#"{ "name": "close" }"#, ""),
    "method-missing",
    "Browser.close"
)]
#[case::an_event(
    (r#""name": "frameNavigated""#, r#""name": "frameRenamed""#),
    "event-missing",
    "Page.frameNavigated"
)]
#[case::a_parameter_the_driver_sends(
    (r#"{ "name": "frameId", "optional": true, "$ref": "FrameId" },"#, ""),
    "parameter-missing",
    "Page.navigate frameId"
)]
#[case::a_parameter_of_the_answer_the_driver_needs(
    (r#"{ "name": "loaderId", "type": "string" },"#, ""),
    "parameter-missing",
    "Page.navigate loaderId"
)]
#[case::a_parameter_of_the_event_the_driver_needs(
    (r#"{ "name": "type", "type": "string" }"#, r#"{ "name": "kind", "type": "string" }"#),
    "parameter-missing",
    "Page.frameNavigated type"
)]
fn a_refused_case_fails(
    #[case] edit: (&'static str, &'static str),
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("chromium-protocol");
    edited(edit.0, edit.1)(&workspace);

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), [finding(kind, subject)]);
}

/// A parameter the driver never sends, or an answer's optional one, may be
/// missing.
#[test]
fn a_parameter_the_driver_neither_sends_nor_needs_may_be_missing() {
    let workspace = Workspace::from_fixture("chromium-protocol");
    edited(r#""name": "referrerPolicy""#, r#""name": "otherPolicy""#)(&workspace);
    edited(r#""name": "errorText""#, r#""name": "otherText""#)(&workspace);

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
}

/// Until the gate commits the build's protocol JSON there is nothing to
/// check against.
#[test]
fn without_a_committed_protocol_the_check_passes() {
    let workspace = Workspace::from_fixture("chromium-protocol");
    edited(r#"{ "name": "enable" },"#, "")(&workspace);
    let refused = check(&workspace).code;
    workspace.remove(PROTOCOL);

    let outcome = check(&workspace);

    assert_eq!(
        (refused, outcome.code),
        (1, 0),
        "{}{}",
        outcome.stdout,
        outcome.stderr
    );
}
