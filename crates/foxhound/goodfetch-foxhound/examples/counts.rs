//! B1's counts cell (ADR 0140), beside the in-memory crawl's heap cell: runs
//! `benches/scenarios.rs`' crawl once with its default features on and once
//! per feature on its paths that can be switched off, and prints what each
//! counted as one JSON line, which `xtask counts` divides and holds to the
//! Quality register's limits. An example and not a `[[bench]]`, as `xtask
//! perf` runs every bench target under gungraun: counts need no optimized
//! build.
//!
//! `F3.prepares` is what no wrapper sees (the spec's "What wrappers cannot
//! see"): each outcome's is Callgrind's call count of SQLite's prepare inside
//! the one function that does that outcome's admission. Each cell runs small
//! under Valgrind once per outcome for it, so this needs Valgrind; without it
//! those counts are not printed, and `xtask counts` says so.
//!
//! `--plant <row>` does one extra unit of that row's work per page served,
//! through its one counted function, in the default cell alone. The crawl
//! fails here if the work it counts did not happen, or if a feature off did
//! not take its work away.
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

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use scenarios::{Frontier, Options, PAGES, Plant, body_bytes, crawl_with};
use serde_json::{Map, Value, json};

/// Each outcome's `F3.prepares`, and the one function that does its admission.
const PREPARES: [(&str, &str); 3] = [
    ("F3.prepares", "goodfetch_crawl::enqueue::links::add"),
    ("F3.prepares.hop", "goodfetch_frontier::identity::admit_hop"),
    (
        "F3.prepares.sitemap",
        "goodfetch_crawl::work_items::read_sitemap",
    ),
];

/// SQLite's prepare, whose calls Callgrind counts.
const PREPARE: &str = "sqlite3_prepare_v3";

/// [`planted_prepare`], as Callgrind names it.
const PLANTED_PREPARE: &str = "counts::planted_prepare";

/// The one prepare a planted `F3.prepares` does, in a function of its own, as
/// Callgrind counts only what its named functions call.
#[inline(never)]
fn planted_prepare(connection: &rusqlite::Connection) {
    connection
        .prepare("SELECT 1")
        .expect("invariant: a literal statement");
}

/// The plant of the row named `name`.
fn plant(name: &str, work: &CountedWork) -> Plant {
    let work = work.clone();
    if name == "F1.parses" {
        return Arc::new(move |head: &str| {
            let _ = work.parse_html(Row::PageParses, head);
        });
    }
    if PREPARES.iter().any(|(row, _)| *row == name) {
        return Arc::new(move |_: &str| {
            let connection = rusqlite::Connection::open_in_memory()
                .expect("invariant: SQLite opens a database in memory");
            planted_prepare(&connection);
        });
    }
    let Some(row) = Row::ALL.into_iter().find(|row| row.name() == name) else {
        panic!("no plant for the row {name}");
    };
    Arc::new(move |_: &str| work.count(row))
}

/// One cell: a name, the settings it runs with beside the fixture's, whether
/// the fixture's Payload has its schema, and the rows it must count fewer of
/// than the default cell: a feature's work goes with the feature (ADR 0140).
struct Cell {
    name: &'static str,
    settings: Value,
    schema: bool,
    removes: &'static [Row],
}

/// A glob in the Scope, which is what a Frontier's reads of it compile.
fn scoped(mut settings: Value) -> Value {
    settings["crawl"] = json!({ "scope": { "exclude": ["http://pages.test/never/**"] } });
    settings
}

fn cells() -> [Cell; 4] {
    [
        Cell {
            name: "default",
            settings: scoped(json!({})),
            schema: true,
            removes: &[],
        },
        Cell {
            name: "filter_lists_off",
            settings: scoped(json!({ "filter_lists": {
                "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false,
            } })),
            schema: true,
            removes: &[Row::PageParses],
        },
        Cell {
            name: "scope_globs_off",
            settings: json!({}),
            schema: true,
            removes: &[Row::BatchCompilations, Row::HopCompilations],
        },
        Cell {
            name: "payload_schema_off",
            settings: scoped(json!({})),
            schema: false,
            removes: &[Row::PayloadCompilations, Row::PayloadValidations],
        },
    ]
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    pages: u64,
    hops: u64,
    sitemaps: u64,
    work: CountedWork,
    removes: &'static [Row],
    /// Each outcome's `F3.prepares`, when Valgrind ran.
    prepares: BTreeMap<&'static str, u64>,
}

/// The crawl of `cell`, on an engine and in a home of its own.
fn crawl(
    cell: &Cell,
    planted: Option<&str>,
    small_pages: bool,
) -> (scenarios::Crawled, CountedWork) {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let settings: Map<String, Value> = match &cell.settings {
        Value::Object(settings) => settings.clone(),
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
            frontier: Some(Frontier {
                schema: cell.schema,
            }),
            small_pages,
        },
    );
    (crawled, work)
}

/// Runs `cell`, and each outcome's prepares when `planted` asks for no other row.
fn run(cell: &Cell, planted: Option<&str>) -> Counted {
    let (crawled, work) = crawl(cell, planted, false);
    assert_eq!(
        crawled.body_bytes,
        body_bytes(),
        "the site sends each body once"
    );
    assert_eq!(
        crawled.records.len(),
        PAGES.len() + 4,
        "the crawl extracts each page once"
    );
    Counted {
        name: cell.name,
        pages: u64::try_from(crawled.records.len()).expect("invariant: a dozen pages"),
        hops: crawled.hops,
        sitemaps: crawled.sitemaps,
        work,
        removes: cell.removes,
        prepares: prepares(cell.name, planted),
    }
}

/// Each outcome's `F3.prepares` in the cell `cell`, side by side, the planted
/// one's alone when a row is planted, and none when another row is.
fn prepares(cell: &'static str, planted: Option<&str>) -> BTreeMap<&'static str, u64> {
    std::thread::scope(|scope| {
        let runs: Vec<_> = PREPARES
            .iter()
            .filter(|(row, _)| planted.is_none_or(|planted| planted == *row))
            .map(|(row, function)| {
                scope.spawn(move || {
                    under_callgrind(cell, function, planted).map(|count| (*row, count))
                })
            })
            .collect();
        runs.into_iter()
            .filter_map(|run| run.join().expect("invariant: a run under Callgrind ended"))
            .collect()
    })
}

/// The prepares inside `function` in the cell `cell`, with `planted`'s plant,
/// under Callgrind. `None` when Valgrind cannot run here.
///
/// Callgrind's call counts ignore its collection state, so `--toggle-collect`
/// scopes nothing: each call of `function` zeroes the counters on entry and
/// dumps them on exit, and each numbered dump holds that one call's prepares.
/// Its last, unnumbered dump holds what ran after and counts for nothing.
fn under_callgrind(cell: &str, function: &str, planted: Option<&str>) -> Option<u64> {
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let out = folder.path().join("out");
    let mut command = Command::new("valgrind");
    command
        .arg("--tool=callgrind")
        .arg(format!("--callgrind-out-file={}", out.display()))
        .args(["--compress-strings=no", "--compress-pos=no"])
        .arg("--log-file=/dev/null");
    let window = |command: &mut Command, function: &str| {
        command
            .arg(format!("--zero-before={function}"))
            .arg(format!("--dump-after={function}"));
    };
    window(&mut command, function);
    if planted.is_some() {
        window(&mut command, PLANTED_PREPARE);
    }
    command
        .arg(std::env::current_exe().expect("invariant: this executable has a path"))
        .args(["--small-cell", cell]);
    if let Some(planted) = planted {
        command.args(["--plant", planted]);
    }
    let Ok(status) = command.stdout(Stdio::null()).stderr(Stdio::null()).status() else {
        return None;
    };
    assert!(status.success(), "Callgrind's run of {function} failed");
    let dumps: Vec<String> = std::fs::read_dir(folder.path())
        .expect("invariant: Callgrind's folder is there")
        .map(|entry| entry.expect("invariant: the folder reads").path())
        .filter(|path| path.extension().is_some())
        .map(|path| std::fs::read_to_string(path).expect("invariant: Callgrind wrote its dump"))
        .collect();
    // A function renamed would otherwise count no prepare, and pass.
    assert!(
        !dumps.is_empty(),
        "Callgrind never entered {function}: the function that does this outcome's admission has moved"
    );
    Some(dumps.iter().map(|dump| calls_to_prepare(dump)).sum())
}

/// The calls of [`PREPARE`] the Callgrind dump `dump` records.
fn calls_to_prepare(dump: &str) -> u64 {
    let mut lines = dump.lines();
    let mut calls = 0;
    while let Some(line) = lines.next() {
        if line.strip_prefix("cfn=") == Some(PREPARE) {
            calls += lines
                .next()
                .and_then(|line| line.strip_prefix("calls="))
                .and_then(|line| line.split(' ').next())
                .and_then(|count| count.parse::<u64>().ok())
                .expect("invariant: `calls=` follows a `cfn=`");
        }
    }
    calls
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let (small, planted) = match arguments.as_slice() {
        [] => (None, None),
        ["--plant", row] => (None, Some(*row)),
        ["--small-cell", cell] => (Some(*cell), None),
        ["--small-cell", cell, "--plant", row] => (Some(*cell), Some(*row)),
        other => panic!("usage: counts [--small-cell <cell>] [--plant <row>], not {other:?}"),
    };
    if let Some(small) = small {
        let cell = cells()
            .into_iter()
            .find(|cell| cell.name == small)
            .expect("invariant: its parent named a cell");
        let _ = crawl(&cell, planted, true);
        return;
    }
    // Side by side, each on its own paused runtime: no cell waits on another,
    // so a family's rows stay within their minute. A plant needs the default
    // cell alone, the one that proves its counter.
    let counted: Vec<Counted> = std::thread::scope(|scope| {
        let cells: Vec<_> = cells()
            .into_iter()
            .filter(|cell| planted.is_none() || cell.name == "default")
            .map(|cell| scope.spawn(move || run(&cell, planted)))
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
        let mut counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(cell.work.read(*row))))
            .collect();
        for (row, count) in &cell.prepares {
            counts.insert((*row).to_owned(), json!(count));
        }
        let units = json!({
            "pages": cell.pages,
            "links_batches": cell.pages,
            "hops": cell.hops,
            "sitemaps": cell.sitemaps,
            "payloads": 1,
        });
        println!(
            "{}",
            json!({ "cell": cell.name, "units": units, "counts": counts })
        );
    }
}
