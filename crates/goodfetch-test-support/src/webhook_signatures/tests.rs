use super::{VerificationOutcome, webhook_signatures};

#[test]
fn the_committed_vectors_hold_the_published_ones_and_every_outcome() {
    let vectors = webhook_signatures();

    let published: Vec<&str> = vectors
        .signed
        .iter()
        .filter(|vector| vector.source.is_some())
        .map(|vector| vector.name.as_str())
        .collect();
    let mut outcomes: Vec<VerificationOutcome> = vectors
        .verifications
        .iter()
        .map(|case| case.outcome)
        .collect();
    outcomes.sort_unstable_by_key(|outcome| format!("{outcome:?}"));
    outcomes.dedup();
    assert_eq!(
        (published, outcomes),
        (
            vec!["v1_published", "v2a_published"],
            vec![
                VerificationOutcome::MissingHeader,
                VerificationOutcome::NoMatchingSignature,
                VerificationOutcome::TimestampInvalid,
                VerificationOutcome::TimestampTooNew,
                VerificationOutcome::TimestampTooOld,
                VerificationOutcome::Verified,
            ]
        )
    );
}
