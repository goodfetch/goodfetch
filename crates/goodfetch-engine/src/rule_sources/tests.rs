//! A Clean-up rules source as an Operator-configured endpoint (ADRs 0017
//! and 0047; spec #564's two egress classes): `egress.no_private_endpoints`
//! refuses a source that resolves to a private address before any
//! connection, and without it the source is reached.
//!
//! Covers: story:4-16, setting:egress.no_private_endpoints
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::egress::EgressRule;

use super::{SourceFailure, get};
use crate::clock::Clock;
use crate::egress::{EndpointPolicy, Refused};
use crate::http_client::Resolve;
use crate::pinned_download::Fetching;

/// A source's URL whose name the `resolve` mapping sends to loopback, on a
/// port nothing listens on.
const SOURCE: &str = "https://rules.test:9/filters.txt";

/// What a fetch may do with `endpoints`, `rules.test` mapped to loopback.
fn fetching(endpoints: EndpointPolicy) -> Fetching {
    Fetching {
        resolve: Resolve::new().with("rules.test", vec!["127.0.0.1".parse().unwrap()]),
        endpoints,
        ..Fetching::new(Clock::system())
    }
}

/// What a GET of the source came to.
fn fetched(endpoints: EndpointPolicy) -> SourceFailure {
    goodfetch_test_support::runtime()
        .block_on(get(SOURCE, &[], &fetching(endpoints), 1024))
        .unwrap_err()
}

#[test]
fn a_source_at_a_private_address_is_refused_under_no_private_endpoints() {
    let refused = fetched(EndpointPolicy::new(true));
    // The positive control: without the setting the source is reached for,
    // and only the closed port fails it.
    let reached = fetched(EndpointPolicy::default());

    let SourceFailure::Refused(refused) = refused else {
        panic!("the private source is refused: {refused:?}");
    };
    assert_eq!(
        refused,
        Refused {
            url: SOURCE.to_owned(),
            rule: EgressRule::PrivateAddress,
            address: Some("127.0.0.1".parse().unwrap()),
            setting: Some("egress.no_private_endpoints"),
        }
    );
    assert!(
        matches!(&reached, SourceFailure::Network { host } if host == "rules.test"),
        "{reached:?}"
    );
}
