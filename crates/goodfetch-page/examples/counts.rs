//! B2's counts cell (ADR 0140): runs `benches/session.rs`' Session once with
//! its default features on and once per feature on its paths that can be
//! switched off, and prints what each counted as one JSON line, which `xtask
//! counts` divides and holds to the Quality register's limits. An example and
//! not a `[[bench]]`, as `xtask perf` runs every bench target under gungraun:
//! counts need no Valgrind and no optimized build.
//!
//! `--plant <row>` does one extra unit of that row's work per command the
//! browser reads. The run fails here if the work it counts did not happen, or
//! if a feature off did not take its work away.
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

/// One cell: a name, the settings it runs with, and the rows it must count
/// fewer of than the default cell: a feature's work goes with the feature
/// (ADR 0140).
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
            removes: &[Row::CdpSends],
        },
    ]
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    session: session::Counted,
    removes: &'static [Row],
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
        let session::Counted {
            commands,
            checks,
            attempts,
            work,
        } = &cell.session;
        assert!(*commands > 0, "the cell {} sent no command", cell.name);
        for row in cell.removes {
            assert!(
                work.read(*row) < default.work.read(*row),
                "the cell {} leaves {} as it is",
                cell.name,
                row.name()
            );
        }
        let counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(work.read(*row))))
            .collect();
        let mut units = Map::from_iter([
            ("commands".to_owned(), json!(commands)),
            ("checks".to_owned(), json!(checks)),
        ]);
        units.extend(
            attempts
                .iter()
                .map(|(kind, count)| (format!("{kind}_attempts"), json!(count))),
        );
        println!(
            "{}",
            json!({ "cell": cell.name, "units": units, "counts": counts })
        );
    }
}
