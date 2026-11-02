//! The crawl benchmark's scenario (ADR 0134's bytes per crawled page), run
//! as a test: a code-free crawl fed from memory through the fetch tiers'
//! test seam fetches each page of the site once, and the body bytes the
//! site sent are those `benches/pages.source.toml` records, which `xtask
//! perf` divides the crawl's total bytes by.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::crawl_scenario::{
    Options, PAGES, Plant, body_bytes, crawl, crawl_with, page_analysis_plant,
};

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

/// What a crawl under `options` counted, on an engine of its own.
fn worked(options: Options) -> CountedWork {
    let env = TestEnv::new();
    let work = options.metrics.counted_work().clone();
    crawl_with(env.home(), env.start_time(), env.seed(), options);
    work
}

/// What a crawl under `options` counted on `row`.
fn counted(row: Row, options: Options) -> u64 {
    worked(options).read(row)
}

/// One extra unit of a Page analysis row's work per page served adds exactly
/// its pages' worth, through its one counted function (ADR 0140): each counter
/// sees the work `xtask counts` holds to its limit.
#[rstest]
#[case::selector_compilations(Row::PageSelectorCompilations)]
#[case::readability_runs(Row::PageReadabilityRuns)]
#[case::file_opens(Row::PageFileOpens)]
#[case::locale_form_builds(Row::PageLocaleFormBuilds)]
#[case::plugin_serializations(Row::PagePluginSerializations)]
#[case::plugin_answer_parses(Row::PagePluginAnswerParses)]
fn a_planted_unit_of_a_page_analysis_row_adds_one_a_page(#[case] row: Row) {
    let default = counted(row, Options::default());
    let metrics = Metrics::none();
    let plant = page_analysis_plant(row, metrics.counted_work());
    let planted = counted(
        row,
        Options {
            metrics,
            plant,
            ..Options::default()
        },
    );

    assert_eq!(planted - default, u64::try_from(PAGES.len() + 1).unwrap());
}

/// The fixture does what the Page analysis rows count: a crawl runs readability,
/// compiles selectors, and opens its trace's files, the Filter lists' selectors
/// go with the Filter lists, and a field's Value transform builds its forms once,
/// however many pages it reads.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "three crawls of a dozen pages side by side, each on its own paused runtime, none waiting on another"
)]
fn the_crawl_does_the_work_its_page_analysis_rows_count() {
    let lists_off = Map::from_iter([(
        "filter_lists".to_owned(),
        json!({ "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false }),
    )]);
    let field = json!({
        "locator": { "kind": "css", "selector": "h1" },
        "transform": { "kind": "number", "locale": "de-DE" },
    });
    let number = Map::from_iter([("number".to_owned(), field)]);
    let compilations = Row::PageSelectorCompilations;

    let (default, off, forms) = std::thread::scope(|scope| {
        let default = scope.spawn(|| worked(Options::default()));
        let off = scope.spawn(|| {
            worked(Options {
                settings: lists_off,
                ..Options::default()
            })
        });
        let forms = scope.spawn(|| {
            worked(Options {
                fields: number,
                ..Options::default()
            })
        });
        (
            default.join().unwrap(),
            off.join().unwrap(),
            forms.join().unwrap(),
        )
    });

    assert_eq!(
        (
            default.read(Row::PageReadabilityRuns),
            default.read(compilations) > off.read(compilations),
            off.read(compilations) > 0,
            default.read(Row::PageFileOpens) > 0,
            default.read(Row::PageLocaleFormBuilds),
            forms.read(Row::PageLocaleFormBuilds),
        ),
        (
            u64::try_from(PAGES.len() + 1).unwrap(),
            true,
            true,
            true,
            0,
            1
        )
    );
}

/// A handler on a document tier waits on each page, which the tier answers from
/// the page's own document: each wait counts its parse and its selector.
#[test]
fn a_handlers_waits_on_a_document_tier_count_as_the_pages_work() {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();

    let crawled = crawl_with(
        env.home(),
        env.start_time(),
        env.seed(),
        Options {
            metrics,
            handler: true,
            ..Options::default()
        },
    );

    let pages = u64::try_from(PAGES.len() + 1).unwrap();
    let (parses, selectors) = (
        work.read(Row::PageParses),
        work.read(Row::PageSelectorCompilations),
    );
    assert_eq!(crawled.handled, pages);
    assert!(
        parses >= 2 * pages && selectors >= 2 * pages,
        "{parses} parses and {selectors} selectors over {pages} pages"
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
