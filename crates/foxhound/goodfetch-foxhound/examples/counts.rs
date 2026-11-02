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
//!
//! A page's rows of [`FROM_FIRST_PAGE`] leave out what the crawl's start did
//! before its first page: a selector checked and a locale's forms built once
//! per crawl are no page's work (ADRs 0138 and 0140), and a build moved onto
//! each page counts at once.
//!
//! The Run-trace rows count per entry: each cell prints, beside its pages,
//! the entries its Run trace holds, read from the trace's files once its
//! engine stopped, and fails if it wrote none.
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

use std::sync::{Arc, OnceLock};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use scenarios::{
    Options, PAGES, Plant, body_bytes, crawl_with, page_analysis_plant, run_trace_plant,
};
use serde_json::{Map, Value, json};

/// The plant of the row named `name`.
fn plant(name: &str, work: &CountedWork) -> Plant {
    let work = work.clone();
    match name {
        "F1.parses" => Arc::new(move |head: &str| {
            let _ = work.parse_html(Row::PageParses, head);
        }),
        other => Row::ALL
            .into_iter()
            .find(|row| row.name() == other)
            .and_then(|row| page_analysis_plant(row, &work).or_else(|| run_trace_plant(row, &work)))
            .unwrap_or_else(|| panic!("no plant for the row {other}")),
    }
}

/// The rows counted from the crawl's first page on, not from its start.
const FROM_FIRST_PAGE: [Row; 6] = [
    Row::PageSelectorCompilations,
    Row::PageReadabilityRuns,
    Row::PageFileOpens,
    Row::PageLocaleFormBuilds,
    Row::PagePluginSerializations,
    Row::PagePluginAnswerParses,
];

/// The field whose Value transform has the crawl build a locale's forms:
/// the `h1`'s number, under `de-DE`, which only the pages holding one have.
fn number_field() -> Map<String, Value> {
    let field = json!({
        "locator": { "kind": "css", "selector": "h1" },
        "match": { "pattern": "[0-9]+", "group": 0 },
        "transform": { "kind": "number", "locale": "de-DE" },
    });
    Map::from_iter([("number".to_owned(), field)])
}

/// One cell: a name, the settings it runs with beside the fixture's, and the
/// rows it must count fewer of than the default cell: a feature's work goes
/// with the feature (ADR 0140).
struct Cell {
    name: &'static str,
    settings: Value,
    removes: &'static [Row],
    handler: bool,
}

fn cells() -> [Cell; 3] {
    [
        Cell {
            name: "default",
            settings: json!({}),
            removes: &[],
            handler: false,
        },
        Cell {
            name: "filter_lists_off",
            settings: json!({ "filter_lists": {
                "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false,
            } }),
            removes: &[Row::PageParses, Row::PageSelectorCompilations],
            handler: false,
        },
        Cell {
            name: "document_tier_handler",
            settings: json!({}),
            removes: &[],
            handler: true,
        },
    ]
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    pages: u64,
    /// The entries its Run trace holds, read from its files.
    entries: u64,
    work: CountedWork,
    /// What [`FROM_FIRST_PAGE`] held when the first page was asked for.
    started: Arc<OnceLock<Vec<u64>>>,
    removes: &'static [Row],
}

impl Counted {
    /// What this cell counted on `row`.
    fn count(&self, row: Row) -> u64 {
        let before = FROM_FIRST_PAGE
            .iter()
            .position(|counted| *counted == row)
            .map_or(0, |at| self.started.get().map_or(0, |started| started[at]));
        self.work.read(row) - before
    }
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
    let planted = planted.map(|name| plant(name, &work));
    let started = Arc::new(OnceLock::new());
    let first_page: Plant = {
        let (work, started) = (work.clone(), Arc::clone(&started));
        Arc::new(move |head: &str| {
            started.get_or_init(|| FROM_FIRST_PAGE.map(|row| work.read(row)).to_vec());
            if let Some(planted) = &planted {
                planted(head);
            }
        })
    };
    let crawled = crawl_with(
        home.path(),
        std::time::UNIX_EPOCH,
        0,
        Options {
            metrics,
            settings,
            plant: Some(first_page),
            fields: if cell.handler {
                Map::new()
            } else {
                number_field()
            },
            handler: cell.handler,
        },
    );
    assert_eq!(
        crawled.body_bytes,
        body_bytes(),
        "the site sends each body once"
    );
    let pages = PAGES.len() + 1;
    if cell.handler {
        assert_eq!(
            crawled.handled,
            u64::try_from(pages).expect("invariant: a dozen pages"),
            "the handler handles each page once"
        );
    } else {
        assert_eq!(
            crawled.records.len(),
            pages,
            "the crawl extracts each page once"
        );
        assert!(
            crawled
                .records
                .iter()
                .any(|record| record["title"] == "Page 3" && record["number"] == 3),
            "the field's Value transform read no number: {:?}",
            crawled.records
        );
        let forms = FROM_FIRST_PAGE
            .iter()
            .position(|row| *row == Row::PageLocaleFormBuilds);
        let built = forms.and_then(|at| started.get().map(|started| started[at]));
        assert_eq!(
            built,
            Some(1),
            "the crawl builds its field's forms once, as it starts"
        );
    }
    assert!(crawled.entries > 0, "the crawl wrote no Run-trace entry");
    Counted {
        name: cell.name,
        pages: u64::try_from(pages).expect("invariant: a dozen pages"),
        entries: crawled.entries,
        work,
        started,
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
                cell.count(*row) < default.count(*row),
                "the cell {} leaves {} as it is",
                cell.name,
                row.name()
            );
        }
        let counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(cell.count(*row))))
            .collect();
        println!(
            "{}",
            json!({
                "cell": cell.name,
                "units": { "pages": cell.pages, "entries": cell.entries },
                "counts": counts,
            })
        );
    }
}
