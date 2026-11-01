//! A page's `rel=canonical` as a pure function of its document and its
//! headers (ADR 0120): the head's first canonical link, resolved against
//! the base URL, else a `Link` header's, resolved against the page's URL.

use rstest::rstest;
use url::Url;

use super::canonical;

#[rstest]
#[case::a_link_in_the_head(
    r#"<html><head><link rel="canonical" href="/docs/"></head><body></body></html>"#,
    &[],
    Some("http://site.test/docs/")
)]
#[case::the_first_of_two(
    r#"<head><link rel="Canonical" href="a"><link rel="canonical" href="b"></head>"#,
    &[],
    Some("http://site.test/base/a")
)]
#[case::among_other_link_types(
    r#"<head><link rel="alternate canonical" href="https://other.test/x"></head>"#,
    &[],
    Some("https://other.test/x")
)]
#[case::a_link_in_the_body_is_none(
    r#"<head></head><body><link rel="canonical" href="/x"></body>"#,
    &[],
    None
)]
#[case::the_header(
    "<p>No head link</p>",
    &[("link", r#"<https://site.test/a>; rel="preload", </page/>; rel="canonical""#)],
    Some("http://site.test/page/")
)]
#[case::the_head_before_the_header(
    r#"<head><link rel="canonical" href="/head"></head>"#,
    &[("link", "</header>; rel=canonical")],
    Some("http://site.test/head")
)]
#[case::none("<p>Nothing</p>", &[("content-type", "text/html")], None)]
fn a_page_names_its_canonical(
    #[case] html: &str,
    #[case] headers: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let base = Url::parse("http://site.test/base/").unwrap();
    let url = Url::parse("http://site.test/page/one").unwrap();

    assert_eq!(
        canonical(html, &headers, &base, &url),
        expected.map(|expected| Url::parse(expected).unwrap())
    );
}
