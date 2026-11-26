//! What a program's own tests build through the `test-util` feature, which
//! builds test values alone, never a capability (ADRs 0087, 0099 and 0105):
//! `Error::fake` is the error the engine would answer with a code, of the
//! group and retryable flag the catalog gives it.
//!
//! Covers: story:11-1107

use goodfetch_foxhound::{Error, ErrorCode, error_code};
use goodfetch_protocol::catalog::ErrorGroup;

#[test]
fn a_faked_foxhound_error_has_its_codes_group_and_retryable_flag() {
    let error = Error::fake(
        error_code::RESPONSE_UNAVAILABLE,
        "https://shop.test/ answered 503",
    );

    assert_eq!(error.code(), error_code::RESPONSE_UNAVAILABLE);
    assert_eq!(error.group(), ErrorGroup::Page);
    assert!(error.retryable());
    assert_eq!(error.to_string(), "https://shop.test/ answered 503");
}

#[test]
fn a_faked_error_of_a_code_no_catalog_has_is_an_internal_one_not_retried() {
    let code = ErrorCode::new("brand_new").unwrap();

    let error = Error::fake(code, "a newer engine's failure");

    assert_eq!(error.code(), code);
    assert_eq!(error.group(), ErrorGroup::Internal);
    assert!(!error.retryable());
}
