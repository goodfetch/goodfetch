use goodfetch_protocol::foxhound::crawl::SitemapRule;
use rstest::rstest;
use url::Url;

use std::fmt::Write as _;
use std::io::Write as _;

use flate2::Compression;
use flate2::write::GzEncoder;
use goodfetch_engine::response_limits::Limits;
use goodfetch_protocol::response_limits::LimitSetting;

use super::{Document, MAX_BYTES, MAX_ENTRIES, Refusal, Rules, Unreadable, parse, read_file};

fn locs(locs: &[&str]) -> Vec<String> {
    locs.iter().map(|loc| (*loc).to_owned()).collect()
}

/// A `urlset`'s `<loc>`s are its entries and a `sitemapindex`'s its
/// sitemaps, in file order, each trimmed, its entities and character
/// references read and a CDATA section kept as written; any other element
/// is ignored, a namespace prefix too.
///
/// Covers: story:6-203
#[rstest]
#[case::a_urlset(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n<url><loc>http://shop.test/a</loc><lastmod>2026-01-01</lastmod></url>\n<url><loc>\n  http://shop.test/b\n</loc></url>\n</urlset>\n",
    Document::Entries(locs(&["http://shop.test/a", "http://shop.test/b"])),
)]
#[case::an_index(
    "<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><sitemap><loc>http://shop.test/1.xml</loc></sitemap><sitemap><loc>/2.xml</loc></sitemap></sitemapindex>",
    Document::Sitemaps(locs(&["http://shop.test/1.xml", "/2.xml"])),
)]
#[case::entities_and_cdata(
    "<urlset><url><loc>http://shop.test/?a=1&amp;b=&#50;</loc></url><url><loc><![CDATA[http://shop.test/?c=3&d=4]]></loc></url></urlset>",
    Document::Entries(locs(&["http://shop.test/?a=1&b=2", "http://shop.test/?c=3&d=4"])),
)]
#[case::a_prefixed_namespace(
    "<s:urlset xmlns:s=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><s:url><s:loc>http://shop.test/a</s:loc></s:url><s:url><image:loc>http://shop.test/a.png</image:loc></s:url></s:urlset>",
    Document::Entries(locs(&["http://shop.test/a"])),
)]
#[case::an_empty_urlset("<urlset/>", Document::Entries(Vec::new()))]
fn a_sitemap_is_read_as_its_entries_or_its_sitemaps(#[case] xml: &str, #[case] read: Document) {
    assert_eq!(parse(xml.as_bytes()), Ok(read));
}

/// A file that is no sitemap, or whose XML breaks off, is unreadable,
/// with why.
///
/// Covers: story:6-203
#[rstest]
#[case::no_sitemap("<html><body>Not here</body></html>")]
#[case::broken("<urlset><url><loc>http://shop.test/a</loc></url>")]
#[case::not_xml("User-agent: *\nDisallow: /")]
fn a_file_that_is_no_sitemap_is_unreadable(#[case] text: &str) {
    assert!(
        matches!(parse(text.as_bytes()), Err(Unreadable(_))),
        "{text}"
    );
}

fn rules(rules: &[(&str, &str)]) -> Rules {
    Rules::of(
        &rules
            .iter()
            .map(|(pattern, label)| SitemapRule {
                pattern: (*pattern).to_owned(),
                label: (*label).to_owned(),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

/// The first rule whose pattern matches an entry's URL gives its Label;
/// one no rule matches has none.
///
/// Covers: story:6-203
#[rstest]
#[case::the_first_match_wins("http://shop.test/books/1", Some("book"))]
#[case::a_later_rule("http://shop.test/authors/2", Some("author"))]
#[case::no_rule("http://shop.test/about", None)]
fn the_first_matching_rule_gives_an_entry_its_label(
    #[case] url: &str,
    #[case] label: Option<&str>,
) {
    let rules = rules(&[
        ("**/books/*", "book"),
        ("**/books/**", "other"),
        ("**/authors/*", "author"),
    ]);

    assert_eq!(rules.label_of(&Url::parse(url).unwrap()), label);
}

/// A rule whose pattern the engine cannot match is refused.
#[test]
fn a_rule_the_engine_cannot_match_is_refused() {
    let refused = Rules::of(&[SitemapRule {
        pattern: "**/{a,{b,c}}".to_owned(),
        label: "page".to_owned(),
    }]);

    assert!(refused.is_err());
}

/// The URL the files below are read from.
const AT: &str = "http://shop.test/sitemap.xml";

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A `urlset` of `count` entries, or a sitemap index of `count` sitemaps.
fn listing(count: usize, index: bool) -> String {
    let (root, child) = if index {
        ("sitemapindex", "sitemap")
    } else {
        ("urlset", "url")
    };
    let mut text = format!("<{root}>");
    for number in 0..count {
        write!(
            text,
            "<{child}><loc>http://shop.test/{number}</loc></{child}>"
        )
        .unwrap();
    }
    write!(text, "</{root}>").unwrap();
    text
}

fn why(refusal: Result<Document, Refusal>) -> String {
    match refusal {
        Err(Refusal::Unreadable(Unreadable(why))) => why,
        other => panic!("not unreadable: {other:?}"),
    }
}

/// A sitemap is read alike plain and gzip-compressed, a `.gz` file being
/// told by its first bytes, whatever its URL or type.
///
/// Covers: story:6-206
#[test]
fn a_gzip_sitemap_reads_as_its_plain_form() {
    let plain = listing(2, false);

    assert_eq!(
        (
            read_file(AT, plain.as_bytes(), Limits::default()).ok(),
            read_file(AT, &gzip(plain.as_bytes()), Limits::default()).ok(),
        ),
        (
            Some(Document::Entries(locs(&[
                "http://shop.test/0",
                "http://shop.test/1"
            ]))),
            Some(Document::Entries(locs(&[
                "http://shop.test/0",
                "http://shop.test/1"
            ]))),
        )
    );
}

/// sitemaps.org's 50,000 URLs bound a sitemap and an index alike: one
/// listing 50,000 is read whole, one listing 50,001 is unreadable, with
/// the limit named.
///
/// Covers: story:6-206
#[rstest]
#[case::a_urlset(false)]
#[case::an_index(true)]
fn a_sitemap_past_fifty_thousand_urls_is_unreadable(#[case] index: bool) {
    let within = read_file(
        AT,
        listing(MAX_ENTRIES, index).as_bytes(),
        Limits::default(),
    );
    let past = read_file(
        AT,
        listing(MAX_ENTRIES + 1, index).as_bytes(),
        Limits::default(),
    );

    let listed = match within {
        Ok(Document::Entries(locs) | Document::Sitemaps(locs)) => locs.len(),
        other => panic!("not read: {other:?}"),
    };
    assert_eq!(
        (listed, why(past)),
        (
            50_000,
            "it lists more than sitemaps.org's 50,000 URLs".to_owned()
        )
    );
}

/// sitemaps.org's 50 MB (52,428,800 bytes) bound a sitemap uncompressed,
/// plain or gzip-compressed: one byte past them it is unreadable, with the
/// limit named.
///
/// Covers: story:6-206
#[rstest]
#[case::plain(false)]
#[case::gzip(true)]
fn a_sitemap_past_fifty_megabytes_uncompressed_is_unreadable(#[case] compressed: bool) {
    let mut text = listing(1, false).into_bytes();
    text.resize(MAX_BYTES + 1, b' ');
    let body = if compressed { gzip(&text) } else { text };
    // A ratio no padding passes, so the size is what is judged.
    let limits = Limits {
        max_decompression_ratio: 1_000_000,
        ..Limits::default()
    };

    assert_eq!(
        why(read_file(AT, &body, limits)),
        "it is larger than sitemaps.org's 50 MB (52,428,800 bytes) uncompressed"
    );
}

/// A decompression bomb, a small `.gz` file that decodes to far more bytes
/// per byte than `network.max_decompression_ratio` allows, is refused
/// within ADR 0067's limits, naming the setting.
///
/// Covers: story:6-206, setting:network.max_decompression_ratio
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[test]
fn a_gzip_bomb_is_refused_by_the_decompression_ratio() {
    let bomb = gzip(&vec![b' '; 10 * 1024 * 1024]);

    let refused = read_file(AT, &bomb, Limits::default());

    match refused {
        Err(Refusal::Limit(exceeded)) => assert_eq!(
            (exceeded.url, exceeded.setting, exceeded.limit),
            (AT.to_owned(), LimitSetting::MaxDecompressionRatio, 100)
        ),
        other => panic!("not refused by a limit: {other:?}"),
    }
}

/// A file that opens as gzip but is no gzip stream, or is cut, is
/// unreadable.
///
/// Covers: story:6-206
#[rstest]
#[case::corrupt(vec![0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 0xff, 1, 2, 3])]
#[case::cut(gzip(listing(2, false).as_bytes())[..20].to_vec())]
fn a_gzip_file_that_does_not_decode_is_unreadable(#[case] body: Vec<u8>) {
    assert_eq!(
        why(read_file(AT, &body, Limits::default())),
        "its gzip stream does not decode"
    );
}

/// The sitemap fuzz target's committed seeds and past crashers replay on
/// every OS (ADR 0067).
mod sitemap_target {
    goodfetch_test_support::fuzz_replay!(crate::sitemap::fuzz_sitemap, "fuzz/sitemap");
}
