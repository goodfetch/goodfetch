//! A decoy's ending carries the protocol's `blocked` details: the status
//! of a page that was fetched, and an explicit `null` for one never
//! fetched, never a made-up 0 (ADR 0035).

use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::page::BlockedDetails;

use super::{noindexed, reached_through_trap};
use crate::delivery::attempt::Answer;

/// The `blocked` details of `answer`, a non-retryable failure.
fn blocked(answer: Answer) -> BlockedDetails {
    let Answer::Failed(failure) = answer else {
        panic!("a decoy is a failure");
    };
    let failure = *failure;
    let (code, retryable, details) = (failure.code, failure.retryable, failure.details);
    assert_eq!((code.as_str(), retryable), ("blocked", false));
    serde_json::from_value(serde_json::Value::Object(details))
        .expect("a decoy's details are `blocked` details")
}

#[test]
fn a_page_reached_through_a_trap_is_a_decoy_never_fetched_so_with_no_status() {
    let details = blocked(reached_through_trap("https://shop.test/p/1"));

    assert_eq!(
        details,
        BlockedDetails {
            cause: BlockCause::Decoy,
            url: "https://shop.test/p/1".to_owned(),
            status: None,
            retry_after: None,
        }
    );
}

#[test]
fn a_noindex_page_that_failed_its_page_check_is_a_decoy_with_its_status() {
    let details = blocked(noindexed("https://shop.test/maze", 200));

    assert_eq!(
        details,
        BlockedDetails {
            cause: BlockCause::Decoy,
            url: "https://shop.test/maze".to_owned(),
            status: Some(200),
            retry_after: None,
        }
    );
}
