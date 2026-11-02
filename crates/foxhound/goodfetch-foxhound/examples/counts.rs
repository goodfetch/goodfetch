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
//! What no wrapper sees is read from outside it: syncs through a counting VFS
//! under the run file, and statement preparations from Callgrind's call count
//! of `sqlite3_prepare_v3`, for which each cell runs this example again as
//! `--callgrind <cell>`, on pages of about 1 kB, as Callgrind takes minutes on
//! a crawl with no large page and far longer on the whole site. A page's size
//! moves no statement. Without Valgrind `F2.prepares` goes unprinted, which `xtask
//! counts` fails as a row not counted.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stderr,
    reason = "why a row goes unprinted, which `xtask counts` fails, beside its finding"
)]
#![expect(
    clippy::print_stdout,
    reason = "the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/plants.rs"]
mod plants;
#[path = "../benches/scenarios.rs"]
#[expect(
    dead_code,
    reason = "the fixture's plain crawl, which the heap benchmark runs and this one does not"
)]
mod scenarios;

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use goodfetch_test_support::CountingVfs;
use goodfetch_test_support::callgrind;
use plants::{Counted as PlantOn, run_state_plant};
use scenarios::{Options, PAGES, Plant, body_bytes, crawl_with};
use serde_json::{Map, Value, json};

/// The plant of the row named `name`.
fn plant(name: &str, work: &CountedWork, vfs: &str) -> Plant {
    let row = work.clone();
    match name {
        "F1.parses" => Arc::new(move |head: &str| {
            let _ = row.parse_html(Row::PageParses, head);
        }),
        other => run_state_plant(other, &PlantOn { work, vfs }),
    }
}

/// The row Callgrind reads, and the C symbol it reads it from.
const PREPARES: &str = "F2.prepares";
const PREPARE: &str = "sqlite3_prepare_v3";

/// The row the counting VFS reads.
const SYNCS: &str = "F2.syncs";

/// The units the rows of no outcome of this fixture divide by: one Run, and
/// no Effect until the Effect journal lands (#1550).
const RUNS: u64 = 1;
const EFFECTS: u64 = 0;

/// The paragraphs of the fixture's smallest page, about 1 kB: all a
/// `--callgrind` run serves a page.
const CALLGRIND_PAGE: usize = PAGES[0];

/// One cell: a name, the settings it runs with beside the fixture's, and the
/// rows it must count fewer of than the default cell: a feature's work goes
/// with the feature (ADR 0140).
struct Cell {
    name: &'static str,
    settings: Value,
    removes: &'static [Row],
    removes_syncs: bool,
}

fn cells() -> [Cell; 5] {
    [
        Cell {
            name: "default",
            settings: json!({}),
            removes: &[],
            removes_syncs: false,
        },
        // A transaction counts one job whatever group it commits in (ADR
        // 0138's note), so a row is read where commits group least and where
        // they group most. The crawl's own jobs differ between the two: one
        // request at a time runs a delivery pass per request. Each is held to
        // the limit, so neither hides the other's work.
        Cell {
            name: "concurrency_1",
            settings: json!({ "politeness": { "domain": { "delay": 0, "concurrency": 1 } } }),
            removes: &[],
            removes_syncs: false,
        },
        Cell {
            name: "concurrency_16",
            settings: json!({ "politeness": { "domain": { "delay": 0, "concurrency": 16 } } }),
            removes: &[],
            removes_syncs: false,
        },
        Cell {
            name: "durability_fast",
            settings: json!({ "run_state": { "durability": "fast" } }),
            removes: &[],
            removes_syncs: true,
        },
        Cell {
            name: "filter_lists_off",
            settings: json!({ "filter_lists": {
                "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false,
            } }),
            removes: &[Row::PageParses],
            removes_syncs: false,
        },
    ]
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    pages: u64,
    attempts: u64,
    work: CountedWork,
    syncs: u64,
    prepares: Option<u64>,
    removes: &'static [Row],
    removes_syncs: bool,
}

/// Runs `cell`, on an engine and in a home of its own.
fn run(cell: Cell, planted: Option<&str>, under_callgrind: bool) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let vfs = CountingVfs::register();
    let settings: Map<String, Value> = match cell.settings {
        Value::Object(settings) => settings,
        other => panic!("a cell's settings are an object, not {other}"),
    };
    let prepares = (!under_callgrind).then(|| prepares(cell.name, planted));
    let crawled = crawl_with(
        home.path(),
        std::time::UNIX_EPOCH,
        0,
        Options {
            metrics,
            settings,
            plant: planted.map(|name| plant(name, &work, vfs.name())),
            run_state_vfs: Some(vfs.name().to_owned()),
            largest_page: under_callgrind.then_some(CALLGRIND_PAGE),
        },
    );
    if !under_callgrind {
        assert_eq!(
            crawled.body_bytes,
            body_bytes(),
            "the site sends each body once"
        );
    }
    assert_eq!(
        crawled.records.len(),
        PAGES.len() + 1,
        "the crawl extracts each page once"
    );
    Counted {
        name: cell.name,
        pages: u64::try_from(crawled.records.len()).expect("invariant: a dozen pages"),
        attempts: crawled.attempts,
        work,
        syncs: vfs.syncs(),
        prepares: prepares.and_then(|prepares| prepares.join().ok()).flatten(),
        removes: cell.removes,
        removes_syncs: cell.removes_syncs,
    }
}

/// Starts `cell`'s run under Callgrind, when this run reads its row: a plant
/// of another row has no use for its figure and would wait for it.
fn prepares(cell: &str, planted: Option<&str>) -> std::thread::JoinHandle<Option<u64>> {
    let cell = cell.to_owned();
    let planted = planted.map(str::to_owned);
    std::thread::spawn(move || {
        if planted.as_deref().is_some_and(|row| row != PREPARES) {
            return None;
        }
        let executable = std::env::current_exe().expect("invariant: this process has a path");
        let mut arguments = vec!["--callgrind", cell.as_str()];
        if planted.is_some() {
            arguments.extend(["--plant", PREPARES]);
        }
        match callgrind::calls(PREPARE, &executable, &arguments) {
            Ok(calls) => Some(calls),
            Err(error) => {
                eprintln!("{PREPARES} is not read: {error}");
                None
            }
        }
    })
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (planted, callgrind_cell) = match arguments.as_slice() {
        [] => (None, None),
        [flag, row] if flag == "--plant" => (Some(row.as_str()), None),
        [flag, cell] if flag == "--callgrind" => (None, Some(cell.as_str())),
        [flag, cell, plant, row] if flag == "--callgrind" && plant == "--plant" => {
            (Some(row.as_str()), Some(cell.as_str()))
        }
        other => panic!("usage: counts [--callgrind <cell>] [--plant <row>], not {other:?}"),
    };
    if let Some(name) = callgrind_cell {
        let cell = cells()
            .into_iter()
            .find(|cell| cell.name == name)
            .expect("invariant: its parent names one of the cells");
        run(cell, planted, true);
        return;
    }
    // Side by side, each on its own paused runtime: no cell waits on another,
    // so a family's rows stay within their minute.
    let counted: Vec<Counted> = std::thread::scope(|scope| {
        let cells: Vec<_> = cells()
            .into_iter()
            .map(|cell| scope.spawn(move || run(cell, planted, false)))
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
        assert!(
            !cell.removes_syncs || cell.syncs < default.syncs,
            "the cell {} leaves {SYNCS} as it is",
            cell.name
        );
        let mut counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(cell.work.read(*row))))
            .collect();
        counts.insert(SYNCS.to_owned(), json!(cell.syncs));
        if let Some(prepares) = cell.prepares {
            counts.insert(PREPARES.to_owned(), json!(prepares));
        }
        let units = json!({
            "pages": cell.pages, "attempts": cell.attempts, "runs": RUNS, "effects": EFFECTS,
        });
        println!(
            "{}",
            json!({ "cell": cell.name, "units": units, "counts": counts })
        );
    }
}
