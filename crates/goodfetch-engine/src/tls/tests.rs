use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use rstest::rstest;
use rustls::CertificateError;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};

use crate::clock::Clock;

use super::{
    CertificateFileError, InvalidDomain, KeyFileError, Trust, TrustScope, certificate_failure,
    failure_of, read_certificates, read_private_key,
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

#[test]
fn each_listed_domain_is_a_scope_with_its_own_and_its_parents_authorities() {
    let (parent, _) = authority("parent");
    let (child, _) = authority("child");
    let parent = parent.der().clone();
    let child = child.der().clone();
    let trust = Trust::default_roots()
        .with_trusted_cas("example.test", vec![parent.clone()])
        .unwrap()
        .with_trusted_cas("Sub.Example.test.", vec![child.clone()])
        .unwrap();

    assert_eq!(
        trust.scopes(),
        vec![
            TrustScope {
                domain: None,
                authorities: Vec::new(),
            },
            TrustScope {
                domain: Some("example.test".to_owned()),
                authorities: vec![parent.clone()],
            },
            TrustScope {
                domain: Some("sub.example.test".to_owned()),
                authorities: vec![child, parent],
            },
        ]
    );
}

#[rstest]
#[case::the_listed_domain("example.test", Some("example.test"))]
#[case::a_subdomain("www.example.test", Some("example.test"))]
#[case::the_most_specific_domain("a.sub.example.test", Some("sub.example.test"))]
#[case::an_unlisted_host("example.com", None)]
#[case::a_host_that_only_ends_alike("notexample.test", None)]
fn a_host_is_in_the_scope_of_the_most_specific_domain_listed_for_it(
    #[case] host: &str,
    #[case] scope: Option<&str>,
) {
    let trust = Trust::default_roots()
        .with_trusted_cas("example.test", Vec::new())
        .unwrap()
        .with_trusted_cas("sub.example.test", Vec::new())
        .unwrap();

    assert_eq!(trust.scope_of(host), scope);
}

/// The host the dated certificate is for.
const DATED_HOST: &str = "site.test";

/// A certificate of [`DATED_HOST`], valid through 2020 alone, its key and
/// the authority that signed it.
fn dated_certificate() -> (
    CertificateDer<'static>,
    CertificateDer<'static>,
    PrivateKeyDer<'static>,
) {
    let authority_key = rcgen::KeyPair::generate().unwrap();
    let mut authority = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    authority
        .distinguished_name
        .push(rcgen::DnType::CommonName, "dated authority");
    authority.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let authority_der = authority.self_signed(&authority_key).unwrap().der().clone();
    let issuer = rcgen::Issuer::new(authority, authority_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let mut leaf = rcgen::CertificateParams::new(vec![DATED_HOST.to_owned()]).unwrap();
    leaf.not_before = rcgen::date_time_ymd(2020, 1, 1);
    leaf.not_after = rcgen::date_time_ymd(2021, 1, 1);
    let leaf_der = leaf.signed_by(&key, &issuer).unwrap().der().clone();
    let key = PrivateKeyDer::try_from(key.serialize_der()).unwrap();
    (authority_der, leaf_der, key)
}

/// A moment of `year`'s first of June, at midnight UTC.
fn june_of(year: i16) -> SystemTime {
    let seconds = jiff::civil::date(year, 6, 1)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
        .as_second();
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds.unsigned_abs())
}

/// The test Clock, paused at `year`'s first of June; made on the runtime it
/// runs on.
#[expect(
    clippy::disallowed_methods,
    reason = "a test drives the engine's time through the test Clock (ADR 0059)"
)]
fn paused_in_june_of(year: i16) -> Clock {
    Clock::paused(june_of(year))
}

/// The Examiner, the decision for chains received outside rustls, judges a
/// certificate's dates at the engine's Clock, so a test's paused Clock
/// drives it (ADR 0059).
#[rstest]
#[case::within_its_dates(2020, None)]
#[case::past_its_end(
    2022,
    Some(CertificateFailure::Known(KnownCertificateFailure::Expired))
)]
fn the_examiner_judges_a_certificate_at_the_clocks_time(
    #[case] year: i16,
    #[case] failure: Option<CertificateFailure>,
) {
    let (authority, leaf, _) = dated_certificate();
    let trust = Trust::default_roots()
        .with_trusted_cas(DATED_HOST, vec![authority])
        .unwrap();

    let presented = goodfetch_test_support::runtime().block_on(async {
        let examiner = trust.examiner(paused_in_june_of(year));
        examiner.examine(DATED_HOST, &[leaf.to_vec()]).unwrap()
    });

    assert_eq!(presented.failure, failure);
}

/// The engine's own client judges a server's certificate at the engine's
/// Clock: rustls's handshake reads its time from it, not the OS.
#[rstest]
#[case::within_its_dates(2020, Ok(()))]
#[case::past_its_end(
    2022,
    Err(Some(CertificateFailure::Known(KnownCertificateFailure::Expired)))
)]
fn a_connection_judges_the_servers_certificate_at_the_clocks_time(
    #[case] year: i16,
    #[case] handshake: Result<(), Option<CertificateFailure>>,
) {
    let (authority, leaf, key) = dated_certificate();
    let trust = Trust::default_roots()
        .with_trusted_cas(DATED_HOST, vec![authority])
        .unwrap();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![leaf], key)
    .unwrap();

    let shaken = goodfetch_test_support::runtime().block_on(async {
        let client = trust.client_config(paused_in_june_of(year));
        hand_shake(client, server)
    });

    assert_eq!(
        shaken.map_err(|error| certificate_failure(&error)),
        handshake
    );
}

/// Runs a TLS handshake of `client` with `server` in memory, to its end.
fn hand_shake(
    client: rustls::ClientConfig,
    server: rustls::ServerConfig,
) -> Result<(), rustls::Error> {
    let name = ServerName::try_from(DATED_HOST).unwrap();
    let mut client = rustls::ClientConnection::new(Arc::new(client), name).unwrap();
    let mut server = rustls::ServerConnection::new(Arc::new(server)).unwrap();
    while client.is_handshaking() || server.is_handshaking() {
        let mut bytes = Vec::new();
        while client.wants_write() {
            client.write_tls(&mut bytes).unwrap();
        }
        let mut sent = bytes.as_slice();
        while !sent.is_empty() {
            server.read_tls(&mut sent).unwrap();
            server.process_new_packets()?;
        }
        let mut bytes = Vec::new();
        while server.wants_write() {
            server.write_tls(&mut bytes).unwrap();
        }
        let mut sent = bytes.as_slice();
        while !sent.is_empty() {
            client.read_tls(&mut sent).unwrap();
            client.process_new_packets()?;
        }
    }
    Ok(())
}
