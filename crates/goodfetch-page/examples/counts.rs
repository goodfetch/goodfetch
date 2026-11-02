//! B2's counts cell (ADR 0140): runs `benches/session.rs`' Session once with
//! its default features on, once per feature on its paths that can be
//! switched off, once with no Live view viewer (C6 in ADR 0140: the default
//! cell is the one with a viewer, which #959 gives it) and once with a Page
//! check, and prints what each counted as one JSON line, which `xtask counts`
//! divides and holds to the Quality register's limits. An example and not a
//! `[[bench]]`, as `xtask perf` runs every bench target under gungraun: counts
//! need no Valgrind and no optimized build.
//!
//! Its units are the commands the browser read, the engine reads in a frame
//! the program made, and the documents and frames those reads ran in. C2's
//! rows count over those reads alone (`session::READ_ROWS`).
//!
//! `--plant <row>` does one extra unit of that row's work per command the
//! browser reads, which is at least one per unit of every row: each request
//! paused, each pause and each refusal is answered by a command. The run
//! fails here if the work it counts did not happen, if a feature off did not
//! take its work away, or if the cell with no viewer did any of a Live view
//! frame's work.
#![expect(
    clippy::disallowed_methods,
    reason = "benchmark harness: each cell on a thread of its own, none waiting on another (ADR 0071)"
)]
#![expect(
    clippy::print_stdout,
    reason = "the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/session.rs"]
mod session;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use serde_json::{Map, Value, json};

/// One cell: a name, the settings it runs with, the rows it must count fewer
/// of than the default cell, as a feature's work goes with the feature (ADR
/// 0140), and the rows it must count none of.
struct Cell {
    name: &'static str,
    settings: Value,
    removes: &'static [Row],
    counts_none_of: &'static [Row],
}

/// A Live view frame's work: a Session with no viewer does none of it (ADR
/// 0140's C6).
const LIVE_VIEW_FRAME: &[Row] = &[
    Row::LiveViewFileOpens,
    Row::LiveViewCompilations,
    Row::LiveViewSpawns,
    Row::LiveViewAcknowledgements,
    Row::LiveViewSeals,
];

fn cells() -> [Cell; 5] {
    [
        // The cell with a viewer, once #959 gives B2's Session one.
        Cell {
            name: "default",
            settings: json!({}),
            removes: &[],
            counts_none_of: &[],
        },
        Cell {
            name: "filter_lists_off",
            settings: json!({ "filter_lists": {
                "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false,
            } }),
            removes: &[Row::CdpSends, Row::InterceptedMatches],
            counts_none_of: &[],
        },
        Cell {
            name: "no_viewer",
            settings: json!({}),
            removes: &[],
            counts_none_of: LIVE_VIEW_FRAME,
        },
        // The Consent prompt runner still starts and looks with its handling
        // off (ADR 0048), so its reads and its entry stay.
        Cell {
            name: "consent_handling_off",
            settings: json!({ "consent": { "handling": false } }),
            removes: &[],
            counts_none_of: &[],
        },
        // Navigations with a Page check: it compiles its glob and reads its
        // element, and the built-in detector's entry goes.
        Cell {
            name: "page_check",
            settings: session::page_check(),
            removes: &[Row::NavigationProvenanceEntries],
            counts_none_of: &[],
        },
    ]
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    session: session::Counted,
    removes: &'static [Row],
    counts_none_of: &'static [Row],
}

/// Runs `cell`, on an engine and in a home of its own.
fn run(cell: Cell, planted: Option<&str>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let plant = planted.map(|name| {
        let row = Row::ALL
            .into_iter()
            .find(|row| row.name() == name)
            .unwrap_or_else(|| panic!("no plant for the row {name}"));
        session::counting(metrics.counted_work(), row)
    });
    let settings: Map<String, Value> = match cell.settings {
        Value::Object(settings) => settings,
        other => panic!("a cell's settings are an object, not {other}"),
    };
    Counted {
        name: cell.name,
        session: session::counted(home.path(), metrics, settings, plant),
        removes: cell.removes,
        counts_none_of: cell.counts_none_of,
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let planted = match arguments.as_slice() {
        [] => None,
        [flag, row] if flag == "--plant" => Some(row.as_str()),
        other => panic!("usage: counts [--plant <row>], not {other:?}"),
    };
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
    let default = &counted[0].session;
    for cell in &counted {
        let session = &cell.session;
        assert!(
            session.commands > 0,
            "the cell {} sent no command",
            cell.name
        );
        assert!(
            session.reads > 0 && session.documents > 0,
            "the cell {} read no document",
            cell.name
        );
        assert!(
            session.read_commands >= session.reads,
            "the cell {} read without a command",
            cell.name
        );
        assert!(
            session.pages_opened > 0,
            "the cell {} opened no page",
            cell.name
        );
        assert!(
            session.requests > 0 && session.refused > 0,
            "the cell {} paused {} requests and refused {}",
            cell.name,
            session.requests,
            session.refused
        );
        assert!(
            session.navigations > 0,
            "the cell {} navigated nowhere",
            cell.name
        );
        for row in cell.removes {
            assert!(
                session.count(*row) < default.count(*row),
                "the cell {} leaves {} as it is",
                cell.name,
                row.name()
            );
        }
        for row in cell.counts_none_of {
            assert_eq!(
                session.work.read(*row),
                0,
                "the cell {} counts {}",
                cell.name,
                row.name()
            );
        }
        let counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(session.count(*row))))
            .collect();
        let mut units = Map::from_iter([
            ("commands".to_owned(), json!(session.commands)),
            ("reads".to_owned(), json!(session.reads)),
            ("documents".to_owned(), json!(session.documents)),
            ("pages_opened".to_owned(), json!(session.pages_opened)),
            ("requests".to_owned(), json!(session.requests)),
            ("pauses".to_owned(), json!(session.paused)),
            ("refusals".to_owned(), json!(session.refused)),
            ("checks".to_owned(), json!(session.checks)),
            ("navigations".to_owned(), json!(session.navigations)),
        ]);
        units.extend(
            session
                .attempts
                .iter()
                .map(|(kind, count)| (format!("{kind}_attempts"), json!(count))),
        );
        println!(
            "{}",
            json!({ "cell": cell.name, "units": units, "counts": counts })
        );
    }
}
