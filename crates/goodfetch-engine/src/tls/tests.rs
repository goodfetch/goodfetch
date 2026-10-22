use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use rstest::rstest;
use rustls::CertificateError;

use super::{InvalidDomain, Trust, failure_of};

/// The translation point's table (ADR 0127): each refusal of rustls's
/// verifier the wire names, and one it does not, as its name in text.
#[rstest]
#[case::expired(CertificateError::Expired, "expired")]
#[case::not_yet_valid(CertificateError::NotValidYet, "not_yet_valid")]
#[case::unknown_issuer(CertificateError::UnknownIssuer, "unknown_issuer")]
#[case::name_mismatch(CertificateError::NotValidForName, "name_mismatch")]
#[case::revoked(CertificateError::Revoked, "revoked")]
#[case::unrecognized(
    CertificateError::UnhandledCriticalExtension,
    "unhandled_critical_extension"
)]
fn each_refusal_of_the_verifier_has_its_wire_name(
    #[case] refusal: CertificateError,
    #[case] wire: &str,
) {
    let failure = failure_of(&refusal);

    assert_eq!(
        serde_json::to_value(&failure).unwrap(),
        serde_json::json!(wire)
    );
}

#[test]
fn an_unrecognized_refusal_is_no_known_failure() {
    let failure = failure_of(&CertificateError::BadSignature);

    assert_eq!(
        failure,
        CertificateFailure::Unrecognized("bad_signature".to_owned())
    );
    assert!(!matches!(
        failure,
        CertificateFailure::Known(KnownCertificateFailure::UnknownIssuer)
    ));
}

#[rstest]
#[case::empty("")]
#[case::a_space("intranet example")]
#[case::a_path("intranet.example/ca")]
fn a_trusted_cas_entry_that_names_no_host_is_refused(#[case] domain: &str) {
    let refused = Trust::default_roots().with_trusted_cas(domain, Vec::new());

    assert_eq!(
        refused.map(|_| ()),
        Err(InvalidDomain {
            domain: domain.to_owned()
        })
    );
}
