//! Covers: story:5-128

use super::classify;
use crate::driver::TransportFailure;

/// A failure no library named.
#[derive(Debug, thiserror::Error)]
#[error("something else went wrong")]
struct Unknown;

#[test]
fn an_unknown_failure_maps_to_the_generic_case() {
    let failure = classify(&Unknown);

    assert_eq!(failure, TransportFailure::Other);
}

#[test]
fn an_unknown_failure_wrapped_in_an_io_error_maps_to_the_generic_case() {
    let wrapped = std::io::Error::other(Unknown);

    let failure = classify(&wrapped);

    assert_eq!(failure, TransportFailure::Other);
}

/// Covers: story:5-127
#[test]
fn a_refused_http2_stream_is_the_unprocessed_refusal() {
    let failure = classify(&h2::Error::from(h2::Reason::REFUSED_STREAM));

    assert_eq!(failure, TransportFailure::RefusedStream);
}

/// Covers: story:5-127
#[test]
fn http_1_1_required_wrapped_in_an_io_error_is_its_own_failure() {
    let wrapped = std::io::Error::other(h2::Error::from(h2::Reason::HTTP_1_1_REQUIRED));

    let failure = classify(&wrapped);

    assert_eq!(failure, TransportFailure::Http11Required);
}

/// Covers: story:5-127
#[test]
fn another_http2_error_is_the_generic_case() {
    let failure = classify(&h2::Error::from(h2::Reason::INTERNAL_ERROR));

    assert_eq!(failure, TransportFailure::Other);
}
