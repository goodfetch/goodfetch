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

use super::{InvalidEntry, Judgment, Policy, Refused, Request, RequestKind, judge};
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
#[case::the_entrys_parent("https://test/")]
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
    "[a-z][a-z0-9-]{0,8}[a-z0-9]"
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
