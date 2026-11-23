//! AI work's result through the Collie crate alone (ADR 0079; issue 3162):
//! a Rust program names [`AiWorkResult`], how it [`Ended`], its Verdict and
//! the model's claim, and reads `not_verified`'s [`NotVerifiedDetails`],
//! with no `use` of the client crate or of the protocol crate.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_collie::{
    AiWorkResult, AxisReason, Claim, Ended, ErrorCode, NotVerifiedDetails, PartialOutput, Reported,
    StopReason, Verdict,
};
use goodfetch_test_support::TestEnv;
use serde_json::json;

/// Covers: story:11-34, error:not_verified
#[test]
fn a_program_reads_ai_works_result_and_its_not_verified_details_through_collie_alone() {
    let _env = TestEnv::new();
    let reasons: Vec<AxisReason> = serde_json::from_value(json!([
        { "code": "stopped", "message": "the task stopped before it finished" }
    ]))
    .expect("an axis reason reads from its JSON");
    let result: AiWorkResult<String> = AiWorkResult {
        ended: Ended::Stopped {
            reason: StopReason::Timeout,
            partial: PartialOutput::new(json!({ "name": "Ada" })),
        },
        verdict: Verdict::NeedsReview,
        verdict_reasons: reasons.clone(),
        reported: Some(Reported {
            claim: Claim::Done,
            message: None,
        }),
    };

    let error = result
        .require_verified()
        .expect_err("a stopped result hands over no output");

    assert_eq!(error.code(), ErrorCode::NOT_VERIFIED);
    let details: NotVerifiedDetails = error.details().expect("its details read by type");
    assert_eq!(
        details,
        NotVerifiedDetails {
            verdict: Verdict::NeedsReview,
            reasons,
            reported: Some(Reported {
                claim: Claim::Done,
                message: None,
            }),
        }
    );
}
