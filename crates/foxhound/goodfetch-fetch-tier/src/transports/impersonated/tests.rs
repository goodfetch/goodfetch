//! `wreq`'s translation point (ADR 0127): one named case per failure it
//! knows, and the pin of the forks it reads.
//!
//! Covers: story:5-122

use rstest::rstest;

use super::classify;
use crate::driver::TransportFailure;

/// The forks [`classify`] reads, as `Cargo.lock` held them when its table
/// was written: each one's name, version and checksum.
const PINNED: &[(&str, &str, &str)] = &[
    (
        "btls",
        "0.5.6",
        "2c5e60b8c8d282c86360cab651ded04ab0335a7b5390c8d34145cbeab8cacf5f",
    ),
    (
        "http2",
        "0.5.20",
        "92d3114be2f413b2e491e686b93a28cda30c355cffc8d091a57f8be4b1342896",
    ),
    // Goodfetch's fork, carried in `vendor/wreq` (the workspace's
    // `[patch.crates-io]`), which the lock file holds with no checksum.
    ("wreq", "0.16.1+goodfetch.1", ""),
    (
        "wreq-proto",
        "0.2.6",
        "cb4d829f0427a5064829976fe790641ca21abb1d83d844812baf022dc0bdf2db",
    ),
];

/// The workspace's lock file, read when the test builds, so a change to it
/// rebuilds the test.
const LOCK: &str = include_str!("../../../../../../Cargo.lock");

/// A failure whose words are `text`, as a library writes it.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Worded(&'static str);

/// Each package of `Cargo.lock` named `name`: its version and checksum.
fn locked(name: &str) -> Vec<(String, String)> {
    let lock: toml::Table = toml::from_str(LOCK).unwrap();
    lock["package"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|package| package["name"].as_str() == Some(name))
        .map(|package| {
            (
                package["version"].as_str().unwrap().to_owned(),
                package
                    .get("checksum")
                    .and_then(toml::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_forks_the_translation_point_reads_are_those_it_was_written_against() {
    for (name, version, checksum) in PINNED {
        assert_eq!(
            locked(name),
            vec![((*version).to_owned(), (*checksum).to_owned())],
            "`{name}` moved in Cargo.lock: read the table of `transports/impersonated/tests.rs` again against its sources, then update `PINNED`"
        );
    }
}

#[rstest]
#[case::a_proxy_refusing_the_tunnel(
    &Worded("tunnel error: unsuccessful"),
    TransportFailure::TunnelRefused
)]
#[case::a_proxy_asking_for_credentials(
    &Worded("tunnel error: proxy authorization required"),
    TransportFailure::ProxyAuthenticationRequired
)]
#[case::a_proxy_nothing_answers_on(
    &Worded("tunnel error: failed to create underlying connection"),
    TransportFailure::ProxyUnreachable
)]
#[case::another_tunnel_failure(
    &Worded("tunnel error: unexpected end of file"),
    TransportFailure::Other
)]
#[case::a_refused_stream(
    &http2::Error::from(http2::Reason::REFUSED_STREAM),
    TransportFailure::RefusedStream
)]
#[case::http_1_1_required(
    &http2::Error::from(http2::Reason::HTTP_1_1_REQUIRED),
    TransportFailure::Http11Required
)]
/// Covers: story:5-132
#[case::enhance_your_calm(
    &http2::Error::from(http2::Reason::ENHANCE_YOUR_CALM),
    TransportFailure::EnhanceYourCalm
)]
#[case::another_http2_error(
    &http2::Error::from(http2::Reason::INTERNAL_ERROR),
    TransportFailure::Other
)]
#[case::an_empty_boringssl_error_stack(
    &btls::error::ErrorStack::get(),
    TransportFailure::Tls
)]
#[case::a_refused_connection(
    &std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
    TransportFailure::ConnectionRefused
)]
#[case::a_reset_connection(
    &std::io::Error::from(std::io::ErrorKind::ConnectionReset),
    TransportFailure::ConnectionReset
)]
#[case::a_connection_closed_early(
    &std::io::Error::from(std::io::ErrorKind::UnexpectedEof),
    TransportFailure::ConnectionClosed
)]
#[case::an_unknown_failure(&Worded("something else went wrong"), TransportFailure::Other)]
fn each_failure_wreq_meets_has_its_variant(
    #[case] error: &(dyn std::error::Error + 'static),
    #[case] expected: TransportFailure,
) {
    assert_eq!(classify(error), expected);
}

#[test]
fn a_failure_wrapped_in_an_io_error_is_read_inside_it() {
    let wrapped = std::io::Error::other(http2::Error::from(http2::Reason::HTTP_1_1_REQUIRED));

    assert_eq!(classify(&wrapped), TransportFailure::Http11Required);
}
