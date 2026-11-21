//! The copy each other reader of the Subscription gets of the failure that
//! ended it (ADR 0078): the same error as the last reader's, so a close's
//! own failure stays the runtime's, never one the engine answered.
//! Covers: story:11-19

use goodfetch_protocol::connection::{CallCancelledDetails, CancelledBy, ClosedValue};
use goodfetch_protocol::error_code::ErrorCode;

use super::copied;
use crate::error::Error;

#[test]
fn a_copy_of_the_closes_cancellation_is_the_runtimes_as_the_original() {
    let copy = copied(&Error::cancelled_by_close());

    assert_eq!(
        (copy.code(), copy.is_answer(), copy.is_ended_by_close()),
        (ErrorCode::CALL_CANCELLED, false, true)
    );
    assert_eq!(
        copy.details::<CallCancelledDetails>(),
        Some(CallCancelledDetails {
            by: CancelledBy::Program,
            closed: Some(ClosedValue::Connection),
            person: None,
        })
    );
}
