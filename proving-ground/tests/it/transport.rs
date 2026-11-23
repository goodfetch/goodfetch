//! Pages over HTTP/1.1, HTTP/2 and TLS, the TLS from a certificate
//! authority the Proving ground makes from its seed, which a client trusts
//! explicitly and never by turning verification off.

use crate::clients::{self, Http2Settings};
use crate::proving_ground::ProvingGround;

const SEED: u64 = 159;

const HTTP2: Http2Settings = Http2Settings {
    initial_window_size: 1_048_576,
    initial_connection_window_size: 2_097_152,
    max_concurrent_streams: 77,
    header_table_size: 4096,
};

/// Covers: story:2-43
#[test]
fn the_same_seed_makes_the_same_certificate_authority() {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED);

    let authority = first.certificate_authority();

    assert!(
        authority.starts_with("-----BEGIN CERTIFICATE-----"),
        "{authority}"
    );
    assert_eq!(authority, second.certificate_authority());
}

/// Covers: story:2-43
#[test]
fn another_seed_makes_another_certificate_authority() {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED + 1);

    assert_ne!(
        first.certificate_authority(),
        second.certificate_authority()
    );
}

/// Covers: story:2-43
#[test]
fn a_client_trusting_the_certificate_authority_reads_the_same_page_over_tls() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let plain = proving_ground.get("http://catalog.test/catalog/1");

    let exchange = clients::raw_tls(
        proving_ground.address(),
        "catalog.test",
        config,
        b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n",
    )
    .expect("the certificate verifies");

    assert_eq!(exchange.status(), 200);
    assert_eq!(exchange.body(), plain.body.as_bytes());
}

/// Covers: story:2-43
#[test]
fn a_client_trusting_another_seeds_certificate_authority_is_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let other = ProvingGround::start(SEED + 1);
    let config = clients::trusting(&other.certificate_authority(), &["http/1.1"]);

    let refused = clients::raw_tls(
        proving_ground.address(),
        "catalog.test",
        config,
        b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n",
    );

    assert!(
        matches!(
            refused,
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadSignature | rustls::CertificateError::UnknownIssuer
            ))
        ),
        "{:?}",
        refused.err()
    );
}

/// Covers: story:2-43
#[test]
fn a_client_offering_h2_reads_the_same_page_over_http2() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["h2"]);
    let plain = proving_ground.get("http://catalog.test/catalog/1");

    let answers = clients::http2(
        proving_ground.address(),
        "catalog.test",
        config,
        &HTTP2,
        &["/catalog/1"],
        &[],
    );

    assert_eq!(answers[0].status, 200);
    assert_eq!(answers[0].body, plain.body.as_bytes());
}

/// The host that requires mutual TLS refuses a client that presents no
/// client certificate and lists no handshake; a client presenting the one
/// `GET /client-certificate` and `GET /client-key` hand out reads its page,
/// and the observations list that handshake.
/// Covers: story:2-43
#[test]
fn the_mutual_tls_host_serves_only_the_client_certificate_it_hands_out() {
    let proving_ground = ProvingGround::start(SEED);
    let authority = proving_ground.certificate_authority();
    let (certificate, key) = proving_ground.client_identity();
    let request = b"GET / HTTP/1.1\r\nHost: client-certificate.certificates.test\r\nConnection: close\r\n\r\n";

    let refused = clients::raw_tls(
        proving_ground.address(),
        "client-certificate.certificates.test",
        clients::trusting(&authority, &["http/1.1"]),
        request,
    );
    let after_refusal = proving_ground.observations()["client_certificates"].clone();
    let presented = clients::raw_tls(
        proving_ground.address(),
        "client-certificate.certificates.test",
        clients::presenting(&authority, &certificate, &key),
        request,
    )
    .expect("the client certificate is accepted");

    assert!(
        matches!(
            refused,
            Err(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateRequired
            ))
        ),
        "{:?}",
        refused.err()
    );
    assert_eq!(after_refusal, serde_json::json!([]));
    assert_eq!(presented.status(), 200);
    let page = String::from_utf8_lossy(presented.body());
    assert!(page.contains("Reached with a client certificate"), "{page}");
    assert_eq!(
        proving_ground.observations()["client_certificates"],
        serde_json::json!(["client-certificate.certificates.test"])
    );
}

/// The mutual-TLS host's refusal reaches a client that sent its request
/// with its certificate, before the server read either, as the alert and
/// then an orderly close. A socket closed with bytes unread resets its
/// connection instead, and on Windows a reset drops what the client had
/// received and not yet read, the alert among it (issue 3535).
/// Covers: story:2-43
#[test]
fn a_refused_handshake_ends_with_its_alert_then_an_orderly_close() {
    let proving_ground = ProvingGround::start(SEED);
    // Past what the server's TLS reads at once while it handshakes.
    let request = [b'x'; 32 * 1024];

    let ended = clients::refused_tls(
        proving_ground.address(),
        "client-certificate.certificates.test",
        clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]),
        &request,
    );

    assert_eq!(
        ended,
        (
            Ok(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateRequired
            )),
            Ok(0)
        )
    );
}
