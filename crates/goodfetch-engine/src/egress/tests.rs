//! The domain policy through `egress`'s interface (spec #564's `egress`
//! seam): which hosts `judge` allows and refuses, with the rule and the
//! setting it names, whatever spelling of a host the URL parser accepts.
//!
//! Covers: story:4-2, story:4-3, setting:egress.allowed_domains, setting:egress.blocked_domains
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::egress::EgressRule;
use goodfetch_protocol::settings::SettingsLayer;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    InvalidEntry, Judgment, Policy, Refused, Request, RequestKind, judge, judge_addresses,
};
use crate::settings::{Layers, Values, resolve};

fn policy(allowed: &[&str], blocked: &[&str]) -> Policy {
    Policy::new(allowed, blocked).unwrap()
}

fn fetch(url: &str) -> Request<'_> {
    Request {
        url,
        kind: RequestKind::Fetch,
    }
}

/// What a judgment comes to, as a test compares it: the URL it allowed, or
/// the refusal.
fn verdict(judgment: Judgment) -> Result<String, Refused> {
    match judgment {
        Judgment::Allowed(url) => Ok(url.to_string()),
        Judgment::Refused(refused) => Err(refused),
    }
}

fn refused(url: &str, rule: EgressRule, setting: Option<&'static str>) -> Refused {
    Refused {
        url: url.to_owned(),
        rule,
        address: None,
        setting,
    }
}

const ALLOWED: Option<&str> = Some("egress.allowed_domains");
const BLOCKED: Option<&str> = Some("egress.blocked_domains");

#[rstest]
#[case::the_host_itself("https://scope.test/", "https://scope.test/")]
#[case::a_subdomain("https://www.scope.test/news", "https://www.scope.test/news")]
#[case::a_subdomain_of_a_subdomain("https://a.b.scope.test/", "https://a.b.scope.test/")]
#[case::in_capitals("https://WWW.Scope.TEST/", "https://www.scope.test/")]
fn a_listed_host_and_its_subdomains_are_allowed(#[case] url: &str, #[case] parsed: &str) {
    let session = policy(&["scope.test"], &[]);

    assert_eq!(verdict(judge(&fetch(url), &session)), Ok(parsed.to_owned()));
}

#[rstest]
#[case::another_host("https://elsewhere.test/offsite")]
#[case::a_name_ending_in_the_entry_without_a_dot("https://notscope.test/")]
#[case::the_parent_of_the_entry("https://test/")]
#[case::a_sister_name("https://scope-sister.test/sister")]
fn a_host_the_allowed_domains_do_not_list_is_refused_naming_the_setting(#[case] url: &str) {
    let session = policy(&["scope.test"], &[]);

    assert_eq!(
        verdict(judge(&fetch(url), &session)),
        Err(refused(url, EgressRule::DomainPolicy, ALLOWED))
    );
}

#[rstest]
#[case::a_subdomain_of_an_allowed_host(&["scope.test"], &["shop.scope.test"], "https://shop.scope.test/cart")]
#[case::the_allowed_host_itself(&["scope.test"], &["scope.test"], "https://scope.test/")]
#[case::under_every_host_allowed(&["*"], &["scope.test"], "https://www.scope.test/news")]
#[case::every_host_blocked(&["scope.test"], &["*"], "https://scope.test/")]
fn a_blocked_entry_beats_an_allowed_one(
    #[case] allowed: &[&str],
    #[case] blocked: &[&str],
    #[case] url: &str,
) {
    let session = policy(allowed, blocked);

    assert_eq!(
        verdict(judge(&fetch(url), &session)),
        Err(refused(url, EgressRule::DomainPolicy, BLOCKED))
    );
}

#[test]
fn a_host_beside_a_blocked_one_stays_allowed() {
    let session = policy(&["scope.test"], &["shop.scope.test"]);

    assert_eq!(
        verdict(judge(&fetch("https://www.scope.test/news"), &session)),
        Ok("https://www.scope.test/news".to_owned())
    );
}

#[test]
fn every_host_is_allowed_when_star_is_written() {
    let session = policy(&["*"], &[]);

    assert_eq!(
        verdict(judge(&fetch("https://elsewhere.test/offsite"), &session)),
        Ok("https://elsewhere.test/offsite".to_owned())
    );
}

#[test]
fn an_empty_allowed_list_allows_no_host() {
    let session = policy(&[], &[]);

    assert_eq!(
        verdict(judge(&fetch("https://scope.test/"), &session)),
        Err(refused(
            "https://scope.test/",
            EgressRule::DomainPolicy,
            ALLOWED
        ))
    );
}

#[rstest]
#[case::decimal_integer(&["127.0.0.1"], "http://2130706433/")]
#[case::hexadecimal_integer(&["127.0.0.1"], "http://0x7f000001/")]
#[case::octal_parts(&["127.0.0.1"], "http://0177.0.0.01/")]
#[case::short_form(&["127.0.0.1"], "http://127.1/")]
#[case::an_entry_in_decimal(&["2130706433"], "http://127.0.0.1/")]
#[case::an_ipv6_spelling(&["[::1]"], "http://[0:0:0:0:0:0:0:1]/")]
#[case::an_ipv6_entry_without_brackets(&["::1"], "http://[::1]/")]
#[case::punycode_for_an_entry_in_unicode(&["bücher.example.test"], "http://xn--bcher-kva.example.test/")]
#[case::unicode_for_an_entry_in_punycode(&["xn--bcher-kva.example.test"], "http://bücher.example.test/")]
#[case::capitals(&["shop.scope.test"], "http://SHOP.Scope.TEST/")]
#[case::a_trailing_dot(&["shop.scope.test"], "http://shop.scope.test./")]
#[case::an_entry_with_a_trailing_dot(&["shop.scope.test."], "http://shop.scope.test/")]
#[case::a_percent_encoded_dot(&["shop.scope.test"], "http://shop%2Escope.test/")]
#[case::full_width_digits(&["127.0.0.1"], "http://１２７.０.０.１/")]
fn a_host_is_judged_as_the_url_parser_leaves_it(#[case] blocked: &[&str], #[case] url: &str) {
    let session = policy(&["*"], blocked);

    assert_eq!(
        verdict(judge(&fetch(url), &session)),
        Err(refused(url, EgressRule::DomainPolicy, BLOCKED))
    );
}

#[rstest]
#[case::an_unclosed_ipv6_address("http://[::1/")]
#[case::no_scheme("scope.test/")]
#[case::a_space_in_the_host("http://sco pe.test/")]
#[case::an_empty_text("")]
fn a_url_that_does_not_parse_is_refused(#[case] url: &str) {
    let session = policy(&["*"], &[]);

    assert_eq!(
        verdict(judge(&fetch(url), &session)),
        Err(refused(url, EgressRule::DomainPolicy, None))
    );
}

#[rstest]
#[case::file("file:///etc/passwd")]
#[case::data("data:text/html,<p>hi</p>")]
#[case::javascript("javascript:alert(1)")]
#[case::gopher("gopher://scope.test/")]
#[case::ftp("ftp://scope.test/")]
fn a_scheme_no_request_may_use_is_refused(#[case] url: &str) {
    let session = policy(&["*"], &[]);

    assert_eq!(
        verdict(judge(&fetch(url), &session)),
        Err(refused(url, EgressRule::Scheme, None))
    );
}

#[rstest]
#[case::websocket("ws://scope.test/socket")]
#[case::secure_websocket("wss://scope.test/socket")]
#[case::plain_http("http://scope.test/")]
fn a_scheme_of_the_web_is_judged_by_its_host(#[case] url: &str) {
    let session = policy(&["scope.test"], &[]);

    assert_eq!(verdict(judge(&fetch(url), &session)), Ok(url.to_owned()));
}

#[test]
fn a_request_whose_kind_cannot_be_seen_is_refused() {
    let session = policy(&["*"], &[]);
    let request = Request {
        url: "https://scope.test/",
        kind: RequestKind::Unseen,
    };

    assert_eq!(
        verdict(judge(&request, &session)),
        Err(refused(
            "https://scope.test/",
            EgressRule::DomainPolicy,
            None
        ))
    );
}

#[rstest]
#[case::a_wildcard_label("*.scope.test")]
#[case::a_path("scope.test/news")]
#[case::a_port("scope.test:443")]
#[case::empty("")]
#[case::a_space("sco pe.test")]
fn an_entry_that_is_no_host_is_refused(#[case] entry: &str) {
    assert_eq!(
        Policy::new(&["*"], &[entry]).unwrap_err(),
        InvalidEntry {
            setting: "egress.blocked_domains",
            entry: entry.to_owned(),
        }
    );
}

fn run_values(settings: Value) -> Values {
    let Value::Object(nested) = settings else {
        panic!("the run's settings are a table");
    };
    Values::read(SettingsLayer::Run, &nested, Product::Collie).unwrap()
}

#[test]
fn the_policy_is_the_runs_two_settings() {
    let layers = Layers {
        run: run_values(json!({
            "egress": {
                "allowed_domains": ["scope.test"],
                "blocked_domains": ["shop.scope.test"],
            }
        })),
        ..Layers::default()
    };
    let settings = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    let session = Policy::of(&settings).unwrap();

    assert_eq!(
        [
            "https://www.scope.test/news",
            "https://shop.scope.test/cart",
            "https://elsewhere.test/offsite",
        ]
        .map(|url| verdict(judge(&fetch(url), &session)).map_err(|refused| refused.setting)),
        [
            Ok("https://www.scope.test/news".to_owned()),
            Err(BLOCKED),
            Err(ALLOWED),
        ]
    );
}

#[test]
fn with_no_setting_every_host_is_allowed() {
    let settings = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();

    let session = Policy::of(&settings).unwrap();

    assert_eq!(
        verdict(judge(&fetch("https://elsewhere.test/offsite"), &session)),
        Ok("https://elsewhere.test/offsite".to_owned())
    );
}

/// A label of a domain name.
fn label() -> impl Strategy<Value = String> {
    // A label starting `xn--` is punycode, which the URL parser refuses
    // unless it decodes: no host at all.
    "[a-z][a-z0-9-]{0,8}[a-z0-9]".prop_filter("an ASCII label", |label| !label.starts_with("xn--"))
}

proptest! {
    /// A blocked host's subdomains are refused whatever the allowed list
    /// holds.
    #[test]
    fn a_blocked_hosts_subdomains_are_refused_whatever_is_allowed(
        blocked in label(),
        subdomain in prop::collection::vec(label(), 0..3),
        allowed in prop::collection::vec(label(), 0..4),
    ) {
        let blocked = format!("{blocked}.test");
        let mut allowed: Vec<String> = allowed.into_iter().map(|label| format!("{label}.test")).collect();
        allowed.push("*".to_owned());
        let host = subdomain.iter().rev().fold(blocked.clone(), |host, label| format!("{label}.{host}"));
        let url = format!("https://{host}/");
        let session = Policy::new(&allowed, &[blocked]).unwrap();

        prop_assert_eq!(
            verdict(judge(&fetch(&url), &session)),
            Err(refused(&url, EgressRule::DomainPolicy, BLOCKED))
        );
    }
}

mod cdp_urls;
mod endpoints;
mod gate;
mod navigations;
mod ssrf;

/// The socket addresses a connection to `addresses` on port 80 tries.
fn sockets(addresses: &[&str]) -> Vec<std::net::SocketAddr> {
    addresses
        .iter()
        .map(|address| std::net::SocketAddr::new(address.parse().unwrap(), 80))
        .collect()
}

fn private(url: &str, address: &str, setting: Option<&'static str>) -> Refused {
    Refused {
        url: url.to_owned(),
        rule: EgressRule::PrivateAddress,
        address: Some(address.parse().unwrap()),
        setting,
    }
}

const PRIVATE_RANGES: Option<&str> = Some("egress.private_ranges");

/// Covers: story:4-8
#[rstest]
#[case::loopback(&["127.0.0.1"], "127.0.0.1")]
#[case::metadata(&["169.254.169.254"], "169.254.169.254")]
#[case::a_private_answer_after_a_public_one(&["93.184.215.14", "10.0.0.1"], "10.0.0.1")]
#[case::ipv6_unique_local(&["fd00::1"], "fd00::1")]
fn a_connection_that_may_try_a_private_address_is_refused_naming_it(
    #[case] addresses: &[&str],
    #[case] refused: &str,
) {
    let url = "http://internal.test/";

    assert_eq!(
        judge_addresses(url, &sockets(addresses), &policy(&["*"], &[])),
        Err(private(url, refused, PRIVATE_RANGES))
    );
}

#[test]
fn a_connection_to_public_addresses_only_is_allowed() {
    assert_eq!(
        judge_addresses(
            "http://public.test/",
            &sockets(&["93.184.215.14", "2606:2800:21f:cb07:6820:80da:af6b:8b2c"]),
            &policy(&["*"], &[])
        ),
        Ok(())
    );
}

/// Covers: story:4-9
#[test]
fn a_connection_with_a_zone_id_is_refused_whatever_the_ranges() {
    let url = "http://link.test/";
    let session = policy(&["*"], &[])
        .with_private_ranges(&["fe80::/10"])
        .unwrap();
    let scoped = std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
        "fe80::1".parse().unwrap(),
        80,
        0,
        2,
    ));

    assert_eq!(
        judge_addresses(url, &[scoped], &session),
        Err(private(url, "fe80::1", None))
    );
}

/// Covers: story:4-9
#[rstest]
#[case::a_named_zone("http://[fe80::1%25eth0]/", "fe80::1")]
#[case::a_numbered_zone("http://[fe80::1%251]:8080/path", "fe80::1")]
fn a_url_with_a_zone_id_is_refused_as_a_private_address(#[case] url: &str, #[case] address: &str) {
    assert_eq!(
        verdict(judge(&fetch(url), &policy(&["*"], &[]))),
        Err(private(url, address, None))
    );
}

#[rstest]
#[case::an_address_in_a_listed_range(&["127.0.0.0/8"], "127.0.0.1", true)]
#[case::a_listed_address(&["10.0.0.7"], "10.0.0.7", true)]
#[case::an_address_outside_the_range(&["127.0.0.1/32"], "127.0.0.2", false)]
#[case::an_ipv6_range(&["fd00::/8"], "fd00::1", true)]
#[case::metadata_under_a_range_that_covers_it(&["169.254.0.0/16"], "169.254.169.254", false)]
#[case::metadata_listed_by_its_exact_address(&["169.254.169.254"], "169.254.169.254", true)]
#[case::metadata_listed_as_its_own_block(&["169.254.169.254/32"], "169.254.169.254", true)]
#[case::link_local_under_its_range(&["169.254.0.0/16"], "169.254.1.1", true)]
fn a_private_address_is_reached_only_when_a_range_lists_it(
    #[case] ranges: &[&str],
    #[case] address: &str,
    #[case] allowed: bool,
) {
    let url = "http://intranet.test/";
    let session = policy(&["*"], &[]).with_private_ranges(ranges).unwrap();

    assert_eq!(
        judge_addresses(url, &sockets(&[address]), &session),
        if allowed {
            Ok(())
        } else {
            Err(private(url, address, PRIVATE_RANGES))
        }
    );
}

#[rstest]
#[case::a_wildcard_name("*.intranet.test")]
#[case::a_prefix_too_long("10.0.0.0/33")]
#[case::no_prefix_length("10.0.0.0/")]
fn a_private_range_that_is_no_range_is_refused(#[case] entry: &str) {
    assert_eq!(
        policy(&["*"], &[])
            .with_private_ranges(&[entry])
            .unwrap_err(),
        InvalidEntry {
            setting: "egress.private_ranges",
            entry: entry.to_owned(),
        }
    );
}

/// Only a policy that refuses some host needs the browser's own
/// connections held by a proxy auto-config script.
#[rstest]
#[case::every_host_allowed(&["*"], &[], false)]
#[case::a_list_allowed(&["scope.test"], &[], true)]
#[case::every_host_but_one(&["*"], &["elsewhere.test"], true)]
fn only_a_policy_refusing_a_host_holds_the_browsers_own_connections(
    #[case] allowed: &[&str],
    #[case] blocked: &[&str],
    #[case] held: bool,
) {
    assert_eq!(policy(allowed, blocked).proxy_script().is_some(), held);
}

/// The Session's policy of a Run whose settings are `settings`.
fn of_the_run(settings: Value) -> Policy {
    let layers = Layers {
        run: run_values(settings),
        ..Layers::default()
    };
    Policy::of(&resolve(&layers, Product::Collie, Product::Collie).unwrap()).unwrap()
}

/// `egress.private_ranges` from the Run's settings (story 4-11): a range
/// opens the addresses it covers to any host, a host name opens its own
/// connections, not its subdomains', to private addresses, and neither
/// opens a cloud-metadata address, which only its exact address does.
/// Covers: story:4-11, setting:egress.private_ranges
#[rstest]
#[case::loopback_under_its_range("http://catalog.test/", "127.0.0.1", true)]
#[case::another_loopback_address_under_its_range("http://catalog.test/", "127.4.5.6", true)]
#[case::a_listed_hosts_private_answer("http://intranet.test/", "10.0.0.1", true)]
#[case::a_listed_hosts_name_in_capitals("http://INTRANET.test/", "10.0.0.1", true)]
#[case::an_unlisted_hosts_private_answer("http://elsewhere.test/", "10.0.0.1", false)]
#[case::a_listed_hosts_subdomain("http://www.intranet.test/", "10.0.0.1", false)]
#[case::metadata_through_a_listed_host("http://intranet.test/", "169.254.169.254", false)]
#[case::metadata_under_a_range_that_covers_it("http://catalog.test/", "169.254.169.254", false)]
#[case::metadata_listed_by_its_exact_address("http://catalog.test/", "100.100.100.200", true)]
fn the_runs_private_ranges_open_what_they_list_and_metadata_only_by_its_address(
    #[case] url: &str,
    #[case] address: &str,
    #[case] allowed: bool,
) {
    let session = of_the_run(json!({
        "egress": {
            "private_ranges": ["127.0.0.0/8", "169.254.0.0/16", "intranet.test", "100.100.100.200"],
        }
    }));

    assert_eq!(
        judge_addresses(url, &sockets(&[address]), &session),
        if allowed {
            Ok(())
        } else {
            Err(private(url, address, PRIVATE_RANGES))
        }
    );
}

/// `security.resolved_ip_check` off (story 4-17): the domain policy alone
/// applies, so the private and metadata answers the check refuses are
/// reached; on, its default, they are refused (positive control).
/// Covers: story:4-17, setting:security.resolved_ip_check
#[rstest]
#[case::loopback("127.0.0.1")]
#[case::a_private_network("10.0.0.1")]
#[case::metadata("169.254.169.254")]
fn with_the_resolved_address_check_off_private_answers_are_reached(#[case] address: &str) {
    let url = "http://internal.test/";
    let off = of_the_run(json!({ "security": { "resolved_ip_check": false } }));
    let on = of_the_run(json!({}));

    assert_eq!(
        (
            judge_addresses(url, &sockets(&[address]), &off),
            judge_addresses(url, &sockets(&[address]), &on),
        ),
        (Ok(()), Err(private(url, address, PRIVATE_RANGES)))
    );
}
