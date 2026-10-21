#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use rstest::rstest;

use super::DomainKeys;

/// Covers: story:3-48
#[rstest]
#[case::subdomains_of_one_site("https://www.shop.example.co.uk/a?b#c", "example.co.uk")]
#[case::the_registrable_domain_itself("http://example.com", "example.com")]
#[case::a_port_and_credentials("https://user:secret@Images.Example.COM:8443/x", "example.com")]
#[case::a_trailing_dot("https://www.example.com./", "example.com")]
#[case::a_private_entry("https://alice.github.io/page", "alice.github.io")]
#[case::another_site_on_the_same_private_entry("https://bob.github.io/page", "bob.github.io")]
#[case::a_name_outside_the_list("https://a.shop.test/page", "shop.test")]
#[case::a_public_suffix_alone("https://github.io/", "github.io")]
#[case::an_ipv4_address("http://127.0.0.1:8080/x", "127.0.0.1")]
#[case::an_ipv6_address("http://[::1]:8080/x", "::1")]
fn a_domain_key_is_the_registrable_domain_under_the_public_suffix_list(
    #[case] url: &str,
    #[case] key: &str,
) {
    assert_eq!(DomainKeys::default().key(url), key);
}

/// Covers: story:3-48, setting:crawl.domain_keys
#[rstest]
#[case::a_host_under_an_override("https://alice.github.io/a", "github")]
#[case::the_overridden_name_itself("https://github.io/", "github")]
#[case::the_longest_override_wins("https://api.shop.example.com/a", "api")]
#[case::a_host_beside_the_longest("https://www.shop.example.com/a", "shop")]
#[case::a_name_that_only_ends_alike("https://notgithub.io/", "notgithub.io")]
#[case::a_host_no_override_names("https://example.org/a", "example.org")]
fn an_override_gives_the_hosts_at_or_under_its_name_one_key(#[case] url: &str, #[case] key: &str) {
    let keys = DomainKeys::new([
        ("github.io".to_owned(), "github".to_owned()),
        ("Shop.Example.com".to_owned(), "shop".to_owned()),
        ("api.shop.example.com.".to_owned(), "api".to_owned()),
    ]);

    assert_eq!(keys.key(url), key);
}
