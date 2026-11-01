use goodfetch_page::Party;
use rstest::rstest;

use super::{Class, Met, class, party_of};

/// A response the site gave, with no `Proxy-Status`.
fn site(status: u16) -> Met<'static> {
    Met::Response {
        status,
        proxy_status: None,
        forwarded: false,
    }
}

/// Covers: story:5-132
#[rstest]
#[case::a_429(site(429), Class::TooManyRequests)]
#[case::a_502(site(502), Class::BackOff)]
#[case::a_503(site(503), Class::BackOff)]
#[case::a_504(site(504), Class::BackOff)]
#[case::a_refused_connection(Met::Failure { error: "net::ERR_CONNECTION_REFUSED" }, Class::BackOff)]
#[case::a_timed_out_connection(Met::Failure { error: "net::ERR_CONNECTION_TIMED_OUT" }, Class::BackOff)]
#[case::enhance_your_calm(Met::Failure { error: "net::ERR_HTTP2_PROTOCOL_ERROR" }, Class::BackOff)]
#[case::a_500(site(500), Class::Done)]
#[case::a_404(site(404), Class::Done)]
#[case::a_200(site(200), Class::Done)]
#[case::a_reset_connection(Met::Failure { error: "net::ERR_CONNECTION_RESET" }, Class::Done)]
fn each_answer_of_the_site_asks_its_domain_for_its_class(
    #[case] met: Met<'static>,
    #[case] expected: Class,
) {
    assert_eq!((party_of(met), class(met)), (Party::Site, expected));
}

/// Covers: story:5-133
#[rstest]
#[case::its_own_429(Met::Response { status: 429, proxy_status: Some("proxy.test; error=http_request_denied"), forwarded: true }, Party::Proxy, Class::ProxyRefusal)]
#[case::its_own_502(Met::Response { status: 502, proxy_status: Some("proxy.test; error=destination_unavailable"), forwarded: true }, Party::Proxy, Class::ProxyRefusal)]
#[case::a_refused_tunnel(Met::Failure { error: "net::ERR_TUNNEL_CONNECTION_FAILED" }, Party::Proxy, Class::ProxyRefusal)]
#[case::its_407(Met::Failure { error: "net::ERR_PROXY_AUTH_REQUESTED" }, Party::Proxy, Class::Done)]
#[case::a_proxy_nothing_answers_on(Met::Failure { error: "net::ERR_PROXY_CONNECTION_FAILED" }, Party::Proxy, Class::Done)]
#[case::the_sites_429_through_it(Met::Response { status: 429, proxy_status: Some("proxy.test"), forwarded: true }, Party::Site, Class::TooManyRequests)]
#[case::the_sites_502_over_a_tunnel(Met::Response { status: 502, proxy_status: Some("cdn.test; error=connection_timeout"), forwarded: false }, Party::Site, Class::BackOff)]
fn a_proxy_s_own_answer_lowers_its_ceiling_and_backs_no_domain_off(
    #[case] met: Met<'static>,
    #[case] party: Party,
    #[case] expected: Class,
) {
    assert_eq!((party_of(met), class(met)), (party, expected));
}
