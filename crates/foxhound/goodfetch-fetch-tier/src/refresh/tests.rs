//! The refresh-stub parser as a pure function (ADR 0118): the HTML
//! standard's shared declarative refresh steps over a table of values, and
//! which directive of a document counts, its header or its first `<meta>`
//! outside `<noscript>`; its fuzz target's seeds and crashers replayed.

use std::time::Duration;

use rstest::rstest;
use url::Url;

use super::{Refresh, directive, parse};

fn document() -> Url {
    Url::parse("http://site.test/stub/page").unwrap()
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "each case of the tables expects an `Option`, and writes a refresh in one call"
)]
fn refresh(seconds: u64, url: &str) -> Option<Refresh> {
    Some(Refresh {
        delay: Duration::from_secs(seconds),
        url: Url::parse(url).unwrap(),
    })
}

/// Covers: story:5-125
#[rstest]
#[case::an_instant_refresh("0; url=/target", refresh(0, "http://site.test/target"))]
#[case::a_url_in_capitals("5;URL=/target", refresh(5, "http://site.test/target"))]
#[case::a_quoted_url("5; url='/target'", refresh(5, "http://site.test/target"))]
#[case::a_url_cut_at_its_closing_quote(
    "0; url = \"/a b\"tail",
    refresh(0, "http://site.test/a%20b")
)]
#[case::a_comma("0,url=/target", refresh(0, "http://site.test/target"))]
#[case::whitespace_alone_between("5 url=/target", refresh(5, "http://site.test/target"))]
#[case::a_url_with_no_url_equals("0;/target", refresh(0, "http://site.test/target"))]
#[case::a_quoted_url_with_no_url_equals("0; '/target'", refresh(0, "http://site.test/target"))]
#[case::a_word_that_only_starts_like_url(
    "0; urx=/target",
    refresh(0, "http://site.test/stub/urx=/target")
)]
#[case::an_absolute_url("1; url=https://other.test/x", refresh(1, "https://other.test/x"))]
#[case::a_fraction_is_cut("1.9; url=/target", refresh(1, "http://site.test/target"))]
#[case::a_fraction_alone_is_instant(".5; url=/target", refresh(0, "http://site.test/target"))]
#[case::no_url_is_the_document("3", refresh(3, "http://site.test/stub/page"))]
#[case::surrounding_spaces("  7  ", refresh(7, "http://site.test/stub/page"))]
#[case::empty("", None)]
#[case::no_delay("url=/target", None)]
#[case::text_after_the_delay("0x; url=/target", None)]
#[case::a_url_that_does_not_parse("0; url=http://[::1", None)]
fn the_refresh_steps_read(#[case] input: &str, #[case] expected: Option<Refresh>) {
    assert_eq!(parse(input, &document()), expected);
}

fn headers(refreshes: &[&str]) -> Vec<(String, String)> {
    refreshes
        .iter()
        .map(|value| ("refresh".to_owned(), (*value).to_owned()))
        .collect()
}

fn meta(content: &str) -> String {
    format!(r#"<meta http-equiv="Refresh" content="{content}">"#)
}

/// Covers: story:5-125
#[rstest]
#[case::a_header(&["0; url=/target"], String::new(), refresh(0, "http://site.test/target"))]
#[case::the_first_header(&["2; url=/a", "0; url=/b"], String::new(), refresh(2, "http://site.test/a"))]
#[case::a_meta_in_the_head(
    &[],
    format!("<html><head>{}</head><body></body></html>", meta("5; url=/target")),
    refresh(5, "http://site.test/target")
)]
#[case::a_meta_in_the_body(
    &[],
    format!("<p>Moved.</p>{}", meta("0; url=/target")),
    refresh(0, "http://site.test/target")
)]
#[case::the_header_before_the_meta(
    &["200; url=/header"],
    meta("0; url=/meta"),
    refresh(200, "http://site.test/header")
)]
#[case::a_header_that_does_not_parse_leaves_the_meta(
    &["soon"],
    meta("0; url=/meta"),
    refresh(0, "http://site.test/meta")
)]
#[case::a_meta_inside_noscript_in_the_body(
    &[],
    format!("<body><noscript>{}</noscript><p>Moved.</p></body>", meta("0; url=/target")),
    None
)]
#[case::a_meta_inside_noscript_in_the_head(
    &[],
    format!("<head><noscript>{}</noscript></head>", meta("0; url=/target")),
    None
)]
#[case::the_first_meta_with_content(
    &[],
    format!(r#"<meta http-equiv="refresh">{}{}"#, meta("1; url=/first"), meta("0; url=/second")),
    refresh(1, "http://site.test/first")
)]
#[case::another_http_equiv(
    &[],
    r#"<meta http-equiv="content-type" content="0; url=/target">"#.to_owned(),
    None
)]
#[case::none(&[], "<p>A page.</p>".to_owned(), None)]
fn a_documents_directive_is_its_header_else_its_first_meta_outside_noscript(
    #[case] refreshes: &[&str],
    #[case] html: String,
    #[case] expected: Option<Refresh>,
) {
    assert_eq!(
        directive(&document(), &headers(refreshes), html.as_bytes(), true),
        expected
    );
}

/// Covers: story:5-125
#[test]
fn a_document_that_is_no_html_has_no_meta_directive() {
    let html = meta("0; url=/target");

    assert_eq!(directive(&document(), &[], html.as_bytes(), false), None);
}

/// Covers: story:5-125
#[rstest]
#[case::no_url("0", true)]
#[case::its_own_url("0; url=/stub/page", true)]
#[case::its_own_url_and_a_fragment("0; url=/stub/page#top", true)]
#[case::another_query("0; url=/stub/page?x", false)]
#[case::another_url("0; url=/target", false)]
fn a_refresh_to_the_documents_own_url_reloads(#[case] input: &str, #[case] reloads: bool) {
    let page = Url::parse("http://site.test/stub/page#here").unwrap();

    assert_eq!(parse(input, &page).unwrap().reloads(&page), reloads);
}

mod refresh_target {
    goodfetch_test_support::fuzz_replay!(crate::refresh::fuzz_refresh, "fuzz/refresh");
}
