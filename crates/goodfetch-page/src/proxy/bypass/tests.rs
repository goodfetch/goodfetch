//! `proxy.bypass` read as Chromium's proxy bypass rules read it, against
//! a table of rules and URLs: hosts and their wildcards, a leading dot,
//! a scheme or a port the rule names, IP ranges and `<local>`.

use rstest::rstest;

use super::bypassed;

/// Covers: story:5-88, setting:proxy.bypass
#[rstest]
#[case::its_host("shop.example", "http://shop.example/cart", true)]
#[case::its_host_in_any_case("Shop.Example", "http://shop.example/", true)]
#[case::another_host("shop.example", "http://other.example/", false)]
#[case::a_subdomain_by_a_leading_dot(".example", "https://shop.example/", true)]
#[case::not_the_domain_itself_by_a_leading_dot(".example", "https://example/", false)]
#[case::a_wildcard("*.example", "http://a.b.example/", true)]
#[case::a_wildcard_inside("shop*.example", "http://shop-1.example/", true)]
#[case::every_host("*", "http://any.example/", true)]
#[case::its_port("shop.example:8080", "http://shop.example:8080/", true)]
#[case::another_port("shop.example:8080", "http://shop.example/", false)]
#[case::its_default_port("shop.example:443", "https://shop.example/", true)]
#[case::its_scheme("https://shop.example", "https://shop.example/", true)]
#[case::another_scheme("https://shop.example", "http://shop.example/", false)]
#[case::one_of_a_list("a.example, shop.example", "http://shop.example/", true)]
#[case::an_ip_address("10.0.0.7", "http://10.0.0.7/", true)]
#[case::an_ipv4_range("10.0.0.0/8", "http://10.200.1.2/", true)]
#[case::outside_an_ipv4_range("10.0.0.0/8", "http://11.0.0.1/", false)]
#[case::an_ipv6_range("[fd00::]/8", "http://[fd12::1]/", true)]
#[case::a_name_outside_a_range("10.0.0.0/8", "http://shop.example/", false)]
#[case::a_local_name("<local>", "http://intranet/", true)]
#[case::not_a_dotted_name_as_local("<local>", "http://shop.example/", false)]
#[case::loopback_kept_on_the_proxy("<-loopback>", "http://127.0.0.1/", false)]
#[case::nothing_from_an_empty_list("", "http://shop.example/", false)]
#[case::nothing_for_a_url_without_a_host("*", "data:text/plain,hi", false)]
fn a_url_is_bypassed_when_one_of_the_rules_matches_it(
    #[case] bypass: &str,
    #[case] url: &str,
    #[case] expected: bool,
) {
    assert_eq!(bypassed(Some(bypass), url), expected);
}

/// Covers: story:5-88
#[test]
fn no_url_is_bypassed_without_proxy_bypass() {
    assert!(!bypassed(None, "http://shop.example/"));
}
