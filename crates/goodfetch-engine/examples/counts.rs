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
//! Its Usage charge scenario (`support/charge_counts.rs`) charges under a
//! Limit in every cell but `no_limit`, whose run no Limit holds and which
//! takes the Slice's work away. What no wrapper sees of a charge comes from
//! outside the process: `E3.prepares` is Callgrind's call count of
//! `sqlite3_prepare_v3` inside the charges that took a Slice, run again under
//! Valgrind by this same executable (`--window`), so this needs Valgrind; the
//! covered charges' count must equal the statements the ledger counted, so a
//! statement added beside the counted read fails here.
//! `E3.slice_transactions` is the larger of the commit hook's transactions
//! and the crash VFS's syncs, which the row holds to one limit.
//!
//! A new scenario of the family (`benches/<scenario>.rs`) adds one line to
//! [`run`] that runs it and inserts its units and counts, and its rows' arms
//! to its plant.
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

#[path = "support/charge_counts.rs"]
mod charge_counts;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use charge_counts::{Outcome, charges};
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

/// One cell: a name, how it runs, whether a Limit holds its charges, and the
/// rows it must count fewer of than the default cell: a feature's work goes
/// with the feature (ADR 0140).
struct Cell {
    name: &'static str,
    logs: Logs,
    limited: bool,
    removes: &'static [Row],
}

const CELLS: [Cell; 3] = [
    Cell {
        name: "default",
        logs: Logs::On,
        limited: true,
        removes: &[],
    },
    Cell {
        name: "log_off",
        logs: Logs::Off,
        limited: true,
        removes: &[Row::MessageEncodes],
    },
    // With no Limit, the Slice's work goes with the Limit.
    Cell {
        name: "no_limit",
        logs: Logs::On,
        limited: false,
        removes: &[Row::ChargeOpens, Row::ChargeTransactions],
    },
];

/// What one cell counted: its units, the message scenario's work, and its
/// charges' counts by row, which the charge scenario counts on its own.
struct Counted {
    cell: &'static Cell,
    units: BTreeMap<&'static str, u64>,
    work: CountedWork,
    charges: BTreeMap<&'static str, u64>,
}

impl Counted {
    /// What this cell counted on `row`.
    fn count(&self, row: Row) -> u64 {
        self.charges
            .get(row.name())
            .copied()
            .unwrap_or_else(|| self.work.read(row))
    }
}

fn count(number: usize) -> u64 {
    u64::try_from(number).expect("invariant: a few dozen messages")
}

/// Runs `cell`'s scenarios.
fn run(cell: &'static Cell, planted: Option<&str>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let (message_plant, charge_plant) = match planted {
        Some(name) if name.starts_with("E3.") => (None, Some(name)),
        other => (other, None),
    };
    let sent = messages::run(
        TRAFFIC,
        cell.logs,
        metrics,
        message_plant.map(|name| plant(name, &work)),
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
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let charged = charges(folder.path(), cell.limited, charge_plant, None);
    // With no Limit, the first charge is a covered one and opens the ledger:
    // its set-up is no statement a charge reads with, so only a cell whose
    // first charge takes a Slice compares the two.
    let slice_prepares = if cell.limited {
        if charge_plant.is_none() {
            assert_eq!(
                callgrind(cell.name, "covered", None),
                charged.statements,
                "the cell {}'s covered charges prepared what the ledger did not count",
                cell.name
            );
        }
        callgrind(cell.name, "slice", charge_plant)
    } else {
        0
    };
    let units = BTreeMap::from([
        ("incoming_messages", count(TRAFFIC.incoming())),
        ("outgoing_messages", count(sent.outgoing)),
        ("covered_charges", charged.covered),
        ("slices", charged.slices),
        ("charges", charged.covered + charged.slices),
    ]);
    let charges = BTreeMap::from([
        (Row::ChargeStatements.name(), charged.statements),
        (
            Row::ChargeTransactions.name(),
            charged.transactions.max(charged.syncs),
        ),
        (Row::ChargeOpens.name(), charged.opens),
        ("E3.prepares", slice_prepares),
    ]);
    Counted {
        cell,
        units,
        work,
        charges,
    }
}

/// Runs one cell's charges under this executable's `--window`: Callgrind's own
/// process.
fn window(cell: &str, outcome: &str, planted: Option<&str>) {
    let limited = CELLS
        .iter()
        .find(|each| each.name == cell)
        .unwrap_or_else(|| panic!("no cell {cell}"))
        .limited;
    let outcome = match outcome {
        "covered" => Outcome::Covered,
        "slice" => Outcome::SliceTaken,
        other => panic!("no outcome {other}"),
    };
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    charges(folder.path(), limited, planted, Some(outcome));
}

/// The calls of `sqlite3_prepare_v3` in the windows of a Callgrind profile
/// written in `folder` with `--compress-strings=no`, which writes each called
/// function by its name: one part per dump, a window's dumped as it ends.
fn prepares(folder: &Path) -> u64 {
    std::fs::read_dir(folder)
        .expect("invariant: Callgrind wrote its profile")
        .map(|part| std::fs::read_to_string(part.expect("invariant: a listed part").path()))
        .map(|part| part.expect("invariant: Callgrind's part is text"))
        .filter(|part| {
            part.lines()
                .find(|line| line.starts_with("desc: Trigger:"))
                .is_some_and(|trigger| trigger.contains("window_ends"))
        })
        .map(|part| calls_in(&part))
        .sum()
}

/// The calls of `sqlite3_prepare_v3` in one part.
fn calls_in(text: &str) -> u64 {
    let mut lines = text.lines();
    let mut calls = 0;
    while let Some(line) = lines.next() {
        if line == "cfn=sqlite3_prepare_v3" {
            calls += lines
                .next()
                .and_then(|line| line.strip_prefix("calls="))
                .and_then(|line| line.split(' ').next())
                .and_then(|count| count.parse::<u64>().ok())
                .expect("invariant: a call's line follows its callee's");
        }
    }
    calls
}

/// What Callgrind counts of `outcome`'s charges in `cell`.
fn callgrind(cell: &str, outcome: &str, planted: Option<&str>) -> u64 {
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let profile = folder.path().join("callgrind.out");
    let mut command = Command::new("valgrind");
    command
        .args([
            "--tool=callgrind",
            "--compress-strings=no",
            "--dump-before=*window_starts*",
            "--dump-before=*window_ends*",
        ])
        .arg(format!("--callgrind-out-file={}", profile.display()))
        .arg(std::env::current_exe().expect("invariant: this executable has a path"))
        .args(["--window", cell, outcome]);
    if let Some(planted) = planted {
        command.arg(planted);
    }
    let output = command
        .output()
        .expect("invariant: Valgrind runs, as ADR 0071's benchmarks need it");
    assert!(
        output.status.success(),
        "Callgrind failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    prepares(folder.path())
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let planted = match arguments.as_slice() {
        [] => None,
        ["--plant", row] => Some(*row),
        ["--window", cell, outcome] => return window(cell, outcome, None),
        ["--window", cell, outcome, row] => return window(cell, outcome, Some(row)),
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
                cell.count(*row) < default.count(*row),
                "the cell {} leaves {} as it is",
                cell.cell.name,
                row.name()
            );
        }
        let mut counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(cell.work.read(*row))))
            .collect();
        for (row, count) in &cell.charges {
            counts.insert((*row).to_owned(), json!(count));
        }
        println!(
            "{}",
            json!({ "cell": cell.cell.name, "units": cell.units, "counts": counts })
        );
    }
}
