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

/// A page whose top-level item reaches, through `itemref`, a chain of
/// `levels` containers, each holding `width` items whose `itemprop` has
/// `names` tokens and whose `itemref` names the next container: each item
/// reaches every item of the next container, so the items multiply by
/// `width` a level, and their values by `names`, though each container is
/// written once.
fn shared_containers(levels: usize, width: usize, names: usize) -> String {
    let names = (0..names)
        .map(|name| format!("n{name}"))
        .collect::<Vec<_>>()
        .join(" ");
    let item = |level: usize| {
        format!(
            r#"<div itemprop="{names}" itemscope itemref="c{}"></div>"#,
            level + 1
        )
    };
    let containers = (0..levels)
        .map(|level| format!(r#"<div id="c{level}">{}</div>"#, item(level).repeat(width)))
        .collect::<Vec<_>>()
        .concat();
    format!(r#"<div itemscope itemref="c0"></div>{containers}"#)
}

/// What a reading wrote: its values other than `"ERROR"`, nested items
/// included, the bytes of their text, and its `"ERROR"` values.
#[derive(Debug, Default)]
struct Written {
    values: usize,
    text: usize,
    errors: usize,
}

impl Written {
    fn of(items: &[MicrodataItem]) -> Self {
        let mut written = Self::default();
        for item in items {
            written.add(item);
        }
        written
    }

    fn add(&mut self, item: &MicrodataItem) {
        for value in item.properties.values().flatten() {
            match value {
                MicrodataValue::Text(text) if text == "ERROR" => self.errors += 1,
                MicrodataValue::Text(text) => {
                    self.values += 1;
                    self.text += text.len();
                }
                MicrodataValue::Item(item) => {
                    self.values += 1;
                    self.add(item);
                }
            }
        }
    }
}

/// Covers: story:6-6
#[test]
fn items_that_share_their_containers_are_read_whole_within_the_budget() {
    let html = shared_containers(3, 2, 1);

    let written = Written::of(&metadata(&html).microdata);

    assert_eq!((written.values, written.errors), (14, 0), "{written:?}");
}

/// Covers: story:6-6
#[test]
fn a_page_whose_items_share_their_containers_writes_at_most_the_budget_and_error_past_it() {
    // 2.2 KB of HTML whose items, read whole, would be 2^19 - 1.
    let html = shared_containers(18, 2, 1);

    let written = Written::of(&metadata(&html).microdata);

    assert!(
        written.values <= 100_000 && written.text <= 4 << 20 && written.errors > 0,
        "{written:?}"
    );
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        // Each case past the budget reads up to the whole budget.
        cases: 32,
        ..goodfetch_test_support::proptest_config()
    })]

    /// Whatever the page's items share, and however many names each
    /// property has, one call writes at most the budget's values and text.
    /// Covers: story:6-6
    #[test]
    fn microdata_stays_within_the_budget_whatever_the_items_share(
        levels in 1_usize..=24,
        width in 1_usize..=3,
        names in 1_usize..=3,
    ) {
        let html = shared_containers(levels, width, names);

        let written = Written::of(&metadata(&html).microdata);

        proptest::prop_assert!(
            written.values <= 100_000 && written.text <= 4 << 20,
            "{:?}",
            written
        );
    }
}
