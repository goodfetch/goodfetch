//! An attempt's failure is classed where it enters the crawl: a fetch's
//! by what its transport met, an error object's by its code and details,
//! read once.

use std::time::{Duration, UNIX_EPOCH};

use goodfetch_fetch_tier::{FetchFailure, TransportFailure};
use goodfetch_frontier::Url;
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::HANDLER_FAILED;
use goodfetch_protocol::page::proxy::{PROXY_AUTH_FAILED, PROXY_UNAVAILABLE};
use goodfetch_protocol::page::{BLOCKED, NAVIGATION_FAILED};
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{AttemptFailure, Why};

const URL: &str = "https://shop.test/1";

/// A fetch's failure is classed by what its transport met: a site that
/// refused, timed out or reset the connection is unreachable, and any
/// other failure of the site's says nothing more than its `retryable`.
#[rstest]
#[case::refused(TransportFailure::ConnectionRefused, Why::Unreachable)]
#[case::timed_out(TransportFailure::ConnectionTimedOut, Why::Unreachable)]
#[case::enhance_your_calm(TransportFailure::EnhanceYourCalm, Why::Unreachable)]
#[case::name_not_resolved(TransportFailure::NameNotResolved, Why::Other)]
#[case::reset(TransportFailure::ConnectionReset, Why::Other)]
#[case::closed(TransportFailure::ConnectionClosed, Why::Other)]
#[case::closed_at_reuse(TransportFailure::ClosedAtReuse, Why::Other)]
#[case::refused_stream(TransportFailure::RefusedStream, Why::Other)]
#[case::http_1_1_required(TransportFailure::Http11Required, Why::Other)]
#[case::tls(TransportFailure::Tls, Why::Other)]
#[case::invalid_response(TransportFailure::InvalidResponse, Why::Other)]
#[case::content_decoding(TransportFailure::ContentDecodingFailed, Why::Other)]
#[case::other(TransportFailure::Other, Why::Other)]
fn a_sites_transport_failure_is_classed_by_what_the_hop_met(
    #[case] met: TransportFailure,
    #[case] why: Why,
) {
    let failure = AttemptFailure::of_fetch(&FetchFailure::of(met, URL, None));

    assert_eq!(failure.why(), &why);
    assert_eq!(failure.code, NAVIGATION_FAILED.name);
    assert!(failure.retryable);
}

/// A proxy's own failure is the proxy's, with its Error code: a 407 is
/// not retried, and a tunnel it refused or a proxy nothing answers on is.
#[rstest]
#[case::credentials(TransportFailure::ProxyAuthenticationRequired, PROXY_AUTH_FAILED.name, false)]
#[case::tunnel_refused(TransportFailure::TunnelRefused, PROXY_UNAVAILABLE.name, true)]
#[case::unreachable(TransportFailure::ProxyUnreachable, PROXY_UNAVAILABLE.name, true)]
fn a_proxy_s_own_failure_is_the_proxy_s(
    #[case] met: TransportFailure,
    #[case] code: &str,
    #[case] retryable: bool,
) {
    let proxy = Url::parse("http://user:secret@proxy.test:3128").unwrap();

    let failure = AttemptFailure::of_fetch(&FetchFailure::of(met, URL, Some(&proxy)));

    assert_eq!(failure.why(), &Why::Proxy);
    assert_eq!(
        (failure.code.as_str(), failure.retryable),
        (code, retryable)
    );
    assert_eq!(failure.details["proxy"], json!("http://proxy.test:3128"));
}

/// The details of an error object entering the crawl.
fn details(value: Value) -> Map<String, Value> {
    let Value::Object(details) = value else {
        panic!("details are a table");
    };
    details
}

/// An error object entering the crawl, a handler's answer, is read once.
#[rstest]
#[case::rate_limit("blocked", json!({ "cause": "rate_limit" }), Why::Blocked(BlockCause::RateLimit))]
#[case::consent("blocked", json!({ "cause": "consent_required" }), Why::Blocked(BlockCause::ConsentRequired))]
#[case::unavailable("response_unavailable", json!({ "status": 503 }), Why::Unavailable(Some(503)))]
#[case::unsent("response_unavailable", json!({ "status": null }), Why::Unavailable(None))]
#[case::refused("navigation_failed", json!({ "error": "net::ERR_CONNECTION_REFUSED" }), Why::Unreachable)]
#[case::aborted("navigation_failed", json!({ "error": "net::ERR_ABORTED" }), Why::Other)]
#[case::first_byte("deadline_exceeded", json!({ "limit": { "key": "timeout.first_byte" } }), Why::TimedOut)]
#[case::handler_deadline("deadline_exceeded", json!({ "limit": { "key": "timeout.handler" } }), Why::Other)]
#[case::decoy(
    "page_check_failed",
    json!({ "noindex": true, "url": URL, "status": 200 }),
    Why::PageCheckFailed { noindex: Some((URL.to_owned(), 200)) }
)]
#[case::page_check("page_check_failed", json!({ "url": URL }), Why::PageCheckFailed { noindex: None })]
#[case::handler(HANDLER_FAILED.name, json!({}), Why::Handler)]
#[case::a_hooks_own_exception("hook_failed", json!({ "wrapped_code": null }), Why::Hook)]
#[case::a_hooks_wrapped_code("hook_failed", json!({ "wrapped_code": "header_forbidden" }), Why::Other)]
#[case::a_hooks_deadline("deadline_exceeded", json!({ "limit": { "key": "timeout.hook" } }), Why::Other)]
#[case::unknown("something_else", json!({ "error": "net::ERR_CONNECTION_REFUSED" }), Why::Other)]
fn an_error_object_is_classed_as_it_enters(
    #[case] code: &str,
    #[case] given: Value,
    #[case] why: Why,
) {
    let failure = AttemptFailure::entered(code.to_owned(), true, String::new(), details(given));

    assert_eq!(failure.why(), &why);
}

/// The time a site asked to wait for is read as the error enters.
#[test]
fn the_time_a_site_asked_for_is_read_as_the_error_enters() {
    let failure = AttemptFailure::entered(
        BLOCKED.name.to_owned(),
        true,
        String::new(),
        details(json!({ "cause": "rate_limit", "retry_after": "2026-01-05T09:01:00Z" })),
    );

    assert_eq!(
        failure.retry_after(),
        Some(UNIX_EPOCH + Duration::from_hours(491_001) + Duration::from_mins(1))
    );
}
