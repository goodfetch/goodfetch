use crate::catalog::OperationClass;

use super::RUN_RETRY_FAILED;

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
