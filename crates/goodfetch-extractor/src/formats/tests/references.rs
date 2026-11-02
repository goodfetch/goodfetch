//! The page's links, images and media (spec #580, ticket #786): each URL
//! resolved against the document's base URL by the WHATWG URL Standard, a
//! value that does not parse kept as written with no URL, read from the
//! document as it stands.
#![expect(
    clippy::disallowed_types,
    reason = "ADR 0140: a test's own parse, on no Shared path"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a test's own parse, on no Shared path"
)]

use goodfetch_protocol::page::{Image, Link, Media, MediaKind, PageFormats};
use proptest::prelude::*;
use serde_json::{Value, json};
use url::Url;

use crate::{CleanUp, Context, Document, Serialization, formats};

/// The document's URL after its redirects.
const URL: &str = "https://kites.test/journal/after/bridle";

/// The formats `request` names, read from `html` at [`URL`] with
/// `base_url` as its base URL.
fn read_at(
    html: &str,
    base_url: &str,
    serialization: Serialization,
    request: Value,
) -> PageFormats {
    formats(
        &Document {
            html,
            url: URL,
            base_url,
            serialization,
        },
        &serde_json::from_value(request).unwrap(),
        &Context::default(),
    )
}

fn links(html: &str, base_url: &str) -> Vec<Link> {
    read_at(
        html,
        base_url,
        Serialization::Served,
        json!({ "links": true }),
    )
    .links
    .unwrap()
}

fn link(url: Option<&str>, href: &str, text: &str, rel: &[&str]) -> Link {
    Link {
        url: url.map(ToOwned::to_owned),
        href: href.to_owned(),
        text: text.to_owned(),
        rel: rel.iter().map(|&token| token.to_owned()).collect(),
    }
}

/// Covers: story:6-4
#[test]
fn links_resolve_against_the_base_url_and_after_a_redirect_in_document_order() {
    let html = r##"<!doctype html><html><body>
        <p><a href="reels">Reels</a> <a href="/shop">Shop</a> <a name="top">No href</a>
        <a href="https://elsewhere.test/a?b#c">Elsewhere</a> <a href="#knots">Knots</a></p>
        <map><area href="map/north" alt="North"></map>
        </body></html>"##;

    let after_redirect = links(html, URL);
    let under_base = links(html, "https://cdn.kites.test/assets/");

    assert_eq!(
        after_redirect,
        [
            link(
                Some("https://kites.test/journal/after/reels"),
                "reels",
                "Reels",
                &[]
            ),
            link(Some("https://kites.test/shop"), "/shop", "Shop", &[]),
            link(
                Some("https://elsewhere.test/a?b#c"),
                "https://elsewhere.test/a?b#c",
                "Elsewhere",
                &[]
            ),
            link(
                Some("https://kites.test/journal/after/bridle#knots"),
                "#knots",
                "Knots",
                &[]
            ),
            link(
                Some("https://kites.test/journal/after/map/north"),
                "map/north",
                "",
                &[]
            ),
        ]
    );
    assert_eq!(
        under_base
            .iter()
            .map(|link| link.url.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("https://cdn.kites.test/assets/reels"),
            Some("https://cdn.kites.test/shop"),
            Some("https://elsewhere.test/a?b#c"),
            Some("https://cdn.kites.test/assets/#knots"),
            Some("https://cdn.kites.test/assets/map/north"),
        ]
    );
}

/// Covers: story:6-4
#[test]
fn a_value_that_does_not_parse_is_kept_as_written_with_a_null_url() {
    let html = r#"<a href="http://[::1">Broken host</a><a href="https://exa mple.test/">Space</a><img src="http://:80/x.png" alt="Bad">"#;

    let read = read_at(
        html,
        URL,
        Serialization::Served,
        json!({ "links": true, "images": true }),
    );

    assert_eq!(
        read.links.unwrap(),
        [
            link(None, "http://[::1", "Broken host", &[]),
            link(None, "https://exa mple.test/", "Space", &[]),
        ]
    );
    assert_eq!(
        read.images.unwrap(),
        [Image {
            url: None,
            src: "http://:80/x.png".to_owned(),
            alt: Some("Bad".to_owned()),
        }]
    );
}

/// Covers: story:6-4
#[test]
fn character_references_are_decoded_as_the_html_standard_decodes_them() {
    let html = r#"<a href="/search?q=fish&amp;chips&copy=1&#x41;&#66;">Fish &amp; chips &copy 2026 &lt;b&gt;</a>"#;

    assert_eq!(
        links(html, URL),
        [link(
            Some("https://kites.test/search?q=fish&chips&copy=1AB"),
            "/search?q=fish&chips&copy=1AB",
            "Fish & chips \u{a9} 2026 <b>",
            &[],
        )]
    );
}

/// Covers: story:6-4
#[test]
fn a_links_text_is_its_descendants_text_white_space_collapsed_and_its_rel_its_tokens() {
    let html = "<a href=\"/x\" rel=\" nofollow\n noopener \">\n  Read <em>the <b>whole</b></em>\tguide\n</a><a href=\"/y\" rel=\"\">Y</a>";

    assert_eq!(
        links(html, URL),
        [
            link(
                Some("https://kites.test/x"),
                "/x",
                "Read the whole guide",
                &["nofollow", "noopener"]
            ),
            link(Some("https://kites.test/y"), "/y", "Y", &[]),
        ]
    );
}

/// Covers: story:6-4
#[test]
fn images_carry_their_alt_and_media_their_kind_and_type() {
    let html = r#"<img src="line.jpg" alt="A kite line"><img src="spacer.gif" alt=""><img src="/no-alt.png"><img alt="No src">
        <video src="flight.webm" poster="poster.jpg"></video>
        <video><source src="/clips/launch.mp4" type="video/mp4"><source src="launch.webm"></video>
        <audio src="wind.ogg"><source src="wind.mp3" type="audio/mpeg"></audio>"#;

    let read = read_at(
        html,
        URL,
        Serialization::Served,
        json!({ "images": true, "media": true }),
    );

    let image = |url: &str, src: &str, alt: Option<&str>| Image {
        url: Some(url.to_owned()),
        src: src.to_owned(),
        alt: alt.map(ToOwned::to_owned),
    };
    assert_eq!(
        read.images.unwrap(),
        [
            image(
                "https://kites.test/journal/after/line.jpg",
                "line.jpg",
                Some("A kite line")
            ),
            image(
                "https://kites.test/journal/after/spacer.gif",
                "spacer.gif",
                Some("")
            ),
            image("https://kites.test/no-alt.png", "/no-alt.png", None),
        ]
    );
    let media = |url: &str, src: &str, kind, media_type: Option<&str>| Media {
        url: Some(url.to_owned()),
        src: src.to_owned(),
        kind,
        media_type: media_type.map(ToOwned::to_owned),
    };
    assert_eq!(
        read.media.unwrap(),
        [
            media(
                "https://kites.test/journal/after/flight.webm",
                "flight.webm",
                MediaKind::Video,
                None
            ),
            media(
                "https://kites.test/clips/launch.mp4",
                "/clips/launch.mp4",
                MediaKind::Video,
                Some("video/mp4")
            ),
            media(
                "https://kites.test/journal/after/launch.webm",
                "launch.webm",
                MediaKind::Video,
                None
            ),
            media(
                "https://kites.test/journal/after/wind.ogg",
                "wind.ogg",
                MediaKind::Audio,
                None
            ),
            media(
                "https://kites.test/journal/after/wind.mp3",
                "wind.mp3",
                MediaKind::Audio,
                Some("audio/mpeg")
            ),
        ]
    );
}

/// The cosmetic rule of the tests: every `.ad-slot` element out.
struct AdSlots;

impl CleanUp for AdSlots {
    fn remove(&self, html: &str, _url: &str) -> String {
        let document = dom_query::Document::from(html);
        document.select(".ad-slot").remove();
        document.html().to_string()
    }
}

/// Covers: story:6-4, story:6-10
#[test]
fn links_read_the_document_as_it_stands_its_shadow_roots_included_and_no_rule_touches_them() {
    let html = r#"<!doctype html><html><body>
        <article><p>A kite line has to be strong enough for the pull of the kite in the strongest wind, and light enough not to sag on a calm day. <a href="/journal/reels">Reels</a></p></article>
        <div class="ad-slot"><a href="https://ads.test/buy">Buy now</a></div>
        <div id="host"><template shadowrootmode="closed"><a href="/inside">Inside</a></template></div>
        </body></html>"#;

    let read = formats(
        &Document {
            html,
            url: URL,
            base_url: URL,
            serialization: Serialization::Dom,
        },
        &serde_json::from_value(json!({ "links": true, "markdown": true })).unwrap(),
        &Context {
            clean_up: Some(&AdSlots),
            ..Context::default()
        },
    );

    let hrefs: Vec<String> = read
        .links
        .unwrap()
        .into_iter()
        .map(|link| link.href)
        .collect();
    assert_eq!(hrefs, ["/journal/reels", "https://ads.test/buy", "/inside"]);
    assert!(!read.markdown.unwrap().contains("Buy now"));
}

/// An ad's image and video, which a cosmetic rule removes, are left out of
/// `images` and `media`, which read the document after the Clean-up rules
/// (ADR 0047); the ad's link stays in `links`.
/// Covers: story:6-4, story:6-10
#[test]
fn images_and_media_read_the_document_after_the_clean_up_rules_and_links_before_them() {
    let html = r#"<!doctype html><html><body>
        <article><p>A kite line has to be strong enough for the pull of the kite. <img src="/images/line.jpg" alt="A line"></p><video src="/films/launch.webm"></video></article>
        <div class="ad-slot"><a href="https://ads.test/buy"><img src="https://ads.test/banner.png" alt="Buy now"></a><video src="https://ads.test/spot.webm"></video></div>
        </body></html>"#;

    let read = formats(
        &Document {
            html,
            url: URL,
            base_url: URL,
            serialization: Serialization::Dom,
        },
        &serde_json::from_value(json!({ "links": true, "images": true, "media": true })).unwrap(),
        &Context {
            clean_up: Some(&AdSlots),
            ..Context::default()
        },
    );

    assert_eq!(
        (
            read.links
                .unwrap()
                .into_iter()
                .map(|link| link.href)
                .collect::<Vec<_>>(),
            read.images
                .unwrap()
                .into_iter()
                .map(|image| image.src)
                .collect::<Vec<_>>(),
            read.media
                .unwrap()
                .into_iter()
                .map(|media| media.src)
                .collect::<Vec<_>>(),
        ),
        (
            vec!["https://ads.test/buy".to_owned()],
            vec!["/images/line.jpg".to_owned()],
            vec!["/films/launch.webm".to_owned()],
        )
    );
}

/// HTML-escapes `value` for a double-quoted attribute.
fn attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;")
}

/// A URL-ish value: the characters URLs, bases and broken ones are
/// written with, and any other.
fn value() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        "[a-zA-Z0-9./?#:@%\\[\\]\\\\ _~+=-]{0,12}",
        Just("http://".to_owned()),
        Just("https://kites.test/".to_owned()),
        Just("//cdn.test".to_owned()),
        Just("../".to_owned()),
        Just("[::1".to_owned()),
        Just("mailto:a@b.test".to_owned()),
        Just("data:,x".to_owned()),
        "[^\\x00\\r]{0,6}",
    ];
    proptest::collection::vec(piece, 0..5).prop_map(|pieces| pieces.concat())
}

/// Raw markup of an attribute's or a text's character references, decoded
/// or not.
fn references() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        "[a-zA-Z0-9 =;#x]{0,6}",
        Just("&".to_owned()),
        Just("&amp;".to_owned()),
        Just("&amp".to_owned()),
        Just("&copy".to_owned()),
        Just("&notin;".to_owned()),
        Just("&noti".to_owned()),
        Just("&#".to_owned()),
        Just("&#x".to_owned()),
        Just("&#x1F600;".to_owned()),
        Just("&#128;".to_owned()),
        Just("&#0;".to_owned()),
        Just("&#xD800;".to_owned()),
        Just("&#99999999999;".to_owned()),
        Just("&lt;".to_owned()),
        Just("&quot;".to_owned()),
        Just("\u{e9}\u{1f600}".to_owned()),
        Just("\u{feff}".to_owned()),
    ];
    proptest::collection::vec(piece, 0..8).prop_map(|pieces| pieces.concat())
}

/// White space collapsed: one space between words, none at either end.
fn collapsed(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Whatever a link's `href`, an image's `src` and the base URL, each
    /// URL is what the `url` crate resolves, `null` when it does not
    /// parse, and nothing panics.
    /// Covers: story:6-4
    #[test]
    fn resolution_equals_the_url_crates(href in value(), src in value(), base in value()) {
        let html = format!(
            "<a href=\"{}\">A</a><img src=\"{}\" alt=\"\"><video src=\"{}\"></video>",
            attribute(&href),
            attribute(&src),
            attribute(&src)
        );
        let resolved = |value: &str| {
            Url::parse(&base).ok().and_then(|base| base.join(value).ok()).map(String::from)
        };

        let read = read_at(&html, &base, Serialization::Served, json!({ "links": true, "images": true, "media": true }));

        let links = read.links.unwrap();
        let images = read.images.unwrap();
        let media = read.media.unwrap();
        prop_assert_eq!((links.len(), images.len(), media.len()), (1, 1, 1));
        prop_assert_eq!(&links[0].href, &href);
        prop_assert_eq!(links[0].url.clone(), resolved(&href));
        prop_assert_eq!(&images[0].src, &src);
        prop_assert_eq!(images[0].url.clone(), resolved(&src));
        prop_assert_eq!(media[0].url.clone(), resolved(&src));
    }

    /// A link's `href` and text, written with any character references,
    /// are what an html5ever tree holds for them, its text white space
    /// collapsed.
    /// Covers: story:6-4
    #[test]
    fn character_references_decode_as_html5ever_decodes_them(href in references(), text in references()) {
        let html = format!("<a href=\"{}\">{text}</a>", href.replace('"', "&quot;"));
        let tree = scraper::Html::parse_fragment(&html);
        let selector = scraper::Selector::parse("a").unwrap();
        let expected = tree.select(&selector).next().unwrap();

        let links = links(&html, URL);

        prop_assert_eq!(links.len(), 1);
        prop_assert_eq!(Some(links[0].href.as_str()), expected.attr("href"));
        prop_assert_eq!(&links[0].text, &collapsed(&expected.text().collect::<String>()));
    }
}
