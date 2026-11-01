#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use rstest::rstest;

use super::link_base;

/// Links in Notifications and a run's `console_url` name
/// `console.public_url` when it is set and the Console's own address
/// otherwise, and never carry what a URL's query or fragment could hold,
/// such as the sign-in link's token, so a forwarded link grants nothing.
///
/// Covers: story:12-9, setting:console.public_url
#[rstest]
#[case::the_consoles_own_address("http://127.0.0.1:7801/", None, "http://127.0.0.1:7801")]
#[case::the_public_url(
    "http://127.0.0.1:7801/",
    Some("https://console.example.test"),
    "https://console.example.test"
)]
#[case::a_public_url_with_a_path(
    "http://127.0.0.1:7801/",
    Some("https://example.test/goodfetch/"),
    "https://example.test/goodfetch"
)]
#[case::no_token_from_the_address(
    "http://127.0.0.1:7801/#token=abc",
    None,
    "http://127.0.0.1:7801"
)]
#[case::no_query_or_fragment_from_the_public_url(
    "http://127.0.0.1:7801/",
    Some("https://console.example.test/?token=abc#token=def"),
    "https://console.example.test"
)]
#[case::an_empty_public_url("http://127.0.0.1:7801/", Some(""), "http://127.0.0.1:7801")]
fn a_link_names_the_public_url_or_the_consoles_address_and_carries_no_credential(
    #[case] url: &str,
    #[case] public_url: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(link_base(url, public_url), expected);
}
