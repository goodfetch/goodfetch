use goodfetch_protocol::foxhound::crawl::SitemapRule;
use rstest::rstest;
use url::Url;

use super::{Document, Rules, Unreadable, parse};

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
