//! B2's counts cell (ADR 0140): runs `benches/session.rs`' Session once with
//! its default features on, once per feature on its paths that can be
//! switched off, once with no Live view viewer (C6 in ADR 0140: the default
//! cell is the one with a viewer, which #959 gives it), once with a Page
//! check and once on a browser pool's pages (C7: a pool its scripted launcher
//! starts, its pages opened and closed through `BrowserPages`), and prints what each counted as one JSON line, which `xtask counts`
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
//!
//! `--strace` runs the cells as `xtask counts --strace` watches them, under
//! `strace`; it runs nothing beside them either way. `--strace --bypass
//! <work>` has the default cell alone do one raw unit of that work per
//! command the browser reads beside the counted one: an `open` of a Run-trace
//! file or a `directory` of a Run trace created, which no counter sees and
//! `strace` does, so the cross-check proves it fails.
#![expect(
    clippy::disallowed_methods,
    reason = "benchmark harness: each cell on a thread of its own, none waiting on another (ADR 0071)"
)]
#![expect(
    clippy::print_stdout,
    reason = "the counts harness (ADR 0140): the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/session.rs"]
mod session;

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use serde_json::{Map, Value, json};

/// One cell: a name, the browsers its pages open in, the settings it runs
/// with, the rows it must count fewer of than the default cell, as a
/// feature's work goes with the feature (ADR 0140), and the rows it must count
/// none of.
struct Cell {
    name: &'static str,
    browsers: session::Browsers,
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

fn cells() -> [Cell; 7] {
    [
        // The cell with a viewer, once #959 gives B2's Session one.
        Cell {
            name: "default",
            browsers: session::Browsers::Borrowed,
            settings: json!({}),
            removes: &[],
            counts_none_of: &[],
        },
        Cell {
            name: "filter_lists_off",
            browsers: session::Browsers::Borrowed,
            settings: json!({ "filter_lists": {
                "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false,
            } }),
            removes: &[Row::CdpSends, Row::InterceptedMatches],
            counts_none_of: &[],
        },
        Cell {
            name: "no_viewer",
            browsers: session::Browsers::Borrowed,
            settings: json!({}),
            removes: &[],
            counts_none_of: LIVE_VIEW_FRAME,
        },
        // The Consent prompt runner still starts and looks with its handling
        // at `detect` (ADR 0048), so its reads and its entry stay.
        Cell {
            name: "consent_handling_detect",
            browsers: session::Browsers::Borrowed,
            settings: json!({ "consent": { "handling": "detect" } }),
            removes: &[],
            counts_none_of: &[],
        },
        // With its handling `off`, nothing of the runner is registered or
        // started and no cosmetic rule acts (#2434): its detection reads and
        // its Provenance entry go, and with them, its registration and live
        // hiding's domains, commands to the browser, the page's opening
        // ones included.
        Cell {
            name: "consent_handling_off",
            browsers: session::Browsers::Borrowed,
            settings: json!({ "consent": { "handling": "off" } }),
            removes: &[
                Row::NavigationDetectionReads,
                Row::NavigationProvenanceEntries,
                Row::CdpSends,
                Row::PageOpeningCommands,
            ],
            counts_none_of: &[],
        },
        // Navigations with a Page check: it compiles its glob and reads its
        // element, and the built-in detector's entry goes.
        Cell {
            name: "page_check",
            browsers: session::Browsers::Borrowed,
            settings: session::page_check(),
            removes: &[Row::NavigationProvenanceEntries],
            counts_none_of: &[],
        },
        // The same documents on pages of a browser pool (C7), as Foxhound's
        // crawl opens them for its handlers.
        Cell {
            name: "pool",
            browsers: session::Browsers::Pool,
            settings: json!({}),
            removes: &[],
            counts_none_of: &[],
        },
    ]
}

/// The plant of `bypass`: one raw unit of its work per command beside the
/// counted function, in a Run trace of its own under `home`, created first
/// through what counts.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140's strace cross-check: each bypass opens or creates beside the counted function on purpose, which no counter sees"
)]
fn bypass(bypass: &str, home: &Path, work: &CountedWork) -> session::Plant {
    let folder = home.join("bypass").join("trace");
    work.create_dir_all(Row::RunTraceRunFileOpens, &folder)
        .expect("invariant: the bypass's Run trace is created");
    match bypass {
        "open" => Arc::new(move || {
            drop(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(folder.join("bypass.jsonl"))
                    .expect("invariant: the bypass's file opens"),
            );
        }),
        "directory" => Arc::new(move || {
            std::fs::create_dir_all(&folder).expect("invariant: the bypass's folder is there");
        }),
        other => panic!("no bypass {other}: open or directory"),
    }
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    browsers: session::Browsers,
    session: session::Counted,
    removes: &'static [Row],
    counts_none_of: &'static [Row],
}

/// Runs `cell`, on an engine and in a home of its own, with the row `planted`
/// or the bypass `bypassed` planted.
fn run(cell: Cell, planted: Option<&str>, bypassed: Option<&str>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let plant = planted
        .map(|name| {
            let row = Row::ALL
                .into_iter()
                .find(|row| row.name() == name)
                .unwrap_or_else(|| panic!("no plant for the row {name}"));
            session::counting(metrics.counted_work(), row)
        })
        .or_else(|| bypassed.map(|name| bypass(name, home.path(), metrics.counted_work())));
    let settings: Map<String, Value> = match cell.settings {
        Value::Object(settings) => settings,
        other => panic!("a cell's settings are an object, not {other}"),
    };
    Counted {
        name: cell.name,
        browsers: cell.browsers,
        session: session::counted(cell.browsers, home.path(), metrics, settings, plant),
        removes: cell.removes,
        counts_none_of: cell.counts_none_of,
    }
}

/// The units the browser and the program counted of `session`, by name.
fn units(session: &session::Counted) -> Map<String, Value> {
    let mut units = Map::from_iter([
        ("commands".to_owned(), json!(session.commands)),
        ("reads".to_owned(), json!(session.reads)),
        ("documents".to_owned(), json!(session.documents)),
        ("serializations".to_owned(), json!(session.serializations)),
        ("field_reads".to_owned(), json!(session.field_reads)),
        ("pages_opened".to_owned(), json!(session.pages_opened)),
        ("sessions_opened".to_owned(), json!(session.sessions_opened)),
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
    units
}

/// Fails unless `cell` read fields, each read with a command at least, and
/// counted on `C2.read_commands` the commands the browser read meanwhile.
fn fields_read(cell: &Counted) {
    let session = &cell.session;
    let field_read_commands: u64 = session.field_read_commands.values().sum();
    assert!(
        session.field_reads > 0 && field_read_commands >= session.field_reads,
        "the cell {} read fields {} times with {} commands",
        cell.name,
        session.field_reads,
        field_read_commands
    );
    assert_eq!(
        session.field_rows, session.field_read_commands,
        "the cell {} counted other field-read commands than the browser read",
        cell.name
    );
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let (planted, bypassed) = match arguments.as_slice() {
        [] | ["--strace"] => (None, None),
        ["--plant", row] => (Some(*row), None),
        ["--strace", "--bypass", bypass] => (None, Some(*bypass)),
        other => {
            panic!("usage: counts [--plant <row>] | --strace [--bypass <work>], not {other:?}")
        }
    };
    let counted: Vec<Counted> = std::thread::scope(|scope| {
        let cells: Vec<_> = cells()
            .into_iter()
            // A bypass needs the default cell alone, the one that proves the
            // cross-check.
            .filter(|cell| bypassed.is_none() || cell.name == "default")
            .map(|cell| scope.spawn(move || run(cell, planted, bypassed)))
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
            session.serializations > 0 && session.serialization_commands >= session.serializations,
            "the cell {} serialized {} pages with {} commands",
            cell.name,
            session.serializations,
            session.serialization_commands
        );
        fields_read(cell);
        assert!(
            session.pages_opened > 0,
            "the cell {} opened no page",
            cell.name
        );
        // A launched browser's addresses are judged by its Session's egress
        // relay as it connects, which the scripted browser never reaches:
        // only a Borrowed browser's paused requests are refused here.
        let refuses = cell.browsers == session::Browsers::Borrowed;
        assert!(
            session.requests > 0 && (session.refused > 0) == refuses,
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
        let units = units(session);
        println!(
            "{}",
            json!({ "cell": cell.name, "units": units, "counts": counts })
        );
    }
}
