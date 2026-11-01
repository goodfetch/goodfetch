//! The translation point's table (ADR 0127): one named case per answer it
//! knows, and one for an answer it does not recognize, each with the Error
//! code a call that waited for the request answers with.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use rstest::rstest;

use super::{ProxyFailure, Unrecognized, answered};
use crate::egress::EndpointPolicy;
use crate::http_client::UpstreamProxy;

/// A failure's Error code, as the call that waited answers with it.
fn code(failure: &ProxyFailure) -> String {
    let proxy = UpstreamProxy {
        host: "proxy.test".to_owned(),
        port: 3128,
        policy: EndpointPolicy::default(),
    };
    failure
        .error(&proxy, "https://site.test/")
        .code()
        .to_owned()
}

#[rstest]
#[case::established(b"HTTP/1.1 200 Connection Established\r\n\r\n")]
#[case::established_with_another_2xx(b"HTTP/1.0 204 No Content\r\n\r\n")]
fn a_2xx_answer_opens_the_tunnel(#[case] head: &[u8]) {
    assert_eq!(answered(head), Ok(()));
}

#[rstest]
/// Covers: error:proxy_auth_failed
#[case::authentication_required(
    b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"p\"\r\n\r\n",
    ProxyFailure::AuthenticationRequired,
    "proxy_auth_failed"
)]
/// Covers: error:proxy_unavailable
#[case::rate_limited(
    b"HTTP/1.1 429 Too Many Requests\r\n\r\n",
    ProxyFailure::RateLimited,
    "proxy_unavailable"
)]
/// Covers: error:proxy_unavailable
#[case::bad_gateway(
    b"HTTP/1.1 502 Bad Gateway\r\nProxy-Status: p; error=dns_error\r\n\r\n",
    ProxyFailure::BadGateway,
    "proxy_unavailable"
)]
/// Covers: error:proxy_unavailable
#[case::an_unknown_refusal(
    b"HTTP/1.1 403 Forbidden\r\n\r\n",
    ProxyFailure::Unrecognized(Unrecognized { answer: "HTTP/1.1 403 Forbidden".to_owned() }),
    "proxy_unavailable"
)]
/// Covers: error:proxy_unavailable
#[case::no_http_answer(
    b"SSH-2.0-OpenSSH_9.6\r\n\r\n",
    ProxyFailure::Unrecognized(Unrecognized { answer: "SSH-2.0-OpenSSH_9.6".to_owned() }),
    "proxy_unavailable"
)]
fn a_refused_tunnel_is_a_typed_failure_of_the_proxy(
    #[case] head: &[u8],
    #[case] failure: ProxyFailure,
    #[case] error_code: &str,
) {
    let refused = answered(head);

    assert_eq!(refused, Err(failure));
    assert_eq!(code(&refused.unwrap_err()), error_code);
}

/// Covers: error:proxy_unavailable
#[test]
fn a_proxy_nothing_answers_on_is_unavailable() {
    assert_eq!(code(&ProxyFailure::Unreachable), "proxy_unavailable");
}
