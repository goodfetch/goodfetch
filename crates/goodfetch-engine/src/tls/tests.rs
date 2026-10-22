use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use rstest::rstest;
use rustls::CertificateError;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use super::{
    CertificateFileError, InvalidDomain, KeyFileError, Trust, failure_of, read_certificates,
    read_private_key,
};

/// `der` as a PEM block labelled `label`.
fn pem(label: &str, der: &[u8]) -> String {
    let encoded = data_encoding::BASE64.encode(der);
    let lines: Vec<&str> = encoded
        .as_bytes()
        .chunks(64)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect();
    format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        lines.join("\n")
    )
}

/// A certificate authority made for the test, and its key.
fn authority(name: &str) -> (rcgen::Certificate, rcgen::KeyPair) {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, name);
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let certificate = params.self_signed(&key).unwrap();
    (certificate, key)
}

#[test]
fn a_pem_file_of_two_authorities_reads_as_both_in_order() {
    let (first, _) = authority("first");
    let (second, _) = authority("second");
    let file = format!(
        "{}\n{}",
        pem("CERTIFICATE", first.der()),
        pem("CERTIFICATE", second.der())
    );

    let read = read_certificates(file.as_bytes());

    assert_eq!(read, Ok(vec![first.der().clone(), second.der().clone()]));
}

#[test]
fn a_der_file_reads_as_its_one_certificate() {
    let (only, _) = authority("only");

    let read = read_certificates(only.der());

    assert_eq!(read, Ok(vec![only.der().clone()]));
}

#[rstest]
#[case::empty(b"".as_slice(), CertificateFileError::NoCertificate)]
#[case::text(b"intranet authority".as_slice(), CertificateFileError::NoCertificate)]
#[case::a_block_that_is_no_certificate(
    b"-----BEGIN CERTIFICATE-----\naW50cmFuZXQ=\n-----END CERTIFICATE-----\n".as_slice(),
    CertificateFileError::Malformed
)]
fn a_file_without_a_readable_certificate_is_refused(
    #[case] file: &[u8],
    #[case] refused: CertificateFileError,
) {
    assert_eq!(read_certificates(file), Err(refused));
}

#[test]
fn a_pkcs8_key_reads_from_pem_and_from_der() {
    let (_, key) = authority("keyed");

    let read = (
        read_private_key(pem("PRIVATE KEY", &key.serialize_der()).as_bytes()),
        read_private_key(&key.serialize_der()),
    );

    let expected = PrivateKeyDer::try_from(key.serialize_der()).unwrap();
    assert_eq!(read, (Ok(expected.clone_key()), Ok(expected)));
}

#[rstest]
#[case::empty(b"".as_slice(), KeyFileError::NoKey)]
#[case::a_certificate_alone(b"-----BEGIN CERTIFICATE-----\naW50cmFuZXQ=\n-----END CERTIFICATE-----\n".as_slice(), KeyFileError::NoKey)]
#[case::a_block_that_is_no_key(
    b"-----BEGIN PRIVATE KEY-----\naW50cmFuZXQ=\n-----END PRIVATE KEY-----\n".as_slice(),
    KeyFileError::Malformed
)]
fn a_file_without_a_readable_key_is_refused(#[case] file: &[u8], #[case] refused: KeyFileError) {
    assert_eq!(read_private_key(file).map(|_| ()), Err(refused));
}

/// Each certificate a file reads as reads back as itself from its own
/// bytes.
#[test]
fn a_certificate_read_reads_back_from_its_der() {
    let (only, _) = authority("again");
    let read = read_certificates(pem("CERTIFICATE", only.der()).as_bytes()).unwrap();

    let again: Vec<CertificateDer<'static>> = read
        .iter()
        .flat_map(|certificate| read_certificates(certificate).unwrap())
        .collect();

    assert_eq!(again, read);
}

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
