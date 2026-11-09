//! The page's metadata (spec #580, ticket #787): its title, description,
//! canonical URL, language, Open Graph and Twitter tags, its JSON-LD
//! blocks as JSON and its microdata items as the HTML Standard's JSON
//! conversion writes them.

use std::collections::BTreeMap;

use goodfetch_protocol::page::{MetaTag, Metadata, MicrodataItem, MicrodataValue};
use serde_json::json;

use crate::{Context, Document, Serialization, formats};

/// The document's URL after its redirects.
const URL: &str = "https://kites.test/journal/bridle";

/// The metadata of `html` at [`URL`], whose base URL is `base_url`.
fn metadata_at(html: &str, base_url: &str) -> Metadata {
    formats(
        &Document {
            html,
            url: URL,
            base_url,
            serialization: Serialization::Served,
        },
        &serde_json::from_value(json!({ "metadata": true })).unwrap(),
        &Context::default(),
    )
    .metadata
    .unwrap()
}

fn metadata(html: &str) -> Metadata {
    metadata_at(html, URL)
}

fn tag(property: &str, content: &str) -> MetaTag {
    MetaTag {
        property: property.to_owned(),
        content: content.to_owned(),
    }
}

fn text(value: &str) -> MicrodataValue {
    MicrodataValue::Text(value.to_owned())
}

fn item(
    types: &[&str],
    id: Option<&str>,
    properties: &[(&str, Vec<MicrodataValue>)],
) -> MicrodataItem {
    MicrodataItem {
        types: types.iter().map(|&token| token.to_owned()).collect(),
        id: id.map(ToOwned::to_owned),
        properties: properties
            .iter()
            .map(|(name, values)| ((*name).to_owned(), values.clone()))
            .collect(),
    }
}

/// Covers: story:6-6
#[test]
fn the_heads_title_description_canonical_language_and_tags_are_read_as_written() {
    let html = r#"<!doctype html><html lang="fr-FR"><head>
        <meta charset="utf-8">
        <title>  Bridles &amp; lines
          | Northwind </title>
        <meta name="Description" content="How to rig a bridle &mdash; step by step">
        <meta name="description" content="A second description">
        <link rel="stylesheet" href="/site.css">
        <link rel="canonical" href="../bridles?ref=1">
        <meta property="og:title" content="Bridles">
        <meta property="og:image" content="https://cdn.kites.test/a.jpg">
        <meta property="og:image" content="https://cdn.kites.test/b.jpg">
        <meta property="article:author" content="Ana">
        <meta name="og:site_name" content="Northwind">
        <meta name="twitter:card" content="summary_large_image">
        <meta property="twitter:site" content="@northwind">
        <meta name="Author" content="Ana">
        <meta name="author" content="Bo">
        <meta name="robots" content="index, follow">
        <meta http-equiv="refresh" content="300">
        </head><body><svg><title>An icon</title></svg><p>Text</p></body></html>"#;

    assert_eq!(
        metadata_at(html, "https://kites.test/journal/"),
        Metadata {
            title: Some("Bridles & lines | Northwind".to_owned()),
            description: Some("How to rig a bridle \u{2014} step by step".to_owned()),
            canonical: Some("https://kites.test/bridles?ref=1".to_owned()),
            lang: Some("fr-FR".to_owned()),
            open_graph: vec![
                tag("og:title", "Bridles"),
                tag("og:image", "https://cdn.kites.test/a.jpg"),
                tag("og:image", "https://cdn.kites.test/b.jpg"),
                tag("article:author", "Ana"),
                tag("og:site_name", "Northwind"),
            ],
            twitter: vec![
                tag("twitter:card", "summary_large_image"),
                tag("twitter:site", "@northwind"),
            ],
            json_ld: Vec::new(),
            microdata: Vec::new(),
            meta: BTreeMap::from([
                ("author".to_owned(), "Ana".to_owned()),
                (
                    "description".to_owned(),
                    "How to rig a bridle \u{2014} step by step".to_owned()
                ),
                ("robots".to_owned(), "index, follow".to_owned()),
            ]),
        }
    );
}

/// Covers: story:6-6
#[test]
fn a_document_with_no_metadata_answers_nulls_and_empty_lists() {
    assert_eq!(
        metadata("<p>Only text</p>"),
        Metadata {
            title: None,
            description: None,
            canonical: None,
            lang: None,
            open_graph: Vec::new(),
            twitter: Vec::new(),
            json_ld: Vec::new(),
            microdata: Vec::new(),
            meta: BTreeMap::new(),
        }
    );
}

/// Whatever only a body holds ends the head where it stands, `<body>` tag or
/// none, as the HTML Standard's parser ends it: a canonical link past it is
/// the body's. What a head holds, and what a `<template>` keeps to itself,
/// leaves the link in the head.
#[rstest::rstest]
#[case::past_a_paragraph(r#"<p>Text</p><link rel="canonical" href="/x">"#, None)]
#[case::past_a_paragraph_in_a_head_never_closed(
    r#"<head><title>T</title><p>Text</p><link rel="canonical" href="/x">"#,
    None
)]
#[case::past_text(r#"Text<link rel="canonical" href="/x">"#, None)]
#[case::past_a_body_tag(r#"<body></body><link rel="canonical" href="/x">"#, None)]
#[case::only_the_first_is_looked_at(
    r#"<p>Text</p><link rel="canonical" href="/x"><link rel="canonical" href="/y">"#,
    None
)]
#[case::past_what_a_head_holds(
    r#"<!doctype html> <!-- <p> --><title>T</title><style>p {}</style><script>"<p>"</script>
    <meta name="a" content="b"><link rel="canonical" href="/x">"#,
    Some("https://kites.test/x")
)]
#[case::past_a_closed_head(
    r#"<head></head><link rel="canonical" href="/x">"#,
    Some("https://kites.test/x")
)]
#[case::past_a_template(
    r#"<head><template><p>Text</p>Text</template><link rel="canonical" href="/x">"#,
    Some("https://kites.test/x")
)]
#[case::past_one_in_a_template(
    r#"<head><template><link rel="canonical" href="/t"></template><link rel="canonical" href="/x">"#,
    Some("https://kites.test/x")
)]
#[case::in_noscript(
    r#"<head><noscript><link rel="canonical" href="/n"></noscript></head>"#,
    None
)]
fn a_canonical_link_counts_only_while_the_head_stands(
    #[case] html: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(metadata(html).canonical.as_deref(), expected);
}

/// A canonical link in the page's body is not its canonical, as the
/// Frontier's evidence reads it (ADR 0120): the head's alone counts.
/// Covers: story:6-6
#[test]
fn a_canonical_link_in_the_body_is_not_the_pages_canonical() {
    let html = r#"<html><head><title>Bridles</title></head><body>
        <link rel="canonical" href="/elsewhere"></body></html>"#;

    assert_eq!(metadata(html).canonical, None);
}

/// Covers: story:6-6
#[test]
fn a_canonical_url_that_does_not_parse_is_null() {
    let html = r#"<head><link rel="canonical" href="http://[::1/"></head>"#;

    assert_eq!(metadata(html).canonical, None);
}

/// Covers: story:6-6
#[test]
fn every_json_ld_block_that_parses_is_its_json_in_document_order() {
    let html = r#"<html><head>
        <script type="application/ld+json">{ "@context": "https://schema.org", "@type": "Organization", "name": "Northwind &amp; Co" }</script>
        <script type="application/json">{ "not": "linked data" }</script>
        <script type=" Application/LD+JSON ">{ "broken": </script>
        </head><body>
        <script type="application/ld+json">
          [{ "@type": "BreadcrumbList", "itemListElement": [{ "@type": "ListItem", "position": 1, "name": "Kites" }] }]
        </script>
        </body></html>"#;

    assert_eq!(
        metadata(html).json_ld,
        [
            json!({ "@context": "https://schema.org", "@type": "Organization", "name": "Northwind &amp; Co" }),
            json!([{ "@type": "BreadcrumbList", "itemListElement": [{ "@type": "ListItem", "position": 1, "name": "Kites" }] }]),
        ]
    );
}

/// The HTML Standard's microdata, as its JSON conversion reads it: each
/// top-level item with its types, its id and its properties, an item a
/// property's value, each element's value read as the standard reads it,
/// and the properties an `itemref` names.
///
/// Covers: story:6-6
#[test]
fn microdata_items_are_read_as_the_html_standards_json_conversion() {
    let html = r#"<html><body>
        <div itemscope itemtype="https://schema.org/Product  https://schema.org/Thing" itemid="/products/kite-7" itemref="maker">
          <h1 itemprop="name">Delta   kite</h1>
          <img itemprop="image" src="images/kite.jpg" alt="">
          <a itemprop="url sameAs" href="/kites/7">Page</a>
          <meta itemprop="sku" content="K-7">
          <data itemprop="weight" value="0.4">400 g</data>
          <time itemprop="releaseDate" datetime="2026-04-01">April</time>
          <time itemprop="season">Spring</time>
          <div itemprop="offers" itemscope itemtype="https://schema.org/Offer">
            <span itemprop="price">49.90</span><span itemprop="priceCurrency">EUR</span>
          </div>
          <section><p>Made by hand: <span itemprop="material">ripstop</span></p></section>
        </div>
        <p id="maker">By <span itemprop="brand">Northwind</span></p>
        <div itemscope><span itemprop="note">untyped</span></div>
        </body></html>"#;

    assert_eq!(
        metadata_at(html, "https://kites.test/shop/").microdata,
        [
            item(
                &["https://schema.org/Product", "https://schema.org/Thing"],
                Some("https://kites.test/products/kite-7"),
                &[
                    ("name", vec![text("Delta   kite")]),
                    (
                        "image",
                        vec![text("https://kites.test/shop/images/kite.jpg")]
                    ),
                    ("url", vec![text("https://kites.test/kites/7")]),
                    ("sameAs", vec![text("https://kites.test/kites/7")]),
                    ("sku", vec![text("K-7")]),
                    ("weight", vec![text("0.4")]),
                    ("releaseDate", vec![text("2026-04-01")]),
                    ("season", vec![text("Spring")]),
                    (
                        "offers",
                        vec![MicrodataValue::Item(item(
                            &["https://schema.org/Offer"],
                            None,
                            &[
                                ("price", vec![text("49.90")]),
                                ("priceCurrency", vec![text("EUR")]),
                            ],
                        ))],
                    ),
                    ("material", vec![text("ripstop")]),
                    ("brand", vec![text("Northwind")]),
                ],
            ),
            item(&[], None, &[("note", vec![text("untyped")])]),
        ]
    );
}

/// Covers: story:6-6
#[test]
fn an_item_that_holds_itself_through_itemref_is_cut_where_it_loops() {
    let html = r#"<div itemscope><div id="y" itemprop="p" itemscope itemref="z"></div></div>
        <div id="z" itemprop="q" itemscope itemref="y"></div>"#;

    let z = item(&[], None, &[("p", vec![text("ERROR")])]);
    let y = item(&[], None, &[("q", vec![MicrodataValue::Item(z)])]);
    assert_eq!(
        metadata(html).microdata,
        [item(&[], None, &[("p", vec![MicrodataValue::Item(y)])])]
    );
}
