#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::net_error::NetError;
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use rstest::rstest;

/// Chromium's own refusal of a certificate it verified itself, on a host
/// the relay tunnels, becomes `certificate_invalid` with the wire's name of
/// the failure (ADR 0127's translation point): one case per known failure
/// Chromium names apart, and an unknown one as its name in text.
/// Covers: error:certificate_invalid
#[rstest]
#[case::name_mismatch(
    "net::ERR_CERT_COMMON_NAME_INVALID",
    Some(known(KnownCertificateFailure::NameMismatch))
)]
#[case::unknown_issuer(
    "net::ERR_CERT_AUTHORITY_INVALID",
    Some(known(KnownCertificateFailure::UnknownIssuer))
)]
#[case::revoked("net::ERR_CERT_REVOKED", Some(known(KnownCertificateFailure::Revoked)))]
#[case::expired_or_not_yet_valid("net::ERR_CERT_DATE_INVALID", Some(text("date_invalid")))]
#[case::unknown("net::ERR_CERT_WEAK_KEY", Some(text("weak_key")))]
#[case::transparency(
    "net::ERR_CERTIFICATE_TRANSPARENCY_REQUIRED",
    Some(text("certificate_transparency_required"))
)]
#[case::no_certificate_failure("net::ERR_CONNECTION_REFUSED", None)]
fn a_certificate_chromium_refused_is_named_as_the_wire_names_it(
    #[case] error: &str,
    #[case] expected: Option<CertificateFailure>,
) {
    assert_eq!(super::certificate_failure(&NetError::of(error)), expected);
}

fn known(failure: KnownCertificateFailure) -> CertificateFailure {
    CertificateFailure::Known(failure)
}

fn text(name: &str) -> CertificateFailure {
    CertificateFailure::Unrecognized(name.to_owned())
}
