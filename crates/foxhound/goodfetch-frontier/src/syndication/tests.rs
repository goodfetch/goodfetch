use std::fmt::Write as _;
use std::io::Write as _;

use flate2::Compression;
use flate2::write::GzEncoder;
use goodfetch_counted_work::CountedWork;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::response_limits::Limits;
use goodfetch_protocol::foxhound::crawl::{SitemapRule, SyndicationSeed};
use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use goodfetch_protocol::foxhound::outcomes::{AttemptError, Ending, SkipReason};
use goodfetch_protocol::response_limits::LimitSetting;
use rstest::rstest;
use rusqlite::Connection;

use super::{
    FeedDocument, FeedEntry, MAX_FEED_BYTES, MAX_FEED_ENTRIES, Refusal, SyUpdatePeriod, Unreadable,
    admit_seed, ended, parse, parse_feed_date, read, read_file, reports,
};
use crate::bounds::{self, Bounds};
use crate::invariants::Invariants;
use crate::scope::{self, Patterns, Scope, Strategy};
use crate::sitemap::ReadOptions;

fn links(urls: &[&str]) -> FeedDocument {
    FeedDocument {
        entries: urls.iter().map(|url| FeedEntry::link(*url)).collect(),
        ..FeedDocument::default()
    }
}

#[rstest]
#[case::rss_2_0(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>News</title>
    <link>http://shop.test/</link>
    <item><title>One</title><link>http://shop.test/articles/1</link></item>
    <item><title>Two</title><link>
      http://shop.test/articles/2?a=1&amp;b=&#50;
    </link></item>
    <item><title>Three</title><link><![CDATA[http://shop.test/articles/3?x=1&y=2]]></link></item>
  </channel>
</rss>"#,
    links(&["http://shop.test/articles/1", "http://shop.test/articles/2?a=1&b=2", "http://shop.test/articles/3?x=1&y=2"]),
)]
#[case::rss_1_0_rdf(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/">
  <channel rdf:about="http://shop.test/"><title>RDF</title><link>http://shop.test/</link></channel>
  <item rdf:about="http://shop.test/rdf/1"><title>One</title><link>http://shop.test/articles/1</link></item>
  <item rdf:about="http://shop.test/articles/2"><title>Fallback about</title></item>
</rdf:RDF>"#,
    links(&["http://shop.test/articles/1", "http://shop.test/articles/2"]),
)]
#[case::atom_1_0(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom</title>
  <link rel="self" href="http://shop.test/atom.xml"/>
  <entry>
    <title>First</title>
    <link rel="self" href="http://shop.test/self/1"/>
    <link rel="alternate" type="text/html" href="http://shop.test/articles/1"/>
    <link rel="enclosure" href="http://shop.test/audio/1.mp3"/>
  </entry>
  <entry>
    <title>Second (implicit alternate)</title>
    <link href="/articles/2?a=1&amp;b=2"></link>
  </entry>
</feed>"#,
    links(&["http://shop.test/articles/1", "/articles/2?a=1&b=2"]),
)]
#[case::prefixed_atom(
    r#"<a:feed xmlns:a="http://www.w3.org/2005/Atom"><a:entry><a:link href="http://shop.test/p/1"/></a:entry></a:feed>"#,
    links(&["http://shop.test/p/1"]),
)]
#[case::empty_feed("<feed xmlns=\"http://www.w3.org/2005/Atom\"/>", links(&[]))]
fn parses_rss2_rss1_and_atom1_feeds(#[case] xml: &str, #[case] expected: FeedDocument) {
    assert_eq!(parse(xml.as_bytes()), Ok(expected));
}

#[rstest]
#[case::html("<html><body>Not a feed</body></html>")]
#[case::sitemap("<urlset><url><loc>http://shop.test/a</loc></url></urlset>")]
#[case::truncated("<rss version=\"2.0\"><channel><item><link>http://shop.test/a</link></item>")]
#[case::undeclared_entity(
    "<rss><channel><item><link>http://shop.test/?a=&bogus;</link></item></channel></rss>"
)]
#[case::plain_text("not xml at all")]
fn rejects_unreadable_feeds(#[case] text: &str) {
    assert!(
        matches!(parse(text.as_bytes()), Err(Unreadable(_))),
        "{text}"
    );
}

#[test]
fn reads_gzip_feed_and_enforces_size_and_entry_limits() {
    let xml = b"<feed xmlns=\"http://www.w3.org/2005/Atom\"><entry><link href=\"http://shop.test/a\"/></entry></feed>";
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(xml).expect("gzip encode");
    let gz = encoder.finish().expect("gzip finish");
    assert_eq!(
        read_file("http://shop.test/feed.xml.gz", &gz, Limits::default()).expect("reads gzip"),
        links(&["http://shop.test/a"])
    );

    // Over 50 MB uncompressed.
    let mut huge = Vec::with_capacity(MAX_FEED_BYTES + 16);
    huge.extend_from_slice(b"<feed>");
    huge.resize(MAX_FEED_BYTES + 1, b' ');
    assert!(matches!(
        read_file("http://shop.test/huge.xml", &huge, Limits::default()),
        Err(Refusal::Unreadable(_))
    ));

    // Over 50,000 entries.
    let mut many = String::from("<feed>");
    for i in 0..=MAX_FEED_ENTRIES {
        let _ = write!(many, "<entry><link href=\"http://shop.test/{i}\"/></entry>");
    }
    many.push_str("</feed>");
    assert!(matches!(
        parse(many.as_bytes()),
        Err(Unreadable(why)) if why.contains("50,000")
    ));

    // Decompression ratio bomb refused by Limits.
    let zeros = vec![b' '; 2 * 1024 * 1024];
    let mut bomb_enc = GzEncoder::new(Vec::new(), Compression::best());
    bomb_enc.write_all(&zeros).expect("gzip bomb");
    let bomb = bomb_enc.finish().expect("gzip bomb finish");
    let tight = Limits {
        max_decompression_ratio: 10,
        ..Limits::default()
    };
    assert!(matches!(
        read_file("http://shop.test/bomb.xml.gz", &bomb, tight),
        Err(Refusal::Limit(exceeded)) if exceeded.setting == LimitSetting::MaxDecompressionRatio
    ));
}

fn test_db() -> (Connection, Invariants, Policy) {
    let connection = Connection::open_in_memory().expect("in-memory sqlite");
    for tables in [
        crate::identity::TABLES,
        scope::TABLES,
        bounds::TABLES,
        crate::rules::TABLES,
        super::TABLES,
    ] {
        for sql in tables.statements {
            connection.execute_batch(sql).expect("create table");
        }
    }
    let scope = Scope::new(
        Strategy::SameHostname,
        Patterns::default(),
        Patterns::default(),
    );
    let invariants = Invariants::new(scope, Bounds::default()).expect("invariants");
    let policy = Policy::new(&["*"], &[] as &[&str]).expect("policy");
    (connection, invariants, policy)
}

#[test]
fn admits_entries_by_first_matching_rule_and_reports_unreadable_sources() {
    let (connection, invariants, policy) = test_db();
    let work = CountedWork::default();

    let good_seed = SyndicationSeed {
        syndication: "http://shop.test/feed.xml".to_owned(),
        rules: vec![
            SitemapRule {
                pattern: "http://shop.test/articles/*".to_owned(),
                label: "article".to_owned(),
            },
            SitemapRule {
                pattern: "http://shop.test/articles/1".to_owned(),
                label: "second_rule".to_owned(),
            },
        ],
    };
    let mut next_id = 0_u64;
    let mut alloc_id = || {
        next_id += 1;
        format!("req-{next_id}")
    };

    let Enqueued::Added { id: good_id, .. } =
        admit_seed(&connection, &invariants, &policy, &good_seed, |_| {
            Ok(alloc_id())
        })
        .expect("admit good seed")
    else {
        panic!("good seed must be added");
    };
    assert!(matches!(
        admit_seed(&connection, &invariants, &policy, &good_seed, |_| Ok(
            alloc_id()
        ))
        .expect("dup seed"),
        Enqueued::NotAdded {
            reason: NotAddedReason::Duplicate,
            ..
        }
    ));

    let doc = links(&[
        "http://shop.test/articles/1",
        "/articles/2",
        "http://shop.test/about",
        "http://other.test/articles/3",
    ]);
    let mut admitted = Vec::new();
    read(
        &connection,
        &invariants,
        &policy,
        &good_id,
        (&doc, ReadOptions::default()),
        &work,
        |url, label| {
            admitted.push((url.as_str().to_owned(), label.to_owned()));
            Ok(alloc_id())
        },
    )
    .expect("read good feed");

    assert_eq!(
        admitted,
        vec![
            (
                "http://shop.test/articles/1".to_owned(),
                "article".to_owned()
            ),
            (
                "http://shop.test/articles/2".to_owned(),
                "article".to_owned()
            ),
        ]
    );

    let sources = reports(&connection).expect("reports");
    assert_eq!(sources[0].url, "http://shop.test/feed.xml");
    assert_eq!(sources[0].read, 1);
    assert_eq!(sources[0].added, 2);
    assert_eq!(sources[0].not_added, 2);
    assert_eq!(sources[0].not_added_by_reason[0].name, "no_rule");
}

#[test]
fn reports_out_of_scope_entries_and_source_failures() {
    let (connection, invariants, policy) = test_db();
    let work = CountedWork::default();
    let mut next_id = 0_u64;
    let mut alloc_id = || {
        next_id += 1;
        format!("req-{next_id}")
    };
    let rules = vec![SitemapRule {
        pattern: "**".to_owned(),
        label: "article".to_owned(),
    }];

    let bad_seed = SyndicationSeed {
        syndication: "http://shop.test/broken.xml".to_owned(),
        rules: rules.clone(),
    };
    let Enqueued::Added { id: bad_id, .. } =
        admit_seed(&connection, &invariants, &policy, &bad_seed, |_| {
            Ok(alloc_id())
        })
        .expect("admit bad seed")
    else {
        panic!("bad seed must be added");
    };
    let details = serde_json::Map::from_iter([
        (
            "url".to_owned(),
            serde_json::Value::from("http://shop.test/broken.xml"),
        ),
        ("status".to_owned(), serde_json::Value::from(404)),
    ]);
    ended(
        &connection,
        &bad_id,
        &Ending::Failed {
            error: AttemptError {
                attempt: 1,
                code: "response_not_ok".to_owned(),
                message: "The feed could not be read: its XML breaks.".to_owned(),
                details,
            },
        },
    )
    .expect("record bad feed failure");

    let broad_seed = SyndicationSeed {
        syndication: "http://shop.test/broad.xml".to_owned(),
        rules,
    };
    let Enqueued::Added { id: broad_id, .. } =
        admit_seed(&connection, &invariants, &policy, &broad_seed, |_| {
            Ok(alloc_id())
        })
        .expect("admit broad seed")
    else {
        panic!("broad seed must be added");
    };
    read(
        &connection,
        &invariants,
        &policy,
        &broad_id,
        (
            &links(&["http://other.test/external"]),
            ReadOptions::default(),
        ),
        &work,
        |_, _| Ok(alloc_id()),
    )
    .expect("read broad feed");
    ended(
        &connection,
        &broad_id,
        &Ending::Skipped {
            reason: SkipReason::RobotsTxt,
            message: None,
        },
    )
    .expect("record robots_txt skip");

    let sources = reports(&connection).expect("reports");
    assert_eq!(sources[0].failed, 1);
    let failure = sources[0].failure.as_ref().expect("failure recorded");
    assert_eq!(failure.url, "http://shop.test/broken.xml");
    assert_eq!(failure.code, "response_not_ok");
    assert_eq!(failure.status, Some(404));
    assert_eq!(sources[1].not_added, 1);
    assert_eq!(sources[1].not_added_by_reason[0].name, "out_of_scope");
    assert_eq!(
        sources[1].failure.as_ref().expect("skip failure").code,
        "robots_txt"
    );
}

#[test]
fn parses_ttl_sy_update_period_and_entry_dates_across_formats() {
    let rss2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:sy="http://purl.org/rss/1.0/modules/syndication/">
  <channel>
    <title>News</title>
    <ttl>45</ttl>
    <sy:updatePeriod>hourly</sy:updatePeriod>
    <sy:updateFrequency>1</sy:updateFrequency>
    <item>
      <link>http://shop.test/a1</link>
      <pubDate>Mon, 02 Nov 2026 12:30:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#;
    let doc = parse(rss2.as_bytes()).expect("rss2 with hints");
    assert_eq!(doc.ttl_minutes, Some(45));
    assert_eq!(doc.sy_update_period, Some(SyUpdatePeriod::Hourly));
    assert_eq!(doc.sy_update_frequency, Some(1));
    assert_eq!(doc.floor_duration_ms(true, false), Some(45 * 60_000));
    assert_eq!(doc.floor_duration_ms(false, true), Some(60 * 60_000));
    assert_eq!(doc.floor_duration_ms(true, true), Some(60 * 60_000));
    assert_eq!(doc.floor_duration_ms(false, false), None);
    let expected_ts = parse_feed_date("2026-11-02T12:30:00Z").expect("iso");
    assert_eq!(doc.entries[0].updated_at, Some(expected_ts));
    assert_eq!(
        doc.entries_in_last_week(expected_ts.ms + 3_600_000),
        Some(1)
    );
    assert_eq!(
        doc.entries_in_last_week(expected_ts.ms + 8 * 86_400_000),
        Some(0)
    );

    let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <link href="http://shop.test/a2"/>
    <published>2026-11-01T00:00:00Z</published>
    <updated>2026-11-02T12:30:00Z</updated>
  </entry>
</feed>"#;
    let atom_doc = parse(atom.as_bytes()).expect("atom with dates");
    assert_eq!(atom_doc.entries[0].updated_at, Some(expected_ts));
}

#[test]
fn syndication_read_reports_changed_on_new_or_updated_entry() {
    let expected_ts = parse_feed_date("2026-11-02T12:30:00Z").expect("iso");
    let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <link href="http://shop.test/a2"/>
    <published>2026-11-01T00:00:00Z</published>
    <updated>2026-11-02T12:30:00Z</updated>
  </entry>
</feed>"#;
    let atom_doc = parse(atom.as_bytes()).expect("atom with dates");

    let (connection, invariants, policy) = test_db();
    let work = CountedWork::default();
    let mut seq = 0_u64;
    let mut alloc = || {
        seq += 1;
        format!("id-{seq}")
    };
    let seed = SyndicationSeed {
        syndication: "http://shop.test/atom.xml".to_owned(),
        rules: vec![SitemapRule {
            pattern: "**".to_owned(),
            label: "article".to_owned(),
        }],
    };
    let Enqueued::Added { id, .. } =
        admit_seed(&connection, &invariants, &policy, &seed, |_| Ok(alloc())).expect("admit")
    else {
        panic!("expected added");
    };
    let first = read(
        &connection,
        &invariants,
        &policy,
        &id,
        (&atom_doc, ReadOptions::default()),
        &work,
        |_, _| Ok(alloc()),
    )
    .expect("first read");
    assert!(first.changed);
    assert_eq!(first.added, 1);

    let same = read(
        &connection,
        &invariants,
        &policy,
        &id,
        (
            &atom_doc,
            ReadOptions {
                previous_fetched_at: Some(expected_ts.ms),
                ..ReadOptions::default()
            },
        ),
        &work,
        |_, _| Ok(alloc()),
    )
    .expect("second read unchanged");
    assert!(!same.changed);
    assert_eq!(same.added, 0);

    let updated = read(
        &connection,
        &invariants,
        &policy,
        &id,
        (
            &atom_doc,
            ReadOptions {
                previous_fetched_at: Some(expected_ts.ms - 1_000),
                ..ReadOptions::default()
            },
        ),
        &work,
        |_, _| Ok(alloc()),
    )
    .expect("third read updated entry");
    assert!(updated.changed);
    assert_eq!(updated.added, 0);
}

goodfetch_test_support::fuzz_replay!(crate::syndication::fuzz_syndication, "fuzz/syndication");
