//! The proxy-or-site rule: on a response the proxy forwarded, the field's
//! last member, the proxy's own, naming an error is the proxy's whatever
//! the members before it hold, and every other response the site's, as
//! properties, a tunnelled, SOCKS or bypassed response's included; and a
//! `CONNECT` the proxy answers, which Chromium reports as one of its
//! network errors, against a table of its `net_error_list.h` names.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_cdp::NetError;
use proptest::prelude::*;
use rstest::rstest;

use super::{Answer, Party, RELAY, party, refusal, relay_unreachable};

/// An RFC 9209 proxy name or a parameter's token.
fn token() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_.-]{0,12}"
}

/// A quoted string that may hold the separators and escapes.
fn quoted() -> impl Strategy<Value = String> {
    "[ a-z;,=\"\\\\]{0,16}".prop_map(|text| {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    })
}

/// A parameter no proxy's error is: `next-hop`, `received-status`,
/// `details` or another, with a token, a number or a quoted string.
fn other_parameter() -> impl Strategy<Value = String> {
    let key = prop_oneof![
        Just("next-hop".to_owned()),
        Just("received-status".to_owned()),
        Just("details".to_owned()),
        Just("rcode".to_owned()),
        Just("errors".to_owned()),
        Just("an-error".to_owned()),
    ];
    let value = prop_oneof![
        token(),
        (100_u16..600).prop_map(|n| n.to_string()),
        quoted()
    ];
    (key, value).prop_map(|(key, value)| format!("{key}={value}"))
}

/// A list member: a proxy's name and its parameters, with `error` among
/// them when `error` is given.
fn member(error: Option<String>) -> impl Strategy<Value = String> {
    (token(), prop::collection::vec(other_parameter(), 0..3)).prop_map(
        move |(name, mut parameters)| {
            if let Some(error) = &error {
                parameters.push(format!("error={error}"));
            }
            parameters
                .into_iter()
                .fold(name, |member, parameter| format!("{member}; {parameter}"))
        },
    )
}

/// A `Proxy-Status` field of members none of which names an error.
fn without_an_error() -> impl Strategy<Value = String> {
    prop::collection::vec(member(None), 1..4).prop_map(|members| members.join(", "))
}

/// Members, any of which may name an error.
fn any_members() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop_oneof![
            member(None),
            token().prop_flat_map(|error| member(Some(error)))
        ],
        0..3,
    )
}

/// A `Proxy-Status` field whose last member, the nearest intermediary's,
/// names an error, after members that may.
fn with_an_error() -> impl Strategy<Value = String> {
    (
        any_members(),
        token().prop_flat_map(|error| member(Some(error))),
    )
        .prop_map(|(before, erring)| {
            before
                .into_iter()
                .chain(std::iter::once(erring))
                .collect::<Vec<_>>()
                .join(", ")
        })
}

/// A `Proxy-Status` field one of whose members before the last, an
/// intermediary nearer the site, names an error, while the last names
/// none.
fn with_an_error_nearer_the_site() -> impl Strategy<Value = String> {
    (
        any_members(),
        token().prop_flat_map(|error| member(Some(error))),
        any_members(),
        member(None),
    )
        .prop_map(|(before, erring, after, last)| {
            before
                .into_iter()
                .chain(std::iter::once(erring))
                .chain(after)
                .chain(std::iter::once(last))
                .collect::<Vec<_>>()
                .join(", ")
        })
}

/// A response to a URL the proxy forwarded, carrying `field`.
fn forwarded(field: &str) -> Answer<'_> {
    Answer::Response {
        proxy_status: Some(field),
        forwarded: true,
    }
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:5-88, error:proxy_unavailable
    #[test]
    fn a_forwarded_response_whose_last_proxy_status_member_names_an_error_comes_from_the_proxy(field in with_an_error()) {
        prop_assert_eq!(party(forwarded(&field)), Party::Proxy);
    }

    /// Covers: story:5-88
    #[test]
    fn a_forwarded_response_whose_proxy_status_names_no_error_is_the_sites(field in without_an_error()) {
        prop_assert_eq!(party(forwarded(&field)), Party::Site);
    }

    /// Covers: story:5-88
    #[test]
    fn an_error_an_intermediary_nearer_the_site_names_is_the_sites(field in with_an_error_nearer_the_site()) {
        prop_assert_eq!(party(forwarded(&field)), Party::Site);
    }

    /// Covers: story:5-88
    #[test]
    fn a_tunnelled_socks_or_bypassed_response_is_the_sites_whatever_its_proxy_status(field in with_an_error()) {
        prop_assert_eq!(
            party(Answer::Response { proxy_status: Some(&field), forwarded: false }),
            Party::Site
        );
    }

    /// Covers: story:5-88
    #[test]
    fn any_proxy_status_is_read_without_failing(field in any::<String>()) {
        let _ = party(forwarded(&field));
    }
}

/// Covers: story:5-88
#[test]
fn a_response_without_proxy_status_is_the_sites() {
    assert_eq!(
        party(Answer::Response {
            proxy_status: None,
            forwarded: true,
        }),
        Party::Site
    );
}

/// Covers: story:5-90
#[test]
fn the_relays_own_answer_names_the_relay() {
    assert!(
        String::from_utf8_lossy(&relay_unreachable())
            .contains(&format!("Proxy-Status: {RELAY}; error="))
    );
}

/// Covers: story:5-88, error:proxy_unavailable
#[rstest]
#[case::connect_refused("net::ERR_TUNNEL_CONNECTION_FAILED", Party::Proxy)]
#[case::proxy_down("net::ERR_PROXY_CONNECTION_FAILED", Party::Proxy)]
#[case::proxy_certificate("net::ERR_PROXY_CERTIFICATE_INVALID", Party::Proxy)]
#[case::socks_refused("net::ERR_SOCKS_CONNECTION_FAILED", Party::Proxy)]
#[case::socks_host_unreachable("net::ERR_SOCKS_CONNECTION_HOST_UNREACHABLE", Party::Proxy)]
#[case::proxy_auth_unsupported("net::ERR_PROXY_AUTH_UNSUPPORTED", Party::Proxy)]
#[case::https_proxy_redirect("net::ERR_HTTPS_PROXY_TUNNEL_RESPONSE_REDIRECT", Party::Proxy)]
#[case::no_supported_proxies("net::ERR_NO_SUPPORTED_PROXIES", Party::Proxy)]
#[case::site_refused("net::ERR_CONNECTION_REFUSED", Party::Site)]
#[case::name_not_resolved("net::ERR_NAME_NOT_RESOLVED", Party::Site)]
#[case::site_certificate("net::ERR_CERT_AUTHORITY_INVALID", Party::Site)]
#[case::aborted("net::ERR_ABORTED", Party::Site)]
fn a_failure_comes_from_the_proxy_when_chromium_names_its_answer(
    #[case] error: &str,
    #[case] expected: Party,
) {
    assert_eq!(
        party(Answer::Failure {
            error: &NetError::of(error)
        }),
        expected
    );
}

/// Covers: story:5-90
#[rstest]
#[case::its_own_429(
    forwarded("proving-ground; error=http_request_denied"),
    Some(429),
    true
)]
#[case::its_own_502(forwarded("squid; error=connection_refused"), Some(502), true)]
#[case::a_tunnel_it_refused(Answer::Failure { error: &NetError::of("net::ERR_TUNNEL_CONNECTION_FAILED") }, None, true)]
#[case::its_own_403(
    forwarded("proving-ground; error=destination_ip_prohibited"),
    Some(403),
    false
)]
#[case::a_sites_429(Answer::Response { proxy_status: None, forwarded: true }, Some(429), false)]
#[case::a_sites_502_through_it(forwarded("squid; received-status=502"), Some(502), false)]
#[case::a_cdns_429_over_a_tunnel(Answer::Response { proxy_status: Some("cdn; error=http_request_denied"), forwarded: false }, Some(429), false)]
#[case::a_cdns_429_the_proxy_passed_on(
    forwarded("cdn; error=http_request_denied, squid"),
    Some(429),
    false
)]
#[case::the_relays_own_502_for_a_proxy_that_is_down(
    forwarded("goodfetch-relay; error=connection_refused"),
    Some(502),
    false
)]
#[case::it_is_down(Answer::Failure { error: &NetError::of("net::ERR_PROXY_CONNECTION_FAILED") }, None, false)]
#[case::the_site_is_down(Answer::Failure { error: &NetError::of("net::ERR_CONNECTION_REFUSED") }, None, false)]
fn only_the_proxy_itself_answering_429_502_or_refusing_a_tunnel_is_a_refusal_that_lowers_its_ceiling(
    #[case] answer: Answer<'_>,
    #[case] status: Option<u16>,
    #[case] expected: bool,
) {
    assert_eq!(refusal(answer, status), expected);
}
