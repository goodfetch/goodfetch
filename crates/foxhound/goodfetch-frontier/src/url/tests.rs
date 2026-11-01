use goodfetch_protocol::foxhound::frontier::NotAddedReason;
use proptest::prelude::*;
use rstest::rstest;
use url::Url;

use encoding_rs::{Encoding, SHIFT_JIS, UTF_8, UTF_16LE, WINDOWS_1252};

use super::{document_base, parse, parse_link};

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

/// What browsers make of a URL as written, as the WHATWG URL Standard's
/// own examples give it.
///
/// Covers: story:6-211
#[rstest]
#[case::a_host_lowercased_and_its_default_port_dropped(
    "HTTP://Example.COM:80/a",
    "http://example.com/a"
)]
#[case::the_default_port_of_https("https://example.com:443", "https://example.com/")]
#[case::another_port_kept("http://example.com:8080/", "http://example.com:8080/")]
#[case::an_international_host_in_punycode(
    "http://bücher.example/",
    "http://xn--bcher-kva.example/"
)]
#[case::an_ipv4_host_in_hexadecimal("http://0xc0.0.2.1/", "http://192.0.2.1/")]
#[case::an_ipv4_host_as_one_number("http://3221225985/", "http://192.0.2.1/")]
#[case::an_ipv6_host_compressed("http://[0:0:0:0:0:0:0:1]:80/", "http://[::1]/")]
#[case::backslashes_as_slashes("http:\\\\example.com\\a\\b", "http://example.com/a/b")]
#[case::tabs_newlines_and_surrounding_spaces(
    " \thttp://exa\nmple.com/a\tb c\r\n ",
    "http://example.com/ab%20c"
)]
#[case::percent_encoding_kept_as_written(
    "http://example.com/%7e/~/%2F",
    "http://example.com/%7e/~/%2F"
)]
#[case::other_characters_percent_encoded(
    "http://example.com/é?q=é#é",
    "http://example.com/%C3%A9?q=%C3%A9#%C3%A9"
)]
#[case::dot_segments_removed("http://example.com/a/./b/../c", "http://example.com/a/c")]
fn a_url_parses_as_a_browser_parses_it(#[case] input: &str, #[case] parsed: &str) {
    assert_eq!(parse(input, None).map(String::from), Ok(parsed.to_owned()));
}

/// Covers: story:6-201, story:6-202, story:6-211
#[rstest]
#[case::a_relative_path("/catalog/1", NotAddedReason::InvalidUrl)]
#[case::a_bare_name("book/1", NotAddedReason::InvalidUrl)]
#[case::a_scheme_relative_url("//example.com/a", NotAddedReason::InvalidUrl)]
#[case::no_host("http://", NotAddedReason::InvalidUrl)]
#[case::a_space_in_the_host("http://exa mple.com/", NotAddedReason::InvalidUrl)]
#[case::an_unclosed_ipv6_host("http://[::1/", NotAddedReason::InvalidUrl)]
#[case::an_ipv4_host_out_of_range("http://256.256.256.256/", NotAddedReason::InvalidUrl)]
#[case::mail("mailto:books@example.com", NotAddedReason::UnsupportedScheme)]
#[case::ftp("ftp://example.com/a", NotAddedReason::UnsupportedScheme)]
#[case::a_script("javascript:void(0)", NotAddedReason::UnsupportedScheme)]
#[case::data("data:text/html,<p>x", NotAddedReason::UnsupportedScheme)]
#[case::a_file("file:///etc/hosts", NotAddedReason::UnsupportedScheme)]
fn a_url_the_crawl_cannot_fetch_says_why(#[case] input: &str, #[case] reason: NotAddedReason) {
    assert_eq!(parse(input, None), Err(reason));
}

/// Covers: story:6-211, story:6-231
#[rstest]
#[case::a_bare_name("book/1", "http://links.test/library/shelves/book/1")]
#[case::a_parent("../hall", "http://links.test/library/hall")]
#[case::an_absolute_path("/rooms/R-01", "http://links.test/rooms/R-01")]
#[case::a_query("?page=2", "http://links.test/library/shelves/?page=2")]
#[case::another_host("//other.test/a", "http://other.test/a")]
#[case::an_absolute_url("https://other.test/a", "https://other.test/a")]
#[case::a_backslash("\\rooms\\R-02", "http://links.test/rooms/R-02")]
fn a_relative_url_resolves_against_its_base(#[case] input: &str, #[case] resolved: &str) {
    let base = url("http://links.test/library/shelves/");

    assert_eq!(
        parse(input, Some(&base)).map(String::from),
        Ok(resolved.to_owned())
    );
}

/// Covers: story:6-231
#[rstest]
#[case::its_base_href(Some("/library/shelves/"), "http://links.test/library/shelves/")]
#[case::a_relative_base_href(Some("shelves/"), "http://links.test/guide/shelves/")]
#[case::no_base_element(None, "http://links.test/guide/")]
#[case::a_base_href_that_does_not_parse(Some("http://[::1/"), "http://links.test/guide/")]
#[case::a_script_base_href(Some("javascript:alert(1)"), "http://links.test/guide/")]
#[case::a_data_base_href(Some("data:text/html,x"), "http://links.test/guide/")]
fn a_documents_base_url_is_its_base_href_resolved_against_its_own_url(
    #[case] base_href: Option<&str>,
    #[case] base: &str,
) {
    assert_eq!(
        document_base("http://links.test/guide/", base_href).map(String::from),
        Some(base.to_owned())
    );
}

/// A link's query is percent-encoded in its document's encoding, as the
/// HTML standard's URL parsing encodes it, a character the encoding cannot
/// write as `&#<code point>;` percent-encoded; its path and fragment, and
/// every part of a UTF-16 document's links, in UTF-8 (ADR 0120). The bytes
/// are the encodings' own: 日本 is `93 FA 96 7B` in `Shift_JIS`, whose last
/// byte is `{`, which the query's percent-encode set leaves as it is; é is
/// `E9` and € `80` in windows-1252; 한 is U+D55C, 54620.
///
/// Covers: story:6-233
#[rstest]
#[case::a_shift_jis_query(SHIFT_JIS, "/search?q=日本", "http://links.test/search?q=%93%FA%96{")]
#[case::a_shift_jis_path_in_utf_8(
    SHIFT_JIS,
    "/日本?q=日本#日本",
    "http://links.test/%E6%97%A5%E6%9C%AC?q=%93%FA%96{#%E6%97%A5%E6%9C%AC"
)]
#[case::a_character_shift_jis_cannot_write(
    SHIFT_JIS,
    "?k=한",
    "http://links.test/library/shelves/?k=%26%2354620%3B"
)]
#[case::a_windows_1252_query(WINDOWS_1252, "/a?q=é€", "http://links.test/a?q=%E9%80")]
#[case::a_utf_16_document_in_utf_8(
    UTF_16LE,
    "/a?q=日本",
    "http://links.test/a?q=%E6%97%A5%E6%9C%AC"
)]
#[case::ascii_as_written(SHIFT_JIS, "/a?q=b%20c&d", "http://links.test/a?q=b%20c&d")]
fn a_links_query_is_encoded_in_its_documents_encoding(
    #[case] encoding: &'static Encoding,
    #[case] href: &str,
    #[case] resolved: &str,
) {
    let base = url("http://links.test/library/shelves/");

    assert_eq!(
        parse_link(href, &base, encoding).map(String::from),
        Ok(resolved.to_owned())
    );
}

#[test]
fn a_document_whose_own_url_does_not_parse_has_no_base_url() {
    assert_eq!(document_base("/guide/", Some("/library/")), None);
}

/// Inputs that look like URLs: a scheme, a host and path characters,
/// including those the Standard rewrites.
fn url_like() -> impl Strategy<Value = String> {
    "[ \t]?(https?|HTTP|ftp|mailto|javascript)?:?[/\\\\]{0,3}[a-zA-Z0-9.\\-\\[\\]:@%éü ]{0,20}[/\\\\?#a-z0-9.%~é ]{0,20}"
}

proptest! {
    /// On any input, the Frontier's parse is the `url` crate's, only
    /// refusing what it cannot fetch.
    ///
    /// Covers: story:6-211
    #[test]
    fn the_frontiers_parse_is_the_whatwg_parsers(input in prop_oneof![any::<String>(), url_like()]) {
        let expected = match Url::parse(&input) {
            Err(_) => Err(NotAddedReason::InvalidUrl),
            Ok(url) if ["http", "https"].contains(&url.scheme()) => Ok(String::from(url)),
            Ok(_) => Err(NotAddedReason::UnsupportedScheme),
        };
        prop_assert_eq!(parse(&input, None).map(String::from), expected);
    }

    /// On any input and base, the Frontier's resolution is the `url`
    /// crate's.
    ///
    /// Covers: story:6-211, story:6-231
    #[test]
    fn the_frontiers_resolution_is_the_whatwg_parsers(
        input in prop_oneof![any::<String>(), url_like()],
        base in prop_oneof![Just("http://links.test/library/shelves/"), Just("https://xn--bcher-kva.example:8443/a/b?c#d")],
    ) {
        let base = url(base);
        let expected = match base.join(&input) {
            Err(_) => Err(NotAddedReason::InvalidUrl),
            Ok(url) if ["http", "https"].contains(&url.scheme()) => Ok(String::from(url)),
            Ok(_) => Err(NotAddedReason::UnsupportedScheme),
        };
        prop_assert_eq!(parse(&input, Some(&base)).map(String::from), expected);
    }

    /// On any input and base, a UTF-8 document's link parses as an
    /// enqueue's URL does.
    ///
    /// Covers: story:6-233
    #[test]
    fn a_utf_8_documents_link_parses_as_an_enqueue_does(
        input in prop_oneof![any::<String>(), url_like()],
        base in prop_oneof![Just("http://links.test/library/shelves/"), Just("https://xn--bcher-kva.example:8443/a/b?c#d")],
    ) {
        let base = url(base);
        prop_assert_eq!(parse_link(&input, &base, UTF_8), parse(&input, Some(&base)));
    }
}
