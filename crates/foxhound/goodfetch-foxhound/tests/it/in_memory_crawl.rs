//! The crawl benchmark's scenario (ADR 0134's bytes per crawled page), run
//! as a test: a code-free crawl fed from memory through the fetch tiers'
//! test seam fetches each page of the site once, and the body bytes the
//! site sent are those `benches/pages.source.toml` records, which `xtask
//! perf` divides the crawl's total bytes by.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::crawl_scenario::{Frontier, Options, PAGES, Plant, body_bytes, crawl, crawl_with};

/// The total body bytes `benches/pages.source.toml` records.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the fixture's source file before the engine starts, on no async worker"
)]
fn recorded_body_bytes() -> u64 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/pages.source.toml");
    let recorded: toml::Table = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    recorded["body-bytes"]
        .as_integer()
        .and_then(|bytes| u64::try_from(bytes).ok())
        .unwrap()
}

/// The crawl from the index admits, fetches, parses and follows every page
/// it links, and extracts one Record per page holding its `h1`; the site
/// sends each body once, as many bytes as the fixture's source file
/// records.
#[test]
fn a_crawl_fed_from_memory_extracts_each_page_once_and_reads_the_body_bytes_its_fixture_records() {
    let env = TestEnv::new();
    let recorded = recorded_body_bytes();

    let crawled = crawl(env.home(), env.start_time(), env.seed());

    let titles: BTreeSet<String> = crawled
        .records
        .iter()
        .map(|record| record["title"].as_str().unwrap_or_default().to_owned())
        .collect();
    let expected: BTreeSet<String> = std::iter::once("Index".to_owned())
        .chain((0..PAGES.len()).map(|number| format!("Page {number}")))
        .collect();
    assert_eq!(
        (
            crawled.records.len(),
            titles,
            crawled.body_bytes,
            body_bytes()
        ),
        (PAGES.len() + 1, expected, recorded, recorded),
        "{:?}",
        crawled
            .records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
    );
}

/// The parses of a crawl with `settings` and, when `planted`, one more parse
/// per page served through the one counted function, each on its own engine.
fn parses(settings: &Value, planted: bool) -> u64 {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let plant: Option<Plant> = planted.then(|| {
        let work = work.clone();
        Arc::new(move |head: &str| {
            let _ = work.parse_html(Row::PageParses, head);
        }) as Plant
    });
    crawl_with(
        env.home(),
        env.start_time(),
        env.seed(),
        Options {
            metrics,
            settings: settings.as_object().unwrap().clone(),
            plant,
            ..Options::default()
        },
    );
    work.read(Row::PageParses)
}

/// B1's counts cell (ADR 0140): the crawl's parses are counted from the
/// parse, a planted one per page adds exactly its pages' worth, and turning
/// the Filter lists off takes the clean-up's parse away. `xtask counts` holds
/// the figure to the register, and `--prove` runs the plant through it.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "three crawls of a dozen pages side by side, each on its own paused runtime, none waiting on another"
)]
fn the_crawl_counts_its_parses_a_plant_adds_one_a_page_and_a_feature_off_removes_its_own() {
    let lists_off = json!({ "filter_lists": {
        "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false,
    } });
    let (default, planted, off) = std::thread::scope(|scope| {
        let default = scope.spawn(|| parses(&json!({}), false));
        let planted = scope.spawn(|| parses(&json!({}), true));
        let off = scope.spawn(|| parses(&lists_off, false));
        (
            default.join().unwrap(),
            planted.join().unwrap(),
            off.join().unwrap(),
        )
    });

    let pages = u64::try_from(PAGES.len() + 1).unwrap();
    assert!(default > 0, "the crawl counted no parse");
    assert_eq!(planted - default, pages);
    assert!(
        off < default,
        "{off} parses with the lists off, {default} on"
    );
}

/// The rows of the Frontier's admission, which every outcome reaches.
const FRONTIER_ROWS: [Row; 12] = [
    Row::BatchInvariantReads,
    Row::BatchCompilations,
    Row::BatchTransactions,
    Row::HopInvariantReads,
    Row::HopCompilations,
    Row::HopTransactions,
    Row::SitemapInvariantReads,
    Row::SitemapCompilations,
    Row::SitemapTransactions,
    Row::PayloadCompilations,
    Row::PayloadSerializations,
    Row::PayloadValidations,
];

/// What one crawl of the Frontier's fixture counted: each row of
/// [`FRONTIER_ROWS`], the Records, the hops and the sitemaps.
struct Frontiers {
    counts: Vec<u64>,
    pages: u64,
    hops: u64,
    sitemaps: u64,
}

impl Frontiers {
    fn of(&self, row: Row) -> u64 {
        let at = FRONTIER_ROWS.iter().position(|each| *each == row).unwrap();
        self.counts[at]
    }
}

/// A crawl of the Frontier's fixture, each page small, and when `planted`,
/// one more unit of every row per page served.
fn frontier_crawl(globs: bool, schema: bool, planted: bool) -> Frontiers {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let plant: Option<Plant> = planted.then(|| {
        let work = work.clone();
        Arc::new(move |_: &str| {
            for row in FRONTIER_ROWS {
                work.count(row);
            }
        }) as Plant
    });
    let settings = if globs {
        json!({ "crawl": { "scope": { "exclude": ["http://pages.test/never/**"] } } })
    } else {
        json!({})
    };
    let crawled = crawl_with(
        env.home(),
        env.start_time(),
        env.seed(),
        Options {
            metrics,
            settings: settings.as_object().unwrap().clone(),
            plant,
            frontier: Some(Frontier { schema }),
            small_pages: true,
        },
    );
    Frontiers {
        counts: FRONTIER_ROWS.iter().map(|row| work.read(*row)).collect(),
        pages: u64::try_from(crawled.records.len()).unwrap(),
        hops: crawled.hops,
        sitemaps: crawled.sitemaps,
    }
}

/// B1's Frontier rows (ADR 0140): a batch's, a hop's, a sitemap's and a
/// Payload's work are each counted on their own row, once the crawl met each
/// outcome; a plant per page adds exactly its pages' worth to each; and with
/// no glob in the Scope or no schema on the Label, their work goes.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "four crawls of a few small pages side by side, each on its own paused runtime, none waiting on another"
)]
fn each_outcome_of_the_frontier_counts_on_its_own_row_a_plant_adds_one_a_page_and_off_removes_its_work()
 {
    let (default, planted, no_globs, no_schema) = std::thread::scope(|scope| {
        let default = scope.spawn(|| frontier_crawl(true, true, false));
        let planted = scope.spawn(|| frontier_crawl(true, true, true));
        let no_globs = scope.spawn(|| frontier_crawl(false, true, false));
        let no_schema = scope.spawn(|| frontier_crawl(true, false, false));
        (
            default.join().unwrap(),
            planted.join().unwrap(),
            no_globs.join().unwrap(),
            no_schema.join().unwrap(),
        )
    });

    // The index and its pages, the redirect's target, the sitemap's entry and
    // the seed with a Payload.
    assert_eq!(
        (default.pages, default.hops, default.sitemaps),
        (u64::try_from(PAGES.len()).unwrap() + 4, 1, 1)
    );
    for row in FRONTIER_ROWS {
        assert!(default.of(row) > 0, "{row:?} counted nothing");
        assert_eq!(planted.of(row) - default.of(row), default.pages, "{row:?}");
    }
    assert!(no_globs.of(Row::BatchCompilations) < default.of(Row::BatchCompilations));
    assert!(no_globs.of(Row::HopCompilations) < default.of(Row::HopCompilations));
    assert_eq!(no_schema.of(Row::PayloadValidations), 0);
    assert_eq!(no_schema.of(Row::PayloadCompilations), 0);
}
