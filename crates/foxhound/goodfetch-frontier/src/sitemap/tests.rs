#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::foxhound::crawl::SitemapRule;
use rstest::rstest;
use url::Url;

use std::fmt::Write as _;
use std::io::Write as _;

use flate2::Compression;
use flate2::write::GzEncoder;
use goodfetch_engine::response_limits::Limits;
use goodfetch_protocol::response_limits::LimitSetting;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::egress::Policy;
use goodfetch_protocol::foxhound::crawl::SitemapSeed;
use goodfetch_protocol::foxhound::frontier::Enqueued;
use goodfetch_protocol::run::Completeness;
use rusqlite::Connection;

use super::{
    ChangeFreq, Document, MAX_BYTES, MAX_ENTRIES, ReadOptions, Refusal, Rules, SitemapDate,
    SitemapEntry, SitemapIndexEntry, Unreadable, admit_seed, parse, parse_w3c_datetime, read,
    read_file, reports,
};
use crate::bounds::{self, Bounds, completeness_reasons};
use crate::invariants::Invariants;
use crate::scope::{self, Patterns, Scope, Strategy};

fn entries(locs: &[&str]) -> Document {
    Document::Entries {
        entries: locs.iter().map(|loc| SitemapEntry::loc(*loc)).collect(),
        is_news: false,
    }
}

fn index_entries(locs: &[&str]) -> Vec<SitemapIndexEntry> {
    locs.iter()
        .map(|loc| SitemapIndexEntry::loc(*loc))
        .collect()
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
    Document::Entries {
        entries: vec![
            SitemapEntry {
                loc: "http://shop.test/a".to_owned(),
                lastmod: parse_w3c_datetime("2026-01-01"),
                news_publication_date: None,
                changefreq: None,
                priority: None,
            },
            SitemapEntry::loc("http://shop.test/b"),
        ],
        is_news: false,
    },
)]
#[case::an_index(
    "<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><sitemap><loc>http://shop.test/1.xml</loc></sitemap><sitemap><loc>/2.xml</loc></sitemap></sitemapindex>",
    Document::Sitemaps(index_entries(&["http://shop.test/1.xml", "/2.xml"])),
)]
#[case::entities_and_cdata(
    "<urlset><url><loc>http://shop.test/?a=1&amp;b=&#50;</loc></url><url><loc><![CDATA[http://shop.test/?c=3&d=4]]></loc></url></urlset>",
    entries(&["http://shop.test/?a=1&b=2", "http://shop.test/?c=3&d=4"]),
)]
#[case::a_prefixed_namespace(
    "<s:urlset xmlns:s=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><s:url><s:loc>http://shop.test/a</s:loc></s:url><s:url><image:loc>http://shop.test/a.png</image:loc></s:url></s:urlset>",
    entries(&["http://shop.test/a"]),
)]
#[case::an_empty_urlset("<urlset/>", entries(&[]))]
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

/// Whether the read refused the file as unreadable; the reason's words
/// are no contract: every unreadable sitemap reaches the report as one
/// code.
fn unreadable(read: &Result<Document, Refusal>) -> bool {
    matches!(read, Err(Refusal::Unreadable(_)))
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
            Some(entries(&["http://shop.test/0", "http://shop.test/1"])),
            Some(entries(&["http://shop.test/0", "http://shop.test/1"])),
        )
    );
}

/// sitemaps.org's 50,000 URLs bound a sitemap and an index alike: one
/// listing 50,000 is read whole, one listing 50,001 is unreadable.
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
        Ok(Document::Entries { entries, .. }) => entries.len(),
        Ok(Document::Sitemaps(locs)) => locs.len(),
        other => panic!("not read: {other:?}"),
    };
    assert_eq!((listed, unreadable(&past)), (50_000, true), "{past:?}");
}

/// sitemaps.org's 50 MB (52,428,800 bytes) bound a sitemap uncompressed,
/// plain or gzip-compressed: one of exactly 50 MB is read, one byte past
/// them it is unreadable.
///
/// Covers: story:6-206
#[rstest]
#[case::plain(false)]
#[case::gzip(true)]
fn a_sitemap_past_fifty_megabytes_uncompressed_is_unreadable(#[case] compressed: bool) {
    // A ratio no padding passes, so the size is what is judged.
    let limits = Limits {
        max_decompression_ratio: 1_000_000,
        ..Limits::default()
    };
    let read = |size: usize| {
        let mut text = listing(1, false).into_bytes();
        text.resize(size, b' ');
        let body = if compressed { gzip(&text) } else { text };
        read_file(AT, &body, limits)
    };

    let (within, past) = (read(MAX_BYTES), read(MAX_BYTES + 1));

    assert_eq!(
        (
            matches!(&within, Ok(Document::Entries { entries, .. }) if entries.len() == 1),
            unreadable(&past)
        ),
        (true, true),
        "{within:?} {past:?}"
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
    let read = read_file(AT, &body, Limits::default());

    assert!(unreadable(&read), "{read:?}");
}

/// The sitemap fuzz target's committed seeds and past crashers replay on
/// every OS (ADR 0067).
mod sitemap_target {
    goodfetch_test_support::fuzz_replay!(crate::sitemap::fuzz_sitemap, "fuzz/sitemap");
}

/// A run file holding the Frontier's sitemaps, in memory, under the domain
/// policy that allows every host, of the same-hostname Scope; the work item
/// of each fetch it adds is numbered from 1.
struct Sitemaps {
    connection: Connection,
    /// The Run's invariants, built once as it starts.
    invariants: Invariants,
    policy: Policy,
    /// The URL of each fetch added, in order.
    added: Vec<String>,
}

impl Sitemaps {
    /// A run file whose Run recorded `bounds`.
    fn bounded(bounds: &Bounds) -> Self {
        let connection = Connection::open_in_memory().unwrap();
        for statement in crate::identity::TABLES
            .statements
            .iter()
            .chain(scope::TABLES.statements)
            .chain(bounds::TABLES.statements)
            .chain(crate::rules::TABLES.statements)
            .chain(super::TABLES.statements)
        {
            connection.execute(statement, []).unwrap();
        }
        bounds.record(&connection).unwrap();
        let scope = Scope::new(
            Strategy::SameHostname,
            Patterns::default(),
            Patterns::default(),
        );
        Self {
            connection,
            invariants: Invariants::new(scope, bounds.clone()).unwrap(),
            policy: Policy::new(&["*"], &[] as &[&str]).unwrap(),
            added: Vec::new(),
        }
    }

    /// Admits the sitemap seed at `url`; answers its fetch's work item.
    fn seed(&mut self, url: &str) -> String {
        let seed = SitemapSeed {
            sitemap: url.to_owned(),
            rules: Vec::new(),
        };
        let added = &mut self.added;
        let answer = admit_seed(
            &self.connection,
            &self.invariants,
            &self.policy,
            &seed,
            |url| {
                added.push(url.to_string());
                Ok(added.len().to_string())
            },
        )
        .unwrap();
        match answer {
            Enqueued::Added { id, .. } => id,
            refused @ Enqueued::NotAdded { .. } => panic!("the seed is admitted: {refused:?}"),
        }
    }

    /// Reads, as what the fetch of the work item `id` gave, a sitemap
    /// index that nests `nested`; answers the work item and the URL of
    /// each fetch that added.
    fn read_index(&mut self, id: &str, nested: &[&str]) -> Vec<(String, String)> {
        let before = self.added.len();
        let added = &mut self.added;
        read(
            &self.connection,
            &self.invariants,
            &self.policy,
            id,
            &Document::Sitemaps(index_entries(nested)),
            ReadOptions::default(),
            &CountedWork::default(),
            |url, _| {
                added.push(url.to_string());
                Ok(added.len().to_string())
            },
        )
        .unwrap();
        (before..self.added.len())
            .map(|at| ((at + 1).to_string(), self.added[at].clone()))
            .collect()
    }

    /// Each sitemap the first seed could not read: its URL and its code.
    fn failures(&self) -> Vec<(String, String)> {
        reports(&self.connection).unwrap()[0]
            .failures
            .iter()
            .map(|failure| (failure.url.clone(), failure.code.clone()))
            .collect()
    }

    /// What the bounds cut, by setting, with the Completeness each gives.
    fn cut(&self) -> Vec<(String, Completeness)> {
        completeness_reasons(&self.connection)
            .unwrap()
            .into_iter()
            .map(|reason| (reason.reason.code, reason.completeness))
            .collect()
    }
}

fn pair(id: &str, url: &str) -> (String, String) {
    (id.to_owned(), url.to_owned())
}

/// A sitemap index's nested sitemaps are fetched down to
/// `crawl.max_sitemap_nesting` levels under their seed, 5 by default: one
/// nested a level past it is not fetched, the seed's report keeps its URL
/// with `max_sitemap_nesting`, and the bound leaves the Run `incomplete`.
///
/// Covers: story:6-204, setting:crawl.max_sitemap_nesting
#[test]
fn a_sitemap_nested_past_the_limit_is_not_fetched_and_its_seeds_report_names_it() {
    let mut sitemaps = Sitemaps::bounded(&Bounds::default());
    let mut id = sitemaps.seed("http://shop.test/sitemap.xml");
    let mut fetched = Vec::new();

    for level in 1..=6 {
        let nested = format!("http://shop.test/level-{level}.xml");
        let added = sitemaps.read_index(&id, &[&nested]);
        fetched.push(added.iter().map(|(_, url)| url.clone()).collect::<Vec<_>>());
        if let Some((next, _)) = added.into_iter().next() {
            id = next;
        }
    }

    assert_eq!(
        (fetched, sitemaps.failures(), sitemaps.cut()),
        (
            vec![
                vec!["http://shop.test/level-1.xml".to_owned()],
                vec!["http://shop.test/level-2.xml".to_owned()],
                vec!["http://shop.test/level-3.xml".to_owned()],
                vec!["http://shop.test/level-4.xml".to_owned()],
                vec!["http://shop.test/level-5.xml".to_owned()],
                Vec::new(),
            ],
            vec![pair("http://shop.test/level-6.xml", "max_sitemap_nesting")],
            vec![(
                "crawl.max_sitemap_nesting".to_owned(),
                Completeness::Incomplete
            )],
        )
    );
}

/// A sitemap index does not fetch a sitemap it names on a host other than
/// its seed's, though the domain policy allows that host: the seed's
/// report keeps its URL with `out_of_scope`. One on the seed's host is
/// fetched.
///
/// Covers: story:6-205
#[test]
fn an_index_does_not_fetch_a_sitemap_it_names_on_another_host() {
    let mut sitemaps = Sitemaps::bounded(&Bounds::default());
    let seed = sitemaps.seed("http://shop.test/sitemap.xml");

    let added = sitemaps.read_index(&seed, &["http://partner.test/sitemap.xml", "/own.xml"]);

    assert_eq!(
        (added, sitemaps.failures()),
        (
            vec![pair("2", "http://shop.test/own.xml")],
            vec![pair("http://partner.test/sitemap.xml", "out_of_scope")],
        )
    );
}

/// A robots.txt seed's `Sitemap:` lines cross-submit a sitemap on another
/// host, which is fetched; the sitemaps that one names are fetched on its
/// own host, the partner's, and on the seed's, as sitemaps.org places an
/// index's sitemaps, and not on a third host, since only the robots.txt
/// cross-submits.
///
/// Covers: story:6-205
#[test]
fn a_robots_txt_seed_fetches_the_sitemap_it_cross_submits_on_another_host() {
    let mut sitemaps = Sitemaps::bounded(&Bounds::default());
    let seed = sitemaps.seed("http://shop.test/robots.txt");

    let submitted = sitemaps.read_index(&seed, &["http://partner.test/sitemap.xml"]);
    let nested = sitemaps.read_index(
        "2",
        &[
            "http://partner.test/more.xml",
            "http://shop.test/own.xml",
            "http://third.test/sitemap.xml",
        ],
    );

    assert_eq!(
        (submitted, nested, sitemaps.failures()),
        (
            vec![pair("2", "http://partner.test/sitemap.xml")],
            vec![
                pair("3", "http://partner.test/more.xml"),
                pair("4", "http://shop.test/own.xml"),
            ],
            vec![pair("http://third.test/sitemap.xml", "out_of_scope")],
        )
    );
}

/// A nested sitemap's fetch counts against the crawl's bounds as a page's
/// does, its nesting level as its depth: one past `crawl.max_depth` or
/// `crawl.max_requests` is not fetched, the seed's report keeps its URL
/// with the bound's name, and the Run is `incomplete`.
///
/// Covers: story:6-204
#[rstest]
/// Covers: setting:crawl.max_requests
#[case::max_requests(
    Bounds { max_requests: Some(1), ..Bounds::default() },
    vec![pair("2", "http://shop.test/a.xml")],
    Vec::new(),
    vec![pair("http://shop.test/b.xml", "max_requests"), pair("http://shop.test/c.xml", "max_requests")],
    "crawl.max_requests",
)]
/// Covers: setting:crawl.max_depth
#[case::max_depth(
    Bounds { max_depth: Some(1), ..Bounds::default() },
    vec![pair("2", "http://shop.test/a.xml"), pair("3", "http://shop.test/b.xml")],
    Vec::new(),
    vec![pair("http://shop.test/c.xml", "max_depth")],
    "crawl.max_depth",
)]
fn a_nested_sitemaps_fetch_counts_against_the_crawls_bounds(
    #[case] bounds: Bounds,
    #[case] first: Vec<(String, String)>,
    #[case] second: Vec<(String, String)>,
    #[case] failures: Vec<(String, String)>,
    #[case] setting: &str,
) {
    let mut sitemaps = Sitemaps::bounded(&bounds);
    let seed = sitemaps.seed("http://shop.test/sitemap.xml");

    let added = sitemaps.read_index(&seed, &["/a.xml", "/b.xml"]);
    let deeper = sitemaps.read_index("2", &["/c.xml"]);

    assert_eq!(
        (added, deeper, sitemaps.failures(), sitemaps.cut()),
        (
            first,
            second,
            failures,
            vec![(setting.to_owned(), Completeness::Incomplete)]
        )
    );
}

/// Sitemaps parse `<lastmod>`, `<news:publication_date>`, `<changefreq>`, and
/// `<priority>`, and detect Google News sitemaps by namespace or `<news:news>`
/// elements (ADR 0139; issue 1852).
///
/// Covers: setting:crawl.sitemap.read_lastmod
#[test]
fn sitemap_parses_lastmod_news_changefreq_and_priority() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"
        xmlns:news="http://www.google.com/schemas/sitemap-news/0.9">
  <url>
    <loc>http://shop.test/breaking</loc>
    <lastmod>2026-11-28T08:00:00Z</lastmod>
    <changefreq>hourly</changefreq>
    <priority>0.8</priority>
    <news:news>
      <news:publication_date>2026-11-28T09:15:30+01:00</news:publication_date>
    </news:news>
  </url>
  <url>
    <lastmod>2026-11-20</lastmod>
    <loc>http://shop.test/date-only</loc>
    <changefreq>weekly</changefreq>
    <priority>0.3</priority>
  </url>
</urlset>"#;

    let Document::Entries { entries, is_news } = parse(xml.as_bytes()).expect("valid news sitemap")
    else {
        panic!("expected urlset");
    };
    assert!(is_news, "news namespace must mark sitemap as news");
    assert_eq!(entries.len(), 2);

    let first = &entries[0];
    assert_eq!(first.loc, "http://shop.test/breaking");
    assert_eq!(
        first.lastmod,
        Some(SitemapDate {
            ms: parse_w3c_datetime("2026-11-28T08:00:00Z").unwrap().ms,
            date_only: false,
        })
    );
    // 09:15:30+01:00 == 08:15:30Z
    assert_eq!(
        first.news_publication_date,
        parse_w3c_datetime("2026-11-28T08:15:30Z")
    );
    assert_eq!(first.changefreq, Some(ChangeFreq::Hourly));
    assert_eq!(first.priority, Some(0.8));

    let second = &entries[1];
    assert_eq!(second.loc, "http://shop.test/date-only");
    assert_eq!(second.lastmod.map(|d| d.date_only), Some(true));
    assert_eq!(second.changefreq, Some(ChangeFreq::Weekly));
    assert_eq!(second.priority, Some(0.3));
}

/// An entry dated more than `crawl.continuous.max_entry_age` ago is refused
/// with `max_entry_age` when `crawl.sitemap.read_lastmod` is on, and a
/// sitemap's `changed` flag triggers on a new entry or an entry updated since
/// `previous_fetched_at` (ADR 0139; issue 1852).
///
/// Covers: setting:crawl.continuous.max_entry_age, setting:crawl.sitemap.read_lastmod
#[expect(
    clippy::too_many_lines,
    reason = "tests max_entry_age filtering, unchanged re-read, and updated-lastmod change detection in one sequence"
)]
#[test]
fn sitemap_read_filters_max_entry_age_and_detects_changes() {
    let mut sitemaps = Sitemaps::bounded(&Bounds::default());
    let seed = SitemapSeed {
        sitemap: "http://shop.test/sitemap.xml".to_owned(),
        rules: vec![SitemapRule {
            pattern: "**".to_owned(),
            label: "article".to_owned(),
        }],
    };
    let id = match admit_seed(
        &sitemaps.connection,
        &sitemaps.invariants,
        &sitemaps.policy,
        &seed,
        |url| {
            sitemaps.added.push(url.to_string());
            Ok(sitemaps.added.len().to_string())
        },
    )
    .unwrap()
    {
        Enqueued::Added { id, .. } => id,
        other @ Enqueued::NotAdded { .. } => panic!("expected added: {other:?}"),
    };

    let now_ms = parse_w3c_datetime("2026-11-28T12:00:00Z").unwrap().ms;
    let day_ms = 86_400_000_u64;
    let doc = Document::Entries {
        entries: vec![
            SitemapEntry {
                loc: "http://shop.test/fresh".to_owned(),
                lastmod: Some(SitemapDate {
                    ms: now_ms - day_ms,
                    date_only: false,
                }),
                news_publication_date: None,
                changefreq: None,
                priority: None,
            },
            SitemapEntry {
                loc: "http://shop.test/stale".to_owned(),
                lastmod: Some(SitemapDate {
                    ms: now_ms - 31 * day_ms,
                    date_only: false,
                }),
                news_publication_date: None,
                changefreq: None,
                priority: None,
            },
        ],
        is_news: false,
    };

    let first = read(
        &sitemaps.connection,
        &sitemaps.invariants,
        &sitemaps.policy,
        &id,
        &doc,
        ReadOptions {
            read_lastmod: true,
            max_entry_age: Some((now_ms, 30 * day_ms)),
            previous_fetched_at: None,
        },
        &CountedWork::default(),
        |url, _| {
            sitemaps.added.push(url.to_string());
            Ok(sitemaps.added.len().to_string())
        },
    )
    .unwrap();
    assert!(first.changed, "first read added a fresh entry");
    assert_eq!(sitemaps.added.len(), 2, "only seed + fresh entry admitted");
    assert_eq!(sitemaps.added[1], "http://shop.test/fresh");

    // Re-reading the exact same sitemap with no new entry and no newer lastmod is unchanged.
    let second = read(
        &sitemaps.connection,
        &sitemaps.invariants,
        &sitemaps.policy,
        &id,
        &doc,
        ReadOptions {
            read_lastmod: true,
            max_entry_age: Some((now_ms + day_ms, 30 * day_ms)),
            previous_fetched_at: Some(now_ms),
        },
        &CountedWork::default(),
        |url, _| {
            sitemaps.added.push(url.to_string());
            Ok(sitemaps.added.len().to_string())
        },
    )
    .unwrap();
    assert!(
        !second.changed,
        "unchanged sitemap must report changed = false"
    );

    // Updating an existing entry's <lastmod> past previous_fetched_at counts as changed.
    let updated_doc = Document::Entries {
        entries: vec![SitemapEntry {
            loc: "http://shop.test/fresh".to_owned(),
            lastmod: Some(SitemapDate {
                ms: now_ms + 60_000,
                date_only: false,
            }),
            news_publication_date: None,
            changefreq: None,
            priority: None,
        }],
        is_news: false,
    };
    let third = read(
        &sitemaps.connection,
        &sitemaps.invariants,
        &sitemaps.policy,
        &id,
        &updated_doc,
        ReadOptions {
            read_lastmod: true,
            max_entry_age: Some((now_ms + 120_000, 30 * day_ms)),
            previous_fetched_at: Some(now_ms),
        },
        &CountedWork::default(),
        |url, _| {
            sitemaps.added.push(url.to_string());
            Ok(sitemaps.added.len().to_string())
        },
    )
    .unwrap();
    assert!(
        third.changed,
        "entry updated since previous_fetched_at must report changed = true"
    );
}
