//! B3's counts cell (ADR 0140): runs each of the family's scenarios once with
//! its default features on and once per feature on its paths that can be
//! switched off, on an engine and in a home of its own, and prints what each
//! cell counted as one JSON line, which `xtask counts` divides and holds to
//! the Quality register's limits. An example and not a `[[bench]]`, as `xtask
//! perf` runs every bench target under gungraun: counts need no Valgrind and
//! no optimized build.
//!
//! `--plant <row>` does one extra unit of that row's work per call, through
//! its count. A cell fails here if the work it counts did not happen, or if a
//! feature off did not take its work away.
//!
//! A new scenario of the family (`benches/<scenario>.rs`) adds one line to
//! [`run`] that runs it and inserts its units, and its rows' arms to
//! [`plant`].
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stdout,
    reason = "the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/messages.rs"]
#[expect(
    dead_code,
    reason = "what only the Valgrind targets read of the scenario"
)]
mod messages;

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use messages::{Logs, Plant, Traffic};
use serde_json::{Map, Value, json};

/// The fixture's traffic: every outcome of the message path, the small
/// params' share large enough that a row's figure is theirs as well.
const TRAFFIC: Traffic = Traffic {
    small: 20,
    large: 10,
    plugin: 10,
};

/// The plant of the row named `name`.
fn plant(name: &str, work: &CountedWork) -> Plant {
    let work = work.clone();
    match name {
        "E1.deep_copies" => Arc::new(move |params: &Value| {
            work.count(Row::MessageDeepCopies);
            drop(params.clone());
        }),
        "E1.encodes" => Arc::new(move |params: &Value| {
            work.count(Row::MessageEncodes);
            drop(serde_json::to_vec(params));
        }),
        other => panic!("no plant for the row {other}"),
    }
}

/// One cell: a name, how it runs and the rows it must count fewer of than the
/// default cell: a feature's work goes with the feature (ADR 0140).
struct Cell {
    name: &'static str,
    logs: Logs,
    removes: &'static [Row],
}

const CELLS: [Cell; 2] = [
    Cell {
        name: "default",
        logs: Logs::On,
        removes: &[],
    },
    Cell {
        name: "log_off",
        logs: Logs::Off,
        removes: &[Row::MessageEncodes],
    },
];

/// What one cell counted.
struct Counted {
    cell: &'static Cell,
    units: BTreeMap<&'static str, u64>,
    work: CountedWork,
}

fn count(number: usize) -> u64 {
    u64::try_from(number).expect("invariant: a few dozen messages")
}

/// Runs `cell`'s scenarios.
fn run(cell: &'static Cell, planted: Option<&str>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let sent = messages::run(
        TRAFFIC,
        cell.logs,
        metrics,
        planted.map(|name| plant(name, &work)),
        home.path(),
    );
    assert!(sent.event_batches > 0, "the Plugin's events went out");
    assert_eq!(
        sent.handler_requests, TRAFFIC.plugin,
        "the Plugin's handler requests went out"
    );
    assert_eq!(
        sent.log_lines > 0,
        cell.logs == Logs::On,
        "the log lines follow the Connection's level"
    );
    let units = BTreeMap::from([
        ("incoming_messages", count(TRAFFIC.incoming())),
        ("outgoing_messages", count(sent.outgoing)),
    ]);
    Counted { cell, units, work }
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
        let cells: Vec<_> = CELLS
            .iter()
            .map(|cell| scope.spawn(move || run(cell, planted)))
            .collect();
        cells
            .into_iter()
            .map(|cell| cell.join().expect("invariant: a cell's checks passed"))
            .collect()
    });
    let default = &counted[0];
    for cell in &counted {
        for row in cell.cell.removes {
            assert!(
                cell.work.read(*row) < default.work.read(*row),
                "the cell {} leaves {} as it is",
                cell.cell.name,
                row.name()
            );
        }
        let counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(cell.work.read(*row))))
            .collect();
        println!(
            "{}",
            json!({ "cell": cell.cell.name, "units": cell.units, "counts": counts })
        );
    }
}
