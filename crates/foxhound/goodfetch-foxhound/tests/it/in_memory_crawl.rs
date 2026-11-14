//! The crawl benchmark's scenario (ADR 0134's bytes per crawled page), run
//! as a test: a code-free crawl fed from memory through the fetch tiers'
//! test seam fetches each page of the site once, and the body bytes the
//! site sent are those `benches/pages.source.toml` records, which `xtask
//! perf` divides the crawl's total bytes by.
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::home::Home;
use goodfetch_engine::ledger::Ledger;
use goodfetch_engine::metrics::Metrics;
use goodfetch_test_support::{CountingVfs, TestEnv};
use jiff::Timestamp;
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::crawl_plants::{Counted, politeness_plant, run_state_plant};
use crate::crawl_scenario::{
    Frontier, Options, PAGES, Plant, body_bytes, crawl, crawl_with, entries, page_analysis_plant,
    run_trace_plant,
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
#[case::tree_copies(Row::PageTreeCopies)]
#[case::share_readability_runs(Row::PageShareReadabilityRuns)]
#[case::block_classifications(Row::PageBlockClassifications)]
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

/// One extra unit of a Run-trace row's work per page served adds exactly its
/// pages' worth, through its one counted function or at its seam (ADR 0140):
/// each counter sees the work `xtask counts` holds to its limit per entry.
#[rstest]
#[case::file_opens(Row::RunTraceFileOpens)]
#[case::spawns(Row::RunTraceSpawns)]
#[case::formatting_off(Row::RunTraceFormattingOff)]
fn a_planted_unit_of_a_run_trace_row_adds_one_a_page(#[case] row: Row) {
    let default = counted(row, Options::default());
    let metrics = Metrics::none();
    let plant = run_trace_plant(row, metrics.counted_work());
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

/// The crawl writes Run-trace entries, all Provenance's, and each costs what
/// the Run-trace rows count: a task spawned for its write (Exception X2). The
/// Run holds the Provenance log's file, so its folder is created and its file
/// opened once a Run, and no page opens any. No kind of entry the crawl writes
/// can be switched off, so none is formatted while off.
#[test]
fn each_run_trace_entry_of_the_crawl_spawns_its_write_and_its_run_opens_the_file_once() {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();

    crawl_with(
        env.home(),
        env.start_time(),
        env.seed(),
        Options {
            metrics,
            ..Options::default()
        },
    );

    let entries = entries(env.home());
    assert!(entries > 0, "the crawl wrote no Run-trace entry");
    assert_eq!(
        (
            work.read(Row::RunTraceFileOpens),
            work.read(Row::RunTraceSpawns),
            work.read(Row::RunTraceFormattingOff),
            work.read(Row::PageFileOpens),
        ),
        (2, entries, 0, 0)
    );
}

/// The fixture does what the Page analysis rows count: a crawl runs readability,
/// compiles selectors, and opens its trace's files, the Filter lists compile no
/// selector of their own in a crawl that asks for no format, and a field's Value
/// transform builds its forms once, however many pages it reads.
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

    let pages = u64::try_from(PAGES.len() + 1).unwrap();
    // The crawl asks for no format, so the demand plan runs no Clean-up and
    // its rules compile none of their 613 selectors, with the Filter lists on
    // or off (ADR 0138's demand plan, #1909). The untouched page's readers
    // compile their 9 once a Run (#1910; `base[href]`, which two of them
    // read, once), and the main-content step's 5 and a handler's markup
    // waits' 2 with them, into the engine's cache, which every page then
    // reads: no page compiles any (#1912). The streaming pass of the AI-use
    // signals' markup compiles its 4 once a Run (#1917). The Run holds its
    // Provenance log's file, so no page opens one.
    assert_eq!(
        (
            default.read(Row::PageReadabilityRuns),
            default.read(compilations),
            off.read(compilations),
            default.read(Row::PageFileOpens),
            default.read(Row::PageLocaleFormBuilds),
            forms.read(Row::PageLocaleFormBuilds),
        ),
        (pages, 20, 20, 0, 0, 1)
    );
}

/// A handler on a document tier waits on each page, which the tier answers from
/// the page's kept tree: the page is parsed once for its attempt, its Page
/// signature reads a copy, and the wait's selector comes from the engine's
/// cache, compiled once for the crawl.
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
    // One parse a page, which its waits read on the kept tree (#1918), and
    // one copy a page, which its Page signature reads. The
    // untouched page's readers compile 7 once a Run (#1910; `base[href]`,
    // which two of them read, once), and the main-content step's and the
    // markup waits' 7 with them. The handler's own `h1` compiles once, at its
    // first wait, into the engine's cache, which every later wait reads
    // (#1912). The streaming pass of the AI-use signals' markup parses its 4
    // once a Run (#1917).
    assert_eq!(
        (
            crawled.handled,
            work.read(Row::PageParses),
            work.read(Row::PageTreeCopies),
            work.read(Row::PageSelectorCompilations),
        ),
        (pages, pages, pages, 19)
    );
}

/// A crawl that needs no tree, asking for no format with learning off,
/// reads each page's links and fields in one streaming pass: it parses no
/// page (#1911).
#[test]
fn a_crawl_that_needs_no_tree_parses_no_page() {
    assert_eq!(
        parses(&json!({ "frontier": { "learning": false } }), false),
        0
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

/// B1 meets a Limit (ADR 0140's E3): every page's request is charged through
/// the counted ledger, on the crawl's own engine, which the crawl with no
/// Limit also reads and no Slice's transaction commits; a Slice's commit is
/// the difference. A charge the Slices the Run holds cover runs no SQL
/// (issue 1888), so the ledger counts one statement, the first charge's
/// read of the Limits, which stay current on the crawl's paused clock;
/// and the ledger holds one request per page as charged to the Limit once
/// the crawl ended.
#[test]
fn a_crawl_under_a_limit_charges_each_page_through_the_counted_ledger() {
    const LIMIT: u64 = 1_000_000;
    let charged = |limit: Option<u64>| {
        let env = TestEnv::new();
        let metrics = Metrics::none();
        let work = metrics.counted_work().clone();
        crawl_with(
            env.home(),
            env.start_time(),
            env.seed(),
            Options {
                metrics,
                limit,
                ..Options::default()
            },
        );
        let ledger = Ledger::at(Home::new(env.home().to_owned()).usage_ledger(), None);
        let now = Timestamp::try_from(env.start_time()).unwrap();
        let used: Vec<u64> = ledger
            .limits()
            .unwrap()
            .iter()
            .map(|limit| limit.amount - ledger.left(limit, now).unwrap())
            .collect();
        (
            (
                work.read(Row::ChargeStatements),
                work.read(Row::ChargeTransactions),
            ),
            used,
        )
    };

    let ((limited, used), (free, _)) = (charged(Some(LIMIT)), charged(None));

    let pages = u64::try_from(PAGES.len() + 1).unwrap();
    assert_eq!(
        (limited.0, used),
        (1, vec![pages]),
        "one read of the Limits, and each of the {pages} pages charged: {limited:?}"
    );
    assert!(
        limited.1 > free.1,
        "a Slice taken commits: {limited:?} with a Limit, {free:?} without"
    );
}

/// B1's counts cell (ADR 0140): the crawl's parses are counted from the
/// parse, a planted one per page adds exactly its pages' worth, and turning
/// the Filter lists off changes nothing in a crawl that asks for no format,
/// which runs no clean-up either way (ADR 0138's demand plan). `xtask counts` holds
/// the figure to the register, and `--prove` runs the plant through it.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "three crawls of a dozen pages side by side, each on its own paused runtime, none waiting on another"
)]
fn the_crawl_counts_its_parses_a_plant_adds_one_a_page_and_the_filter_lists_off_change_nothing() {
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
    assert_eq!(
        off, default,
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
/// attempt's transaction, reading no declaration and building no validator,
/// both done when the crawl starts (ADR 0138); its commit copies none of its
/// data and looks up no value constant for the Run, and one planted unit per
/// page adds exactly its Records' worth to every row. `xtask counts` holds the figures
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
        (
            reads,
            builds,
            validations,
            serializations,
            copies,
            lookups,
            transactions
        ),
        (0, 0, records, records, 0, 0, 0)
    );
    assert_eq!(
        std::array::from_fn::<u64, 7, _>(|row| planted[row] - default[row]),
        [records; 7],
    );
}

/// The rows of the Frontier's admission, which every outcome reaches.
const FRONTIER_ROWS: [Row; 14] = [
    Row::BatchInvariantReads,
    Row::BatchCompilations,
    Row::BatchTransactions,
    Row::HopInvariantReads,
    Row::HopCompilations,
    Row::HopTransactions,
    Row::HopRoundTrips,
    Row::HopEndRoundTrips,
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
            frontier: Some(Frontier {
                schema,
                items: 0,
                second_hop: true,
            }),
            small_pages: true,
            ..Options::default()
        },
    );
    Frontiers {
        counts: FRONTIER_ROWS.iter().map(|row| work.read(*row)).collect(),
        pages: u64::try_from(crawled.records.len()).unwrap(),
        hops: crawled.hops,
        sitemaps: crawled.sitemaps,
    }
}

/// The rows of [`FRONTIER_ROWS`] that the Run's invariants, built once as
/// it starts, keep at 0 on every outcome: none reads the Scope, the Payload
/// rules or the Fragments again, nor compiles a glob of the Scope, a
/// sitemap's rule or a Label's schema (#1896).
const BUILT_ONCE: [Row; 7] = [
    Row::BatchInvariantReads,
    Row::BatchCompilations,
    Row::HopInvariantReads,
    Row::HopCompilations,
    Row::SitemapInvariantReads,
    Row::SitemapCompilations,
    Row::PayloadCompilations,
];

/// B1's Frontier rows (ADR 0140): a batch's, a hop's, a sitemap's and a
/// Payload's work are each counted on their own row; a plant per page adds
/// exactly its pages' worth to each; the Run's invariants are built once,
/// so no outcome reads or compiles one, a glob in the Scope or a schema on
/// the Label included; the one seed with a Payload is serialized once and
/// validated once, and with no schema on its Label not validated (#1896);
/// a redirect hop's check and its chain's end are one round trip each to
/// the actor that owns the crawl's Run state (#2246).
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

    // The index and its pages, the redirect's target, which its chain
    // reaches in two hops, the sitemap's entry and the seed with a Payload.
    assert_eq!(
        (default.pages, default.hops, default.sitemaps),
        (u64::try_from(PAGES.len()).unwrap() + 4, 2, 1)
    );
    for row in FRONTIER_ROWS {
        assert_eq!(planted.of(row) - default.of(row), default.pages, "{row:?}");
    }
    let built_once = |crawl: &Frontiers| BUILT_ONCE.map(|row| crawl.of(row));
    assert_eq!(
        (built_once(&default), built_once(&no_globs)),
        ([0; 7], [0; 7])
    );
    assert!(default.of(Row::HopTransactions) > 0);
    // A code-free page's links ride its outcome's job (#2245), and a
    // sitemap's read keys its entries in its fetch's (#2167).
    assert_eq!(
        (
            default.of(Row::BatchTransactions),
            default.of(Row::SitemapTransactions)
        ),
        (0, 0)
    );
    // Each of the redirect's two hops asks the actor that owns the Run
    // state once, and their one chain's end once more (#2246).
    assert_eq!(
        (
            default.of(Row::HopRoundTrips),
            default.of(Row::HopEndRoundTrips)
        ),
        (default.hops, 1)
    );
    assert_eq!(
        (
            default.of(Row::PayloadSerializations),
            default.of(Row::PayloadValidations),
            no_schema.of(Row::PayloadSerializations),
            no_schema.of(Row::PayloadValidations),
        ),
        (1, 1, 1, 0)
    );
}

/// The run-state rows a crawl with `settings` counted and, when `planted`,
/// with one extra unit of that row's work per page served.
struct RunStateCounts {
    transactions: u64,
    syncs: u64,
    writer_reads: u64,
    evidence_reads: u64,
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
            "F2.evidence_reads" => self.evidence_reads,
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
            ..Options::default()
        },
    );
    RunStateCounts {
        transactions: work.read(Row::RunTransactions),
        syncs: vfs.syncs(),
        writer_reads: work.read(Row::WriterReads),
        evidence_reads: work.read(Row::EvidenceReads),
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
#[case::evidence_reads("F2.evidence_reads")]
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
/// reading the machine's settings once as the Run opens, however many
/// admissions and settlings follow (ADR 0138), beside the engine's own read
/// as it starts, for its managed Limits (ADR 0041); and durability changes how a
/// transaction reaches the disk and not how many there are: `fast` keeps its
/// transactions and takes syncs away. That a count does not depend on how its
/// jobs group is `run_state`'s own test.
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
        assert!(counts.transactions > 0 && counts.syncs > 0);
        assert_eq!(counts.configuration_reads, 2);
    }
    assert_eq!(full.transactions, fast.transactions);
    assert!(
        fast.syncs < full.syncs,
        "{} at fast, {} at full",
        fast.syncs,
        full.syncs
    );
}

/// What a crawl of the site serving its robots.txt, with `settings`, counted
/// on the Politeness permit's rows (E6) and, when `planted`, with one extra
/// unit of that row's work per page served; and the requests the site was
/// sent.
fn politeness_counts(settings: &Value, planted: Option<&str>) -> (CountedWork, u64) {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let plant = planted.map(|row| politeness_plant(row, &work));
    let crawled = crawl_with(
        env.home(),
        env.start_time(),
        env.seed(),
        Options {
            metrics,
            settings: settings.as_object().unwrap().clone(),
            plant,
            robots_txt: true,
            largest_page: Some(PAGES[0]),
            ..Options::default()
        },
    );
    assert_eq!(crawled.records.len(), PAGES.len() + 1);
    (work, crawled.requests)
}

/// B1's counts cell (ADR 0140): the crawl takes a Permit per request in
/// transactions of the Politeness file it opened, and reads and parses the
/// robots.txt copy its user's runs share, which its site serves.
#[test]
fn the_crawl_does_the_politeness_work_its_rows_count() {
    let (work, requests) = politeness_counts(&json!({}), None);

    let pages = u64::try_from(PAGES.len() + 1).unwrap();
    assert!(
        requests > pages,
        "{requests} requests: no robots.txt was sent"
    );
    assert!(
        work.read(Row::PolitenessTransactions) >= requests,
        "{} Politeness transactions for {requests} requests",
        work.read(Row::PolitenessTransactions)
    );
    assert_eq!(work.read(Row::PolitenessOpens), 1, "the file opens once");
    assert!(
        work.read(Row::RobotsReads) > 0,
        "the crawl read no robots.txt copy"
    );
    assert!(
        work.read(Row::RobotsParses) > 0,
        "the crawl parsed no robots.txt"
    );
    assert_eq!(work.read(Row::PolitenessScriptSends), 0);
}

/// B1's counts cell (ADR 0140): each counter of the Politeness permit sees one
/// extra unit of its work per page, done for real where it can be. `E6.opens`'
/// preparations are Callgrind's, which `xtask counts --prove` plants.
#[rstest]
#[case::transactions("E6.transactions", Row::PolitenessTransactions)]
#[case::script_sends("E6.script_sends", Row::PolitenessScriptSends)]
#[case::robots_parses("E6.robots_parses", Row::RobotsParses)]
#[case::robots_reads("E6.robots_reads", Row::RobotsReads)]
fn a_planted_unit_per_page_moves_its_politeness_row_by_a_page_each(
    #[case] name: &str,
    #[case] row: Row,
) {
    let (default, _) = politeness_counts(&json!({}), None);
    let (planted, _) = politeness_counts(&json!({}), Some(name));

    let pages = u64::try_from(PAGES.len() + 1).unwrap();
    assert!(
        planted.read(row) >= default.read(row) + pages,
        "{name}: {} planted, {} not",
        planted.read(row),
        default.read(row)
    );
}
