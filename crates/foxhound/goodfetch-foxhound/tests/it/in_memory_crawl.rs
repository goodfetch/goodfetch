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

use crate::crawl_scenario::{Options, PAGES, Plant, body_bytes, crawl, crawl_with};

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

/// The rows of a Record's work (F4).
const RECORD_ROWS: [Row; 7] = [
    Row::RecordDeclarationReads,
    Row::RecordValidatorBuilds,
    Row::RecordValidations,
    Row::RecordSerializations,
    Row::RecordDeepCopies,
    Row::RecordConstantLookups,
    Row::RecordTransactions,
];

/// Each of [`RECORD_ROWS`]' counts in a crawl and, when `planted`, with one
/// more unit of each per page served, and how many Records it committed.
fn record_counts(planted: bool) -> ([u64; 7], u64) {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let plant: Option<Plant> = planted.then(|| {
        let work = work.clone();
        Arc::new(move |_head: &str| {
            for row in RECORD_ROWS {
                if row == Row::RecordValidatorBuilds {
                    let _ = goodfetch_dataset::counted_validator(&work, &json!({}));
                } else {
                    work.count(row);
                }
            }
        }) as Plant
    });
    let crawled = crawl_with(
        env.home(),
        env.start_time(),
        env.seed(),
        Options {
            metrics,
            plant,
            ..Options::default()
        },
    );
    (
        RECORD_ROWS.map(|row| work.read(row)),
        u64::try_from(crawled.records.len()).unwrap(),
    )
}

/// B1's counts cell (ADR 0140): each Record of the code-free crawl, under its
/// Dataset's schema, is validated and serialized once and commits in its
/// attempt's transaction; what its check and its commit repeat per Record is
/// counted (the Exceptions X7 and X8), and one planted unit per page adds
/// exactly its Records' worth to every row. `xtask counts` holds the figures
/// to the register, and `--prove` runs each plant through it.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "two crawls of a dozen pages side by side, each on its own paused runtime, none waiting on another"
)]
fn each_record_of_the_crawl_counts_its_work_and_a_plant_adds_one_a_record_to_every_row() {
    let ((default, records), (planted, _)) = std::thread::scope(|scope| {
        let default = scope.spawn(|| record_counts(false));
        let planted = scope.spawn(|| record_counts(true));
        (default.join().unwrap(), planted.join().unwrap())
    });

    let [
        reads,
        builds,
        validations,
        serializations,
        copies,
        lookups,
        transactions,
    ] = default;
    assert_eq!(records, u64::try_from(PAGES.len() + 1).unwrap());
    assert_eq!(
        (validations, serializations, transactions),
        (records, records, 0)
    );
    assert!(
        reads > 0 && builds > 0 && copies > 0 && lookups > 0,
        "reads {reads}, builds {builds}, copies {copies}, lookups {lookups}"
    );
    assert_eq!(
        std::array::from_fn::<u64, 7, _>(|row| planted[row] - default[row]),
        [records; 7],
    );
}
