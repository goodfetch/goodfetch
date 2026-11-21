//! Covers: story:11-34, error:not_verified
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::page::ai_work::{Claim, NotVerifiedDetails, Reported};
use goodfetch_protocol::run::{AxisReason, StopReason, Verdict};
use serde::Deserialize;
use serde_json::json;

use super::{AiWorkResult, Ended, PartialOutput};
use crate::ErrorCode;

#[derive(Debug, PartialEq, Deserialize)]
struct Author {
    name: String,
    born: u16,
}

fn reason(code: &str, message: &str) -> AxisReason {
    AxisReason {
        code: code.to_owned(),
        message: message.to_owned(),
        count: None,
        call_id: None,
    }
}

fn finished(verdict: Verdict, reasons: Vec<AxisReason>) -> AiWorkResult<Author> {
    AiWorkResult {
        ended: Ended::Finished {
            output: Author {
                name: "Ada".to_owned(),
                born: 1815,
            },
        },
        verdict,
        verdict_reasons: reasons,
        reported: Some(Reported {
            claim: Claim::Done,
            message: Some("found her".to_owned()),
        }),
    }
}

#[test]
fn a_verified_finished_result_hands_over_its_output() {
    let output = finished(Verdict::Verified, Vec::new())
        .require_verified()
        .expect("a verified result hands over its output");

    assert_eq!(
        output,
        Author {
            name: "Ada".to_owned(),
            born: 1815,
        }
    );
}

#[test]
fn a_finished_result_its_checks_did_not_prove_raises_not_verified_with_its_details() {
    let reasons = vec![reason("success_check_failed", "the heading was not shown")];

    let error = finished(Verdict::Rejected, reasons.clone())
        .require_verified()
        .expect_err("a rejected result raises");

    assert_eq!(error.code(), ErrorCode::NOT_VERIFIED);
    assert_eq!(error.group(), ErrorGroup::Page);
    assert!(!error.retryable());
    let details: NotVerifiedDetails = error.details().expect("its details read by type");
    assert_eq!(
        details,
        NotVerifiedDetails {
            verdict: Verdict::Rejected,
            reasons,
            reported: Some(Reported {
                claim: Claim::Done,
                message: Some("found her".to_owned()),
            }),
        }
    );
    assert!(error.fix().is_some(), "{error}");
}

#[test]
fn a_stopped_result_raises_not_verified_even_when_its_verdict_reads_verified() {
    let result: AiWorkResult<Author> = AiWorkResult {
        ended: Ended::Stopped {
            reason: StopReason::Timeout,
            partial: PartialOutput::new(json!({ "name": "Ada" })),
        },
        verdict: Verdict::Verified,
        verdict_reasons: Vec::new(),
        reported: None,
    };

    let error = result
        .require_verified()
        .expect_err("no output to hand over");

    let details: NotVerifiedDetails = error.details().expect("its details read by type");
    assert_eq!(details.verdict, Verdict::Verified);
    assert_eq!(details.reported, None);
}

#[test]
fn an_unknown_verdict_is_never_verified() {
    let verdict: Verdict = serde_json::from_value(json!("proven")).expect("an open Verdict");

    let error = finished(verdict.clone(), Vec::new())
        .require_verified()
        .expect_err("an unknown Verdict raises");

    let details: NotVerifiedDetails = error.details().expect("its details read by type");
    assert_eq!(details.verdict, verdict);
}

#[test]
fn how_it_ended_its_verdict_and_the_models_claim_are_read_apart() {
    let result: AiWorkResult<Author> = AiWorkResult {
        ended: Ended::Stopped {
            reason: StopReason::Timeout,
            partial: PartialOutput::new(json!({ "name": "Ada" })),
        },
        verdict: Verdict::NeedsReview,
        verdict_reasons: vec![reason("stopped", "the task stopped before it finished")],
        reported: Some(Reported {
            claim: Claim::Done,
            message: None,
        }),
    };

    // An exhaustive match, with no wildcard arm: a third way to end is a
    // new meaning, never a new value (ADR 0117).
    let partial_name = match &result.ended {
        Ended::Finished { output } => Some(output.name.clone()),
        Ended::Stopped { reason, partial } => {
            assert_eq!(reason, &StopReason::Timeout);
            partial.field::<String>("name").expect("a valid field")
        }
    };

    assert_eq!(partial_name.as_deref(), Some("Ada"));
    assert_eq!(result.verdict, Verdict::NeedsReview);
    assert_eq!(
        result.reported.map(|reported| reported.claim),
        Some(Claim::Done)
    );
}

#[test]
fn a_partial_output_reads_each_field_or_each_element_by_type() {
    let object = PartialOutput::new(json!({ "name": "Ada" }));
    let list = PartialOutput::new(json!([{ "name": "Ada", "born": 1815 }]));

    assert_eq!(
        object.field::<u16>("born").expect("absent is no error"),
        None
    );
    assert!(object.field::<u16>("name").is_err(), "a text is no number");
    assert_eq!(
        list.items::<Author>().expect("each element an Author"),
        vec![Author {
            name: "Ada".to_owned(),
            born: 1815,
        }]
    );
    assert_eq!(
        object.items::<Author>().expect("an object has no elements"),
        Vec::new()
    );
    assert_eq!(object.as_json(), &json!({ "name": "Ada" }));
}
