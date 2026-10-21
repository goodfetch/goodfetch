use rstest::rstest;
use url::Url;

use super::url_key;

fn key(text: &str) -> String {
    url_key(&Url::parse(text).unwrap())
}

/// Two URLs the key's fixed rules make one request (ADR 0120).
///
/// Covers: story:6-213
#[rstest]
#[case::an_encoded_tilde_lowercase_and_the_tilde("http://a.test/%7e", "http://a.test/~")]
#[case::an_encoded_tilde_uppercase_and_the_tilde("http://a.test/%7E", "http://a.test/~")]
#[case::lowercase_and_uppercase_hexadecimal_digits("http://a.test/%c3%a9", "http://a.test/%C3%A9")]
#[case::encoded_unreserved_characters_in_the_query(
    "http://a.test/?q=%41%2d%5f",
    "http://a.test/?q=A-_"
)]
#[case::the_fragment("http://a.test/page#a", "http://a.test/page#b")]
#[case::a_fragment_and_none("http://a.test/page#a", "http://a.test/page")]
#[case::utm_parameters(
    "http://a.test/p?utm_source=news&id=1&utm_medium=mail&utm_campaign=x",
    "http://a.test/p?id=1"
)]
#[case::gclid_and_fbclid("http://a.test/p?gclid=1&id=2&fbclid=3", "http://a.test/p?id=2")]
#[case::parameter_order("http://a.test/p?b=2&a=1&c=3", "http://a.test/p?a=1&b=2&c=3")]
#[case::empty_pairs("http://a.test/p?&a=1&&b=2&", "http://a.test/p?a=1&b=2")]
#[case::an_empty_query("http://a.test/p?", "http://a.test/p")]
#[case::a_query_of_tracking_parameters_only("http://a.test/p?utm_source=x", "http://a.test/p")]
#[case::a_jsessionid_path_parameter(
    "http://a.test/cart;jsessionid=0123ABC?id=1",
    "http://a.test/cart?id=1"
)]
#[case::a_jsessionid_in_any_case("http://a.test/cart;JSESSIONID=0123ABC", "http://a.test/cart")]
#[case::a_jsessionid_among_other_path_parameters(
    "http://a.test/cart;v=2;jsessionid=0123ABC;w=3",
    "http://a.test/cart;v=2;w=3"
)]
fn the_key_makes_one_request_of(#[case] one: &str, #[case] other: &str) {
    assert_eq!(key(one), key(other));
}

/// Two URLs the key keeps apart, since RFC 3986 or HTTP says they may
/// differ, or since only evidence merges them (ADR 0120).
///
/// Covers: story:6-213
#[rstest]
#[case::an_encoded_slash_and_a_slash("http://a.test/a%2Fb", "http://a.test/a/b")]
#[case::a_plus_and_an_encoded_space("http://a.test/?q=a+b", "http://a.test/?q=a%20b")]
#[case::paths_in_another_case("http://a.test/APPLE", "http://a.test/apple")]
#[case::values_of_one_name_in_another_order("http://a.test/?a=1&a=2", "http://a.test/?a=2&a=1")]
#[case::another_path_parameter("http://a.test/cart;v=2", "http://a.test/cart")]
#[case::a_parameter_named_jsessionid_in_the_query(
    "http://a.test/cart?jsessionid=1",
    "http://a.test/cart"
)]
#[case::a_session_parameter_merged_only_on_evidence(
    "http://a.test/story?sid=1",
    "http://a.test/story?sid=2"
)]
#[case::a_trailing_slash("http://a.test/a", "http://a.test/a/")]
#[case::the_www_label("http://a.test/", "http://www.a.test/")]
#[case::another_scheme("http://a.test/", "https://a.test/")]
#[case::an_index_file("http://a.test/a/index.html", "http://a.test/a/")]
fn the_key_keeps_apart(#[case] one: &str, #[case] other: &str) {
    assert_ne!(key(one), key(other));
}

/// The key a URL gets, written out.
///
/// Covers: story:6-213
#[rstest]
#[case::every_rule_at_once(
    "http://a.test/%7e/a%2fb;JSessionID=1;v=2?utm_term=x&b=%41&&a=2&a=1#top",
    "http://a.test/~/a%2Fb;v=2?a=2&a=1&b=A"
)]
#[case::a_url_with_nothing_to_change("https://a.test:8443/a/b?c=d", "https://a.test:8443/a/b?c=d")]
fn the_key_of(#[case] url: &str, #[case] expected: &str) {
    assert_eq!(key(url), expected);
}
