//! A Session's proxy reading its answers: which responses the proxy can
//! have forwarded itself, by the URL's scheme, the proxy's kind and
//! `proxy.bypass`, so a site's own `Proxy-Status` over a tunnel or a SOCKS
//! proxy stays the site's.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::page::proxy::{PROXY_PASSWORD, PROXY_USERNAME, ProxyServer};
use rstest::rstest;
use secrecy::SecretString;

use super::{Answer, Party, SessionProxy, party, relay_unavailable, too_long_for_socks5};

/// A Session's proxy at `server`, with `bypass`, and no relay or limits.
fn proxy(server: &str, bypass: Option<&str>) -> SessionProxy {
    SessionProxy {
        server: ProxyServer::parse(server).unwrap(),
        bypass: bypass.map(str::to_owned),
        username: None,
        password: None,
        relay: None,
        provided: None,
        limits: None,
        through_relay: crate::egress_relay::Latest::default(),
        judged: goodfetch_engine::egress::EndpointPolicy::default(),
    }
}

/// The proxy's own refusal, as a forward proxy writes it.
const ITS_OWN: &str = "squid; error=http_request_denied";

/// Covers: story:5-88, story:5-90, error:proxy_unavailable
#[rstest]
#[case::a_plain_url_it_forwarded(
    "http://fwd:3128",
    None,
    "http://shop.example/",
    Party::Proxy,
    true
)]
#[case::a_plain_url_through_an_https_proxy(
    "https://fwd:3128",
    None,
    "http://shop.example/",
    Party::Proxy,
    true
)]
#[case::a_tunnelled_url("http://fwd:3128", None, "https://shop.example/", Party::Site, false)]
#[case::a_url_through_socks("socks5://fwd:1080", None, "http://shop.example/", Party::Site, false)]
#[case::a_bypassed_url(
    "http://fwd:3128",
    Some(".example"),
    "http://shop.example/",
    Party::Site,
    false
)]
fn only_a_response_the_proxy_forwarded_can_be_its_own(
    #[case] server: &str,
    #[case] bypass: Option<&str>,
    #[case] url: &str,
    #[case] expected: Party,
    #[case] refused: bool,
) {
    let proxy = proxy(server, bypass);
    let answer = proxy.response(url, Some(ITS_OWN));

    assert_eq!(
        (party(answer), proxy.refusal(answer, Some(429))),
        (expected, refused)
    );
}

/// Through a SOCKS5 proxy, a plain response naming the Session's relay is
/// the proxy's only when the relay recorded that it answered the latest
/// connection itself (finding 577-1): a site's own field naming the relay,
/// with the relay having reached the proxy, stays the site's.
/// Covers: story:5-88, error:proxy_unavailable
#[rstest]
#[case::the_relay_answered(crate::egress_relay::serve::Met::Unreachable, Party::Proxy)]
#[case::the_proxy_refused_the_tunnel(crate::egress_relay::serve::Met::Refused, Party::Proxy)]
#[case::the_relay_reached_the_proxy(crate::egress_relay::serve::Met::Reached, Party::Site)]
fn a_socks5_response_the_relay_wrote_is_the_proxys(
    #[case] met: crate::egress_relay::serve::Met,
    #[case] expected: Party,
) {
    let proxy = proxy("socks5://fwd:1080", None);
    proxy.met_through_relay(met);

    let answer = proxy.response(
        "http://shop.example/",
        Some("goodfetch-relay; error=connection_refused"),
    );

    assert_eq!(party(answer), expected);
}

/// Covers: story:5-90, setting:proxy.bypass
#[test]
fn a_bypassed_host_goes_around_the_proxy() {
    let proxy = proxy("http://fwd:3128", Some("intranet.example"));

    assert_eq!(
        (
            proxy.bypasses("https://intranet.example/"),
            proxy.bypasses("https://shop.example/")
        ),
        (true, false)
    );
}

/// Covers: story:5-90
#[test]
fn a_tunnel_refused_is_a_refusal_of_a_proxy_without_a_relay() {
    let proxy = proxy("http://fwd:3128", None);

    assert!(proxy.refusal(
        Answer::Failure {
            error: "net::ERR_TUNNEL_CONNECTION_FAILED"
        },
        None
    ));
}

/// Covers: story:5-88, setting:proxy.username, setting:proxy.password, error:setting_invalid
#[rstest]
#[case::both_fit(255, 255, None)]
#[case::a_long_username(256, 8, Some(PROXY_USERNAME.name))]
#[case::a_long_password(8, 256, Some(PROXY_PASSWORD.name))]
fn credentials_longer_than_socks5_carries_name_their_setting(
    #[case] username: usize,
    #[case] password: usize,
    #[case] expected: Option<&str>,
) {
    let password = SecretString::from("p".repeat(password));

    assert_eq!(
        too_long_for_socks5(&"u".repeat(username), Some(&password)),
        expected
    );
}

/// Covers: story:5-88, error:proxy_relay_unavailable
#[test]
fn a_relay_that_cannot_open_raises_the_engines_own_code_never_the_proxy_code() {
    let server = ProxyServer::parse("socks5://exit.example:1080").unwrap();
    // Too many open files: the machine's failure, before any proxy.
    let error = std::io::Error::from_raw_os_error(24);

    let raised = relay_unavailable(&server, &error);

    assert_eq!(
        (
            raised.code(),
            raised.retryable(),
            raised.details().map(|details| details["proxy"].clone())
        ),
        (
            "proxy_relay_unavailable",
            true,
            Some(serde_json::json!("socks5://exit.example:1080"))
        )
    );
}
