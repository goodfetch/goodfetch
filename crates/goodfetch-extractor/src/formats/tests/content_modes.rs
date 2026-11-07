//! How much of the page a call reads: the Markdown of the main content,
//! of the whole cleaned body, or "fit", the main content without its
//! link-dense blocks, image-only blocks and repeated navigation lists; the
//! HTML as it stands, after the Clean-up rules, or of the main content; and
//! the main content's text.

use goodfetch_protocol::page::{FormatsRequest, PageFormats};
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

use super::{URL, read};
use crate::{CleanUp, Context, Document, Serialization, formats};

/// A page with a site navigation list, an article holding a link-dense
/// paragraph, an image-only figure and the navigation list again, and a
/// footer.
const PAGE: &str = include_str!("../fixtures/navigation-and-related.html");

/// The article's paragraphs, which every reading of its content keeps.
const PARAGRAPHS: [&str; 4] = [
    "A kite line has to be strong enough for the pull of the kite in the strongest wind you fly in, and light enough that it does not sag between your hand and the kite on a calm day.",
    "Braided polyester is the usual choice for single-line kites: it stretches a little, which softens gusts, and it is cheap enough to replace when it frays on a rough reel.",
    "Dyneema lines are thinner for the same strength and barely stretch, so a stunt kite answers your hands at once, but they cut skin and other lines when they rub.",
    "Wherever you fly, wear gloves when the wind rises, and wind the line onto the reel under a little tension so that it does not bite into itself.",
];

fn markdown(html: &str, content: &str) -> String {
    read(html, json!({ "markdown": { "content": content } }))
        .markdown
        .unwrap()
}

/// Covers: story:6-2
#[test]
fn markdown_holds_the_main_content_by_default() {
    let asked = read(PAGE, json!({ "markdown": true })).markdown;

    assert_eq!(asked, Some(markdown(PAGE, "main")));
}

/// Covers: story:6-2
#[test]
fn fit_markdown_leaves_out_link_dense_and_image_only_blocks_and_a_repeated_navigation_list() {
    let main = markdown(PAGE, "main");

    let fit = markdown(PAGE, "fit");

    assert!(
        main.contains("Read also") && main.contains("(start here)"),
        "the main content holds the link-dense paragraph and the repeated list: {main:?}"
    );
    assert_eq!(
        fit,
        format!(
            "## Choosing a kite line\n\n{}\n\n{}\n\n{}\n\n{}\n",
            PARAGRAPHS[0], PARAGRAPHS[1], PARAGRAPHS[2], PARAGRAPHS[3]
        )
    );
    assert!(fit.len() < main.len());
}

/// The repeated navigation list is left out of "fit" whatever its links'
/// form, though the main-content step makes the article's links absolute
/// and the page's stay as written.
/// Covers: story:6-2
#[rstest]
#[case::document_relative("home.html", "shop.html", "journal.html")]
#[case::parent_relative("../", "../shop", "../journal")]
#[case::query_only("?page=1", "?page=2", "?page=3")]
#[case::scheme_relative("//kites.test/", "//kites.test/shop", "//kites.test/journal")]
fn fit_markdown_leaves_out_a_repeated_navigation_list_of_relative_links(
    #[case] home: &str,
    #[case] shop: &str,
    #[case] journal: &str,
) {
    let page = PAGE
        .replace("href=\"/\"", &format!("href=\"{home}\""))
        .replace("href=\"/shop\"", &format!("href=\"{shop}\""))
        .replace("href=\"/journal\"", &format!("href=\"{journal}\""));

    let fit = markdown(&page, "fit");

    assert_eq!(
        fit,
        format!(
            "## Choosing a kite line\n\n{}\n\n{}\n\n{}\n\n{}\n",
            PARAGRAPHS[0], PARAGRAPHS[1], PARAGRAPHS[2], PARAGRAPHS[3]
        )
    );
}

/// Covers: story:6-2
#[test]
fn full_markdown_holds_the_whole_body_and_not_the_head() {
    let full = markdown(PAGE, "full");

    for text in [
        "[Home](/)",
        "[Journal](/journal)",
        "# Choosing a kite line",
        PARAGRAPHS[3],
        "Copyright 2026 Northwind Kites. All rights reserved.",
    ] {
        assert!(full.contains(text), "{text:?} not in {full:?}");
    }
    assert!(!full.contains("Kite Journal | Northwind"), "{full:?}");
}

/// Covers: story:6-2
#[test]
fn full_markdown_is_read_after_the_clean_up_rules() {
    let html = PAGE.replace(
        "<footer>",
        "<div class=\"ad-slot\"><p>Buy a reel today.</p></div><footer>",
    );

    let full = with_ads_removed(&html, json!({ "markdown": { "content": "full" } }))
        .markdown
        .unwrap();

    assert!(
        !full.contains("Buy a reel today.") && full.contains("Copyright 2026"),
        "{full:?}"
    );
}

/// Covers: story:6-3
#[test]
fn html_holds_the_main_content_by_default() {
    let html = read(PAGE, json!({ "html": true })).html.unwrap();

    assert!(
        html.contains(PARAGRAPHS[0]) && !html.contains("Copyright 2026"),
        "{html:?}"
    );
    assert_eq!(
        Some(html),
        read(PAGE, json!({ "html": { "kind": "main" } })).html
    );
}

/// Covers: story:6-3
#[test]
fn raw_html_is_a_served_document_as_it_was_served() {
    let raw = read(PAGE, json!({ "html": { "kind": "raw" } })).html;

    assert_eq!(raw.as_deref(), Some(PAGE));
}

/// Covers: story:6-3
#[test]
fn raw_html_of_a_dom_is_its_light_tree_without_the_shadow_roots_serialized_beside_it() {
    let html = "<!DOCTYPE html><html><head><script>let a = '<template shadowrootmode=\"open\">';</script></head><body><div id=\"host\"><template shadowrootmode=\"open\"><p>Root text.</p><template shadowrootmode=\"closed\"><p>Nested.</p></template></template><span>Light text.</span></div><template><p>A page's own template.</p></template></body></html>";

    let raw = formats(
        &Document {
            html,
            url: URL,
            base_url: URL,
            serialization: Serialization::Dom,
        },
        &request(json!({ "html": { "kind": "raw" } })),
        &Context::default(),
    )
    .html;

    assert_eq!(
        raw.as_deref(),
        Some(
            "<!DOCTYPE html><html><head><script>let a = '<template shadowrootmode=\"open\">';</script></head><body><div id=\"host\"><span>Light text.</span></div><template><p>A page's own template.</p></template></body></html>"
        )
    );
}

/// The `.ad-slot` matcher of the live-document tests: a stand-in for area
/// 5's cosmetic matcher.
struct AdSlots;

impl CleanUp for AdSlots {
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
    )]
    fn remove(&self, document: &goodfetch_counted_work::ParsedHtml, _url: &str) -> bool {
        let ads = document.select(".ad-slot");
        let took = !ads.is_empty();
        ads.remove();
        took
    }
}

fn request(value: Value) -> FormatsRequest {
    serde_json::from_value(value).unwrap()
}

fn with_ads_removed(html: &str, request: Value) -> PageFormats {
    formats(
        &Document {
            html,
            url: URL,
            base_url: URL,
            serialization: Serialization::Served,
        },
        &self::request(request),
        &Context {
            clean_up: Some(&AdSlots),
            ..Context::default()
        },
    )
}

/// Covers: story:6-3, story:6-10
#[test]
fn cleaned_html_is_the_document_after_the_clean_up_rules_and_raw_html_before_them() {
    let html = PAGE.replace(
        "<footer>",
        "<div class=\"ad-slot\"><p>Buy a reel today.</p></div><footer>",
    );

    let raw = with_ads_removed(&html, json!({ "html": { "kind": "raw" } })).html;
    let cleaned = with_ads_removed(&html, json!({ "html": { "kind": "cleaned" } }))
        .html
        .unwrap();

    assert_eq!(raw, Some(html));
    assert!(
        !cleaned.contains("Buy a reel today.") && cleaned.contains("Copyright 2026"),
        "{cleaned:?}"
    );
}

/// Covers: story:6-3
#[test]
fn text_is_the_main_contents_text_a_paragraph_per_block() {
    let text = read(PAGE, json!({ "text": true })).text;

    assert_eq!(
        text.as_deref(),
        Some(
            format!(
                "Choosing a kite line\n\n{}\n\n{}\n\nRead also Setting the bridle, Reels and winders and Three knots for the field.\n\n{}\n\n{}\n\nHome (start here)\n\nShop (lines and reels)\n\nJournal (more articles)",
                PARAGRAPHS[0], PARAGRAPHS[1], PARAGRAPHS[2], PARAGRAPHS[3]
            )
            .as_str()
        )
    );
}

/// Phrasing elements, and an element the HTML Standard does not name, run
/// inside their paragraph; a ruby's readings are kept beside their base,
/// and its fallback parentheses, which a browser hides, are left out.
/// Covers: story:6-3
#[test]
fn text_keeps_a_paragraph_whole_across_the_phrasing_elements_inside_it() {
    let html = format!(
        "<!doctype html><html><head><title>Lines</title></head><body><article>\
         <h1>Choosing a kite line</h1><p>{}</p>\
         <p>Run <tt>reel --wind</tt> before you <big>fly</big>, read the \
         <acronym title=\"frequently asked questions\">FAQ</acronym> \
         <svg width=\"8\" height=\"8\"><circle r=\"4\"></circle></svg> on \
         <kite-term>bridles</kite-term> and tie the \
         <ruby>凧<rp>(</rp><rt>たこ</rt><rp>)</rp></ruby> knots.</p>\
         <p>{}</p></article></body></html>",
        PARAGRAPHS[0], PARAGRAPHS[1]
    );

    let text = read(&html, json!({ "text": true })).text;

    assert_eq!(
        text.as_deref(),
        Some(
            format!(
                "Choosing a kite line\n\n{}\n\nRun reel --wind before you fly, read the FAQ on bridles and tie the 凧たこ knots.\n\n{}",
                PARAGRAPHS[0], PARAGRAPHS[1]
            )
            .as_str()
        )
    );
}

/// Covers: story:6-3, story:6-12
#[test]
fn a_document_with_no_main_content_answers_empty_main_html_and_text() {
    let read = read(
        include_str!("../fixtures/no-main-content.html"),
        json!({ "html": true, "text": true, "markdown": { "content": "fit" } }),
    );

    assert_eq!(
        (
            read.html.as_deref(),
            read.text.as_deref(),
            read.markdown.as_deref()
        ),
        (Some(""), Some(""), Some(""))
    );
}

/// An article whose third paragraph holds `plain` text then a link of
/// `link` text.
fn article_with(plain: &str, link: &str) -> String {
    format!(
        "<!doctype html><html><head><title>Lines</title></head><body><article>\
         <h1>Choosing a kite line</h1><p>{}</p><p>{}</p>\
         <p>{plain} <a href=\"/journal/more\">{link}</a></p>\
         <p>{}</p><p>{}</p></article></body></html>",
        PARAGRAPHS[0], PARAGRAPHS[1], PARAGRAPHS[2], PARAGRAPHS[3]
    )
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// A block is link-dense, and left out of "fit", exactly when more
    /// than half its text, white space aside, is in links: at, just under
    /// and just over one half.
    /// Covers: story:6-2
    #[test]
    fn a_block_is_left_out_of_fit_when_more_than_half_its_text_is_in_links(
        plain_length in 4_usize..40,
        offset in -2_isize..=2,
    ) {
        let link_length = plain_length.saturating_add_signed(offset).max(1);
        let plain = "p".repeat(plain_length);
        let link = "l".repeat(link_length);

        let main = markdown(&article_with(&plain, &link), "main");
        let fit = markdown(&article_with(&plain, &link), "fit");

        prop_assert!(main.contains(&plain), "{:?}", main);
        prop_assert_eq!(fit.contains(&plain), link_length <= plain_length, "{:?}", fit);
    }
}
