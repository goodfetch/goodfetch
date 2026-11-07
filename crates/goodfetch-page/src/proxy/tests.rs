//! A Session's proxy reading its answers: which responses the proxy can
//! have forwarded itself, by the URL's scheme, the proxy's kind and
//! `proxy.bypass`, so a site's own `Proxy-Status` over a tunnel or a SOCKS
//! proxy stays the site's.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_cdp::NetError;
use goodfetch_protocol::page::proxy::{PROXY_PASSWORD, PROXY_USERNAME, ProxyServer};
use rstest::rstest;
use secrecy::SecretString;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use goodfetch_engine::egress::EndpointPolicy;
use goodfetch_test_support::{Outcome, Refusal, Runner, SsrfCase, SsrfTable};

use super::{
    Answer, Party, SessionProxy, party, proxy_addresses, relay_unavailable, too_long_for_socks5,
};

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
        through_relay: crate::egress_relay::Outcomes::default(),
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
        (party(answer), proxy.refusal(url, answer, Some(429))),
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
fn a_socks5_response_the_relay_wrote_belongs_to_the_proxy(
    #[case] met: crate::egress_relay::serve::Met,
    #[case] expected: Party,
) {
    let proxy = proxy("socks5://fwd:1080", None);
    proxy.met_through_relay("shop.example:80", met);

    let answer = proxy.response(
        "http://shop.example/",
        Some("goodfetch-relay; error=connection_refused"),
    );

    assert_eq!(party(answer), expected);
}

/// Each of a Session's concurrent connections is classified by its own
/// outcome, never by whichever connection the relay met last (deep review
/// finding 577-5): the proxy refused one connection's tunnel and the
/// credentials of another's, and served a third's after them, yet the
/// first's response is still the proxy's, its tunnel failure no refusal of
/// a proxy that was down, and the second's failure `proxy_auth_failed`;
/// the third's are the site's.
///
/// Covers: story:5-88, error:proxy_unavailable, error:proxy_auth_failed
#[test]
fn each_concurrent_connection_is_classified_by_its_own_outcome() {
    use crate::egress_relay::serve::Met;
    let proxy = proxy("socks5://fwd:1080", None);
    let relayed = Some("goodfetch-relay; error=connection_refused");
    let tunnel = Answer::Failure {
        error: &NetError::TunnelRefused,
    };
    proxy.met_through_relay("down.example:80", Met::Unreachable);
    proxy.met_through_relay("refused.example:80", Met::CredentialsRefused);
    proxy.met_through_relay("served.example:80", Met::Reached);

    let classified = |url: &str| {
        (
            party(proxy.response(url, relayed)),
            proxy.refusal(url, tunnel, None),
            proxy
                .failure(url, &NetError::TunnelRefused)
                .map(|error| error.code().to_owned()),
        )
    };

    assert_eq!(
        [
            classified("http://down.example/"),
            classified("http://refused.example/"),
            classified("http://served.example/"),
        ],
        [
            (Party::Proxy, false, Some("proxy_unavailable".to_owned())),
            (Party::Proxy, true, Some("proxy_auth_failed".to_owned())),
            (Party::Site, true, Some("proxy_unavailable".to_owned())),
        ]
    );
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
        "https://shop.example/",
        Answer::Failure {
            error: &NetError::TunnelRefused
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
        too_long_for_socks5(&"u".repeat(username), Some(&password)).map(|setting| setting.name),
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

/// An address that stands for a public one.
const PUBLIC: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 31, 196, 1));

/// What the Session's open finds of the proxy at `server` when its host's
/// lookups answer `answers` in turn, `relayed` for a proxy behind the
/// loopback relay, under `egress.no_private_endpoints` when `refusing`; and
/// how many lookups it made.
fn opened_with(
    server: &str,
    relayed: bool,
    refusing: bool,
    answers: Vec<Option<Vec<IpAddr>>>,
) -> (Result<Vec<IpAddr>, String>, usize) {
    let server = ProxyServer::parse(server).unwrap();
    let port = server.port;
    let answers = RefCell::new(VecDeque::from(answers));
    let made = Cell::new(0);
    let lookup = || {
        made.set(made.get() + 1);
        let answer = answers.borrow_mut().pop_front().flatten();
        std::future::ready(answer.map(|addresses| {
            addresses
                .into_iter()
                .map(|address| SocketAddr::new(address, port))
                .collect()
        }))
    };

    let found = goodfetch_test_support::runtime().block_on(proxy_addresses(
        &server,
        EndpointPolicy::new(refusing),
        relayed,
        lookup,
    ));

    (
        found
            .map(|addresses| addresses.iter().map(SocketAddr::ip).collect())
            .map_err(|error| error.code().to_owned()),
        made.get(),
    )
}

/// The Session's open looks its proxy's host up once and judges that
/// answer, and the loopback relay is given only the addresses judged (deep
/// review findings 577-1, 564-3 and 599-13): a name that answers a public
/// address, then a private one, reaches the relay as the public one; under
/// `egress.no_private_endpoints` a lookup that fails refuses the open,
/// relayed or not, while with the switch off a proxy the browser reaches
/// itself is left to its connection.
///
/// Covers: story:4-16, setting:egress.no_private_endpoints, error:egress_refused, error:proxy_unavailable
#[rstest]
#[case::a_rebinding_answer(
    true,
    true,
    vec![Some(vec![PUBLIC]), Some(vec![Ipv4Addr::LOCALHOST.into()])],
    Ok(vec![PUBLIC]),
)]
#[case::a_failed_lookup_then_a_private_answer(
    true,
    true,
    vec![None, Some(vec![Ipv4Addr::LOCALHOST.into()])],
    Err("proxy_unavailable"),
)]
#[case::a_failed_lookup_of_a_proxy_the_browser_reaches(
    false,
    true,
    vec![None, Some(vec![Ipv4Addr::LOCALHOST.into()])],
    Err("proxy_unavailable"),
)]
#[case::a_failed_lookup_with_the_switch_off(false, false, vec![None], Ok(vec![]))]
#[case::a_private_answer(
    true,
    true,
    vec![Some(vec![Ipv4Addr::LOCALHOST.into()])],
    Err("egress_refused"),
)]
fn the_open_judges_one_lookup_and_the_relay_dials_only_its_judged_addresses(
    #[case] relayed: bool,
    #[case] refusing: bool,
    #[case] answers: Vec<Option<Vec<IpAddr>>>,
    #[case] expected: Result<Vec<IpAddr>, &str>,
) {
    let opened = opened_with("socks5://px.ssrf.test:1080", relayed, refusing, answers);

    assert_eq!(opened, (expected.map_err(str::to_owned), 1));
}

/// The Session's proxy at its open, the layer `session_proxy` of the SSRF
/// case table (ADR 0067): `case`'s URL as the proxy's, its lookups answered
/// in turn, one open for each of its fetches, each judged under
/// `egress.no_private_endpoints` when `refusing`; its connections are the
/// addresses the loopback relay is given to connect to.
fn session_proxy_case(case: &SsrfCase, refusing: bool) -> Outcome {
    let Some((host, port)) = url::Url::parse(&case.url)
        .ok()
        .and_then(|url| Some((url.host_str()?.to_owned(), url.port_or_known_default()?)))
    else {
        return Outcome::default();
    };
    let Ok(server) = ProxyServer::parse(&format!("socks5://{host}:{port}")) else {
        return Outcome::default();
    };
    let mut lookups: VecDeque<Vec<IpAddr>> = case
        .lookups
        .iter()
        .map(|lookup| {
            lookup
                .answers
                .iter()
                .map(|answer| match answer {
                    goodfetch_test_support::Answer::Public => PUBLIC,
                    goodfetch_test_support::Answer::Address(address) => *address,
                })
                .collect()
        })
        .collect();
    let literal = server.host.parse::<IpAddr>().ok();
    let mut outcome = Outcome::default();
    for _ in 0..case.fetches {
        let lookup = || {
            let answer = literal
                .map(|address| vec![address])
                .or_else(|| lookups.pop_front());
            std::future::ready(answer.map(|addresses| {
                addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, server.port))
                    .collect()
            }))
        };
        let found = goodfetch_test_support::runtime().block_on(proxy_addresses(
            &server,
            EndpointPolicy::new(refusing),
            true,
            lookup,
        ));
        outcome.refusal = match found {
            Ok(addresses) => {
                outcome
                    .connections
                    .extend(addresses.iter().map(SocketAddr::ip));
                None
            }
            Err(error) if error.code() == "egress_refused" => Some(Refusal::EgressRefused {
                rule: goodfetch_test_support::EgressRule::PrivateAddress,
            }),
            Err(_) => None,
        };
    }
    outcome
}

/// The table's Operator endpoint and rebinding cases, each URL the
/// Session's proxy's: refused under `egress.no_private_endpoints` before the
/// loopback relay is given an address that was not judged, a rebinding name
/// at the open after its answer changed, while the positive control, a
/// proxy on loopback with the switch off, is given its address.
///
/// Covers: story:4-16, story:4-89, error:egress_refused, setting:egress.no_private_endpoints
#[test]
fn every_operator_endpoint_case_of_the_ssrf_table_is_refused_at_the_sessions_open() {
    let runner = Runner::committed(
        "session_proxy",
        file!(),
        "every_operator_endpoint_case_of_the_ssrf_table_is_refused_at_the_sessions_open",
    )
    .unwrap();
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "loopback_proxy_with_the_switch_off"
        family = "operator_endpoint"
        url = "http://127.0.0.1:1080/"
        route = { via = "operator_endpoint", kind = "proxy" }
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();

    let result = runner.run(&controls.cases, PUBLIC, |case| {
        let control = case.name == "loopback_proxy_with_the_switch_off";
        session_proxy_case(case, !control)
    });

    assert!(
        runner
            .cases()
            .iter()
            .any(|case| case.name == "proxy_endpoint_that_rebinds"),
        "{:?}",
        runner.cases()
    );
    // A zone ID's URL is no proxy URL (`ProxyServer::parse` refuses it with
    // `setting_invalid`), so no Session opens with it and nothing is given
    // to the relay.
    assert_eq!(
        result.map_err(|failed| {
            let let_through: Vec<_> = failed
                .let_through
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect();
            (failed.no_control, failed.held, let_through)
        }),
        Err((false, Vec::new(), vec![("zone_id".to_owned(), None, false)]))
    );
}
