//! One reading of a host for every module (ADRs 0014, 0042 and 0059), and
//! one rule for which listed domain covers it.

use rstest::rstest;

use super::{ListedDomains, NoHost, key, lookup_key};

#[rstest]
#[case::lower_case("Example.TEST", Some("example.test"))]
#[case::fully_qualified("example.test.", Some("example.test"))]
#[case::unicode("Bücher.Test.", Some("xn--bcher-kva.test"))]
#[case::punycode("xn--bcher-kva.test", Some("xn--bcher-kva.test"))]
#[case::ipv4_as_a_number("2130706433", Some("127.0.0.1"))]
#[case::ipv6_without_brackets("::1", Some("::1"))]
#[case::ipv6_with_brackets("[::1]", Some("::1"))]
#[case::no_host("exa mple.test", None)]
#[case::empty("", None)]
#[case::root_alone(".", None)]
fn a_host_is_keyed_as_the_url_parser_reads_it(#[case] host: &str, #[case] keyed: Option<&str>) {
    assert_eq!(key(host).as_deref(), keyed);
}

#[test]
fn text_that_is_no_host_is_looked_up_in_lower_case_without_its_trailing_dot() {
    assert_eq!(lookup_key("Exa Mple.Test."), "exa mple.test");
}

/// The listed domains of `tls.trusted_cas`'s shape: a domain, one of its
/// subdomains and an address.
fn listed() -> ListedDomains<u8> {
    let mut listed = ListedDomains::new();
    listed.insert("Example.test.", 1).unwrap();
    listed.insert("sub.example.test", 2).unwrap();
    listed.insert("127.0.0.1", 3).unwrap();
    listed
}

#[rstest]
#[case::the_domain("example.test", vec![("example.test", 1)])]
#[case::a_subdomain_of_both("a.sub.example.test", vec![("sub.example.test", 2), ("example.test", 1)])]
#[case::spelled_otherwise("A.SUB.Example.Test.", vec![("sub.example.test", 2), ("example.test", 1)])]
#[case::an_unlisted_domain("example.other", vec![])]
#[case::a_listed_address("127.0.0.1", vec![("127.0.0.1", 3)])]
#[case::no_host("exa mple.test", vec![])]
fn the_listed_domains_that_cover_a_host_are_the_host_and_its_parents_most_specific_first(
    #[case] host: &str,
    #[case] covering: Vec<(&str, u8)>,
) {
    let listed = listed();

    let found: Vec<(&str, u8)> = listed
        .covering(host)
        .into_iter()
        .map(|(domain, value)| (domain, *value))
        .collect();

    assert_eq!(
        (found, listed.most_specific(host).map(|(domain, _)| domain)),
        (
            covering.clone(),
            covering.first().map(|(domain, _)| *domain)
        )
    );
}

#[test]
fn an_address_is_covered_by_itself_alone_and_a_name_by_its_parents_too() {
    assert_eq!(
        (
            super::covering("127.0.0.1").collect::<Vec<_>>(),
            super::covering("a.example.test").collect::<Vec<_>>(),
        ),
        (
            vec!["127.0.0.1"],
            vec!["a.example.test", "example.test", "test"]
        )
    );
}

#[test]
fn an_entry_that_names_no_host_is_refused() {
    let mut listed = ListedDomains::new();

    assert_eq!(
        listed.insert("exa mple.test", ()),
        Err(NoHost("exa mple.test".to_owned()))
    );
}
