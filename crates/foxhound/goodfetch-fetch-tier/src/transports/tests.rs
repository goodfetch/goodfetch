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
