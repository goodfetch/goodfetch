//! A page's `rel=canonical` as a pure function of its document and its
//! headers (ADR 0120): the head's first canonical link, resolved against
//! the base URL, else a `Link` header's, resolved against the page's URL.

use rstest::rstest;
use url::Url;

use super::canonical;

/// The fetch tier, reading a parsed tree's head, and `metadata.canonical`,
/// reading the document as it streams, name one canonical for one page.
#[rstest]
#[case::a_link_in_the_head(r#"<head><link rel="canonical" href="/x"></head>"#)]
#[case::no_head_tag_past_a_title(r#"<title>T</title><link rel="canonical" href="/x">"#)]
#[case::past_a_closed_head(r#"<head></head><link rel="canonical" href="/x"><body>"#)]
#[case::past_a_paragraph_and_no_body_tag(r#"<p>Text</p><link rel="canonical" href="/x">"#)]
#[case::past_a_paragraph_in_a_head(r#"<head><p>Text</p><link rel="canonical" href="/x"></head>"#)]
#[case::past_text(r#"Text<link rel="canonical" href="/x">"#)]
#[case::past_white_space_and_a_comment("\n <!-- <p> -->\t<link rel=\"canonical\" href=\"/x\">")]
#[case::past_a_body_tag(r#"<body></body><link rel="canonical" href="/x">"#)]
#[case::past_a_template(r#"<template><p>Text</p></template><link rel="canonical" href="/x">"#)]
#[case::in_a_template(r#"<head><template><link rel="canonical" href="/x"></template></head>"#)]
#[case::past_one_in_a_template(
    r#"<head><template><link rel="canonical" href="/t"></template><link rel="canonical" href="/x">"#
)]
#[case::in_noscript(r#"<head><noscript><link rel="canonical" href="/x"></noscript></head>"#)]
#[case::the_second_past_a_paragraph(
    r#"<link rel="canonical" href="/x"><p>Text</p><link rel="canonical" href="/y">"#
)]
fn the_fetch_tier_and_the_metadata_name_one_canonical(#[case] html: &str) {
    let base = "http://site.test/base/";
    let url = Url::parse(base).unwrap();
    let document = goodfetch_extractor::Document {
        html,
        url: base,
        base_url: base,
        serialization: goodfetch_extractor::Serialization::Served,
    };
    let formats = serde_json::from_value(serde_json::json!({ "metadata": true })).unwrap();

    let metadata = goodfetch_extractor::formats(
        &document,
        &formats,
        &goodfetch_extractor::Context::default(),
    )
    .metadata
    .unwrap();

    assert_eq!(
        canonical(
            html,
            &[],
            &url,
            &url,
            &goodfetch_counted_work::CountedWork::default()
        )
        .map(String::from),
        metadata.canonical
    );
}

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
#[case::a_link_past_a_paragraph_is_none(r#"<p>Text</p><link rel="canonical" href="/x">"#, &[], None)]
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
        canonical(
            html,
            &headers,
            &base,
            &url,
            &goodfetch_counted_work::CountedWork::default()
        ),
        expected.map(|expected| Url::parse(expected).unwrap())
    );
}
