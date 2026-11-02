//! B1's counts cell (ADR 0140), beside the in-memory crawl's heap cell: runs
//! `benches/scenarios.rs`' crawl once with its default features on and once
//! per feature on its paths that can be switched off, and prints what each
//! counted as one JSON line, which `xtask counts` divides and holds to the
//! Quality register's limits. An example and not a `[[bench]]`, as `xtask
//! perf` runs every bench target under gungraun: counts need no Valgrind and
//! no optimized build.
//!
//! `--plant <row>` does one extra unit of that row's work per page served,
//! through its one counted function. The crawl fails here if the work it
//! counts did not happen, or if a feature off did not take its work away.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stdout,
    reason = "the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/scenarios.rs"]
#[expect(
    dead_code,
    reason = "the fixture's plain crawl, which the heap benchmark runs and this one does not"
)]
mod scenarios;

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use scenarios::{Options, PAGES, Plant, body_bytes, crawl_with};
use serde_json::{Map, Value, json};

/// The plant of the row named `name`.
fn plant(name: &str, work: &CountedWork) -> Plant {
    let work = work.clone();
    match name {
        "F1.parses" => Arc::new(move |head: &str| {
            let _ = work.parse_html(Row::PageParses, head);
        }),
        "F4.validator_builds" => Arc::new(move |_: &str| {
            let _ = goodfetch_dataset::counted_validator(&work, &json!({}));
        }),
        "F4.declaration_reads" => counting(work, Row::RecordDeclarationReads),
        "F4.validations" => counting(work, Row::RecordValidations),
        "F4.serializations" => counting(work, Row::RecordSerializations),
        "F4.deep_copies" => counting(work, Row::RecordDeepCopies),
        "F4.constant_lookups" => counting(work, Row::RecordConstantLookups),
        "F4.transactions" => counting(work, Row::RecordTransactions),
        other => panic!("no plant for the row {other}"),
    }
}

/// The plant of a row counted at its seam, which no function wraps.
fn counting(work: CountedWork, row: Row) -> Plant {
    Arc::new(move |_: &str| work.count(row))
}

/// One cell: a name, the settings it runs with beside the fixture's, and the
/// rows it must count fewer of than the default cell: a feature's work goes
/// with the feature (ADR 0140).
struct Cell {
    name: &'static str,
    settings: Value,
    removes: &'static [Row],
}

fn cells() -> [Cell; 2] {
    [
        Cell {
            name: "default",
            settings: json!({}),
            removes: &[],
        },
        Cell {
            name: "filter_lists_off",
            settings: json!({ "filter_lists": {
                "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false,
            } }),
            removes: &[Row::PageParses],
        },
    ]
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    pages: u64,
    work: CountedWork,
    removes: &'static [Row],
}

/// Runs `cell`, on an engine and in a home of its own.
fn run(cell: Cell, planted: Option<&str>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let settings: Map<String, Value> = match cell.settings {
        Value::Object(settings) => settings,
        other => panic!("a cell's settings are an object, not {other}"),
    };
    let crawled = crawl_with(
        home.path(),
        std::time::UNIX_EPOCH,
        0,
        Options {
            metrics,
            settings,
            plant: planted.map(|name| plant(name, &work)),
        },
    );
    assert_eq!(
        crawled.body_bytes,
        body_bytes(),
        "the site sends each body once"
    );
    assert_eq!(
        crawled.records.len(),
        PAGES.len() + 1,
        "the crawl extracts each page once"
    );
    Counted {
        name: cell.name,
        pages: u64::try_from(crawled.records.len()).expect("invariant: a dozen pages"),
        work,
        removes: cell.removes,
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let planted = match arguments.as_slice() {
        [] => None,
        [flag, row] if flag == "--plant" => Some(row.as_str()),
        other => panic!("usage: counts [--plant <row>], not {other:?}"),
    };
    // Side by side, each on its own paused runtime: no cell waits on another,
    // so a family's rows stay within their minute.
    let counted: Vec<Counted> = std::thread::scope(|scope| {
        let cells: Vec<_> = cells()
            .into_iter()
            .map(|cell| scope.spawn(move || run(cell, planted)))
            .collect();
        cells
            .into_iter()
            .map(|cell| cell.join().expect("invariant: a cell's checks passed"))
            .collect()
    });
    let default = &counted[0];
    for cell in &counted {
        for row in cell.removes {
            assert!(
                cell.work.read(*row) < default.work.read(*row),
                "the cell {} leaves {} as it is",
                cell.name,
                row.name()
            );
        }
        let counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(cell.work.read(*row))))
            .collect();
        println!(
            "{}",
            json!({
                "cell": cell.name,
                "units": { "pages": cell.pages, "records": cell.pages },
                "counts": counts,
            })
        );
    }
}
