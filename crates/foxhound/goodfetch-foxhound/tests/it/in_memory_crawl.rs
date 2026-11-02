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
use goodfetch_test_support::{CountingVfs, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::crawl_plants::{Counted, run_state_plant};
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

/// The run-state rows a crawl with `settings` counted and, when `planted`,
/// with one extra unit of that row's work per page served.
struct RunStateCounts {
    transactions: u64,
    syncs: u64,
    writer_reads: u64,
    configuration_reads: u64,
    prunes: u64,
    effect_syncs: u64,
    attempts: u64,
}

impl RunStateCounts {
    /// The figure of the row named `row`.
    fn of(&self, row: &str) -> u64 {
        match row {
            "F2.transactions" => self.transactions,
            "F2.syncs" => self.syncs,
            "F2.writer_reads" => self.writer_reads,
            "F2.configuration_reads" => self.configuration_reads,
            "F2.prunes" => self.prunes,
            "F2.effect_syncs" => self.effect_syncs,
            other => panic!("no such row: {other}"),
        }
    }
}

fn run_state_counts(settings: &Value, planted: Option<&str>) -> RunStateCounts {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let vfs = CountingVfs::register();
    let plant = planted.map(|row| {
        run_state_plant(
            row,
            &Counted {
                work: &work,
                vfs: vfs.name(),
            },
        )
    });
    let crawled = crawl_with(
        env.home(),
        env.start_time(),
        env.seed(),
        Options {
            metrics,
            settings: settings.as_object().unwrap().clone(),
            plant,
            run_state_vfs: Some(vfs.name().to_owned()),
            // Small pages: a page's size moves none of these counts.
            largest_page: Some(PAGES[0]),
        },
    );
    RunStateCounts {
        transactions: work.read(Row::RunTransactions),
        syncs: vfs.syncs(),
        writer_reads: work.read(Row::WriterReads),
        configuration_reads: work.read(Row::ConfigurationReads),
        prunes: work.read(Row::Prunes),
        effect_syncs: work.read(Row::EffectSyncs),
        attempts: crawled.attempts,
    }
}

/// B1's counts cell (ADR 0140): each counter of the run-state writer sees one
/// extra unit of its work per page, done for real where it can be.
#[rstest]
#[case::transactions("F2.transactions")]
#[case::syncs("F2.syncs")]
#[case::writer_reads("F2.writer_reads")]
#[case::configuration_reads("F2.configuration_reads")]
#[case::prunes("F2.prunes")]
#[case::effect_syncs("F2.effect_syncs")]
fn a_planted_unit_per_page_moves_its_run_state_row_by_a_page_each(#[case] row: &str) {
    let default = run_state_counts(&json!({}), None);
    let planted = run_state_counts(&json!({}), Some(row));

    let pages = u64::try_from(PAGES.len() + 1).unwrap();
    assert_eq!(default.attempts, pages);
    assert!(
        planted.of(row) >= default.of(row) + pages,
        "{row}: {} planted, {} not",
        planted.of(row),
        default.of(row)
    );
}

/// The crawl does the writer's work the rows count at concurrency 1 and 16,
/// and durability changes how a transaction reaches the disk and not how many
/// there are: `fast` keeps its transactions and takes syncs away. That a count
/// does not depend on how its jobs group is `run_state`'s own test.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "three crawls of a dozen pages side by side, each on its own paused runtime, none waiting on another"
)]
fn the_crawl_does_the_writers_work_at_any_concurrency_and_fast_syncs_less() {
    let at = |concurrency: u64| json!({ "politeness": { "domain": { "delay": 0, "concurrency": concurrency } } });
    let (one, sixteen, full, fast) = std::thread::scope(|scope| {
        let one = scope.spawn(|| run_state_counts(&at(1), None));
        let sixteen = scope.spawn(|| run_state_counts(&at(16), None));
        let full = scope.spawn(|| run_state_counts(&json!({}), None));
        let fast = scope
            .spawn(|| run_state_counts(&json!({ "run_state": { "durability": "fast" } }), None));
        (
            one.join().unwrap(),
            sixteen.join().unwrap(),
            full.join().unwrap(),
            fast.join().unwrap(),
        )
    });

    for counts in [&one, &sixteen, &full] {
        assert!(counts.transactions > 0 && counts.syncs > 0 && counts.configuration_reads > 0);
    }
    assert_eq!(full.transactions, fast.transactions);
    assert!(
        fast.syncs < full.syncs,
        "{} at fast, {} at full",
        fast.syncs,
        full.syncs
    );
}
