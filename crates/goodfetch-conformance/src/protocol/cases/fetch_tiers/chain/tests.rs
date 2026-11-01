//! How the redirect chain cases find and time a hop among the requests a
//! host received.

use serde_json::{Value, json};

use super::hop;

/// A request for `target` that arrived at `arrived` µs and was answered
/// at `answered` µs.
fn request(target: &str, arrived: u64, answered: u64) -> Value {
    json!({
        "target": target,
        "timing": { "arrived_microseconds": arrived, "answered_microseconds": answered },
    })
}

/// Another seed arriving between a redirect and its hop is not the hop:
/// the `Retry-After` wait counts from the redirect's answer to the hop.
#[test]
fn a_seed_between_a_redirect_and_its_hop_is_not_the_hop() {
    let requests = [
        request("/retry-after/302/seconds", 0, 1_000),
        request("/retry-after/302/imf-fixdate/long", 3_000, 3_500),
        request("/target", 2_001_000, 2_002_000),
    ];

    let found = hop(&requests, "/retry-after/302/seconds", "/target");

    assert_eq!(
        found.as_ref().and_then(super::Hop::after_the_answer),
        Some(2_000_000)
    );
}

/// The domain's delay spaces the hop from the host's request just before
/// it, arrival to arrival, whichever request that is; a slow answer to the
/// redirect takes nothing from it.
#[test]
fn the_domain_delay_counts_from_the_arrival_of_the_request_before_the_hop() {
    let requests = [
        request("/redirect/307", 0, 400_000),
        request("/target", 1_500_000, 1_501_000),
    ];
    let between = [
        request("/redirect/307", 0, 1_000),
        request("/redirect/robots", 1_500_000, 1_501_000),
        request("/target", 3_000_000, 3_001_000),
    ];

    assert_eq!(
        (
            hop(&requests, "/redirect/307", "/target")
                .as_ref()
                .and_then(super::Hop::after_the_request_before),
            hop(&between, "/redirect/307", "/target")
                .as_ref()
                .and_then(super::Hop::after_the_request_before),
        ),
        (Some(1_500_000), Some(1_500_000))
    );
}

/// A request for the hop's target before the redirect is not its hop, and
/// a redirect never followed has none.
#[test]
fn only_a_request_after_the_redirect_is_its_hop() {
    let requests = [
        request("/target", 0, 1_000),
        request("/redirect/307", 2_000, 3_000),
    ];

    assert_eq!(hop(&requests, "/redirect/307", "/target"), None);
}
